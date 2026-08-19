//! End-to-end exercise of the control socket.
//!
//! Starts a real server on a temporary socket and drives it over a real Unix stream, so
//! the framing, dispatch, concurrency and shutdown paths are covered rather than assumed.
//! Nothing here needs a keyboard: read-only requests work without one, and write requests
//! are checked for a *well-formed* outcome rather than a successful upload.

use catbus99_daemon::protocol::{Origin, Request, Response};
use catbus99_daemon::server::{request, Server};
use catbus99_daemon::{Daemon, Paths};
use catbus99_model::{Align, Binding, Color, Layout, Rect, TextSize, Value, Widget};
use std::path::PathBuf;
use std::time::Duration;

struct TestDaemon {
    dir: PathBuf,
    socket: PathBuf,
}

impl TestDaemon {
    async fn start(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("catbus99-it-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("ctl.sock");

        let layout = Layout::new("test")
            .with_slot(
                "title",
                Rect::new(0, 0, 160, 10),
                Widget::Label {
                    text: Binding::literal_text("HELLO"),
                    size: TextSize::Small,
                    align: Align::Left,
                    color: Color::WHITE,
                },
            )
            .with_slot("body", Rect::new(0, 12, 160, 40), Widget::Blank);

        let paths = Paths {
            socket: socket.clone(),
            wear_state: dir.join("wear.json"),
            sources: dir.join("sources.toml"),
            layout: dir.join("layout.json"),
        };
        let daemon = Daemon::new(layout, paths).unwrap();
        let server = Server::new(daemon).with_socket(socket.clone());
        tokio::spawn(server.run());

        // Wait for the socket to appear rather than sleeping a fixed interval.
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(socket.exists(), "server did not bind its socket");
        Self { dir, socket }
    }

    async fn send(&self, req: Request) -> Response {
        request(&self.socket, &req).await.expect("request failed")
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn status_reports_the_loaded_layout() {
    let d = TestDaemon::start("status").await;
    match d.send(Request::Status).await {
        Response::Status(s) => {
            assert_eq!(s.layout_id, "test");
            assert_eq!(s.slots, 2);
            assert_eq!(s.data_points, 0);
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn data_points_can_be_pushed_and_read_back() {
    let d = TestDaemon::start("points").await;
    let point = catbus99_model::DataPoint {
        source: "t".into(),
        key: "k".into(),
        value: Value::Number(0.5),
        unit: Some("%".into()),
        label: None,
        observed_at: chrono::Utc::now(),
        ttl_secs: Some(60),
    };
    assert!(matches!(
        d.send(Request::PushDataPoint {
            point: Box::new(point)
        })
        .await,
        Response::Ok
    ));

    match d.send(Request::GetDataPoints).await {
        Response::DataPoints { points } => {
            assert_eq!(points.len(), 1);
            assert_eq!(points[0].key, "k");
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn a_widget_can_be_set_and_shows_up_in_the_layout() {
    let d = TestDaemon::start("setwidget").await;
    let widget = Widget::Label {
        text: Binding::literal_text("CHANGED"),
        size: TextSize::Medium,
        align: Align::Center,
        color: Color::new(1, 2, 3),
    };
    assert!(matches!(
        d.send(Request::SetWidget {
            slot: "body".into(),
            widget: Some(Box::new(widget.clone())),
            render: false,
            origin: None,
        })
        .await,
        Response::Ok
    ));

    match d.send(Request::GetLayout).await {
        Response::Layout(l) => {
            let slot = l.slots.iter().find(|s| s.id == "body").unwrap();
            assert_eq!(slot.widget.as_ref(), Some(&widget));
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn setting_an_unknown_slot_is_an_error_that_names_the_real_slots() {
    let d = TestDaemon::start("badslot").await;
    match d
        .send(Request::SetWidget {
            slot: "nonexistent".into(),
            widget: None,
            render: false,
            origin: None,
        })
        .await
    {
        Response::Error { message } => {
            assert!(message.contains("nonexistent"));
            assert!(
                message.contains("title"),
                "should list real slots: {message}"
            );
        }
        other => panic!("got {other:?}"),
    }
}

/// Preview must never touch the device, with or without hardware attached.
#[tokio::test]
async fn preview_returns_a_png_and_never_writes() {
    let d = TestDaemon::start("preview").await;
    let before = uploads(&d).await;
    match d.send(Request::Preview).await {
        Response::Preview {
            png_base64,
            width,
            height,
        } => {
            assert_eq!((width, height), (160, 96));
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&png_base64)
                .unwrap();
            assert_eq!(&bytes[1..4], b"PNG", "not a PNG");
        }
        other => panic!("got {other:?}"),
    }
    assert_eq!(
        uploads(&d).await,
        before,
        "preview must not spend an upload"
    );
}

#[tokio::test]
async fn a_dry_run_render_never_reports_an_upload() {
    let d = TestDaemon::start("dryrun").await;
    let before = uploads(&d).await;
    match d
        .send(Request::Render {
            execute: false,
            origin: Some(Origin::Interactive),
        })
        .await
    {
        Response::Write(w) => {
            assert!(!w.uploaded);
            assert_eq!(w.reason, "would_upload");
            assert!(w.bytes.unwrap_or(0) > 0);
        }
        other => panic!("got {other:?}"),
    }
    assert_eq!(
        uploads(&d).await,
        before,
        "a dry run must not spend an upload"
    );
}

async fn uploads(d: &TestDaemon) -> u64 {
    match d.send(Request::Wear).await {
        Response::Wear(w) => w.total_uploads,
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn running_an_unknown_source_is_an_error() {
    let d = TestDaemon::start("nosource").await;
    assert!(matches!(
        d.send(Request::RunSource { id: "ghost".into() }).await,
        Response::Error { .. }
    ));
}

#[tokio::test]
async fn sources_list_is_empty_without_a_registry() {
    let d = TestDaemon::start("emptysources").await;
    match d.send(Request::ListSources).await {
        Response::Sources { sources } => assert!(sources.is_empty()),
        other => panic!("got {other:?}"),
    }
}

/// Malformed input must produce an error response rather than dropping the connection,
/// so a buggy client gets a diagnosis instead of a silent hang.
#[tokio::test]
async fn malformed_json_gets_an_error_response() {
    let d = TestDaemon::start("malformed").await;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let stream = tokio::net::UnixStream::connect(&d.socket).await.unwrap();
    let (r, mut w) = stream.into_split();
    w.write_all(b"{not json}\n").await.unwrap();
    w.flush().await.unwrap();

    let mut lines = BufReader::new(r).lines();
    let line = lines.next_line().await.unwrap().expect("no reply");
    let resp: Response = serde_json::from_str(&line).unwrap();
    match resp {
        Response::Error { message } => assert!(message.contains("malformed")),
        other => panic!("got {other:?}"),
    }
}

/// One connection may carry many requests; the server must not close after the first.
#[tokio::test]
async fn a_connection_handles_several_requests() {
    let d = TestDaemon::start("pipeline").await;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let stream = tokio::net::UnixStream::connect(&d.socket).await.unwrap();
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();

    for _ in 0..3 {
        w.write_all(b"{\"op\":\"status\"}\n").await.unwrap();
        w.flush().await.unwrap();
        let line = lines.next_line().await.unwrap().expect("no reply");
        assert!(line.contains("\"result\":\"status\""));
    }
}

/// Several clients at once is the normal case: CLI, MCP server, and the scheduler.
#[tokio::test]
async fn concurrent_clients_are_served() {
    let d = TestDaemon::start("concurrent").await;
    let socket = d.socket.clone();
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let s = socket.clone();
            tokio::spawn(async move { request(&s, &Request::Status).await.is_ok() })
        })
        .collect();
    for t in tasks {
        assert!(t.await.unwrap(), "a concurrent client failed");
    }
}

#[tokio::test]
async fn reaching_a_daemon_that_is_not_running_says_so() {
    let missing = std::env::temp_dir().join("catbus99-definitely-absent.sock");
    let _ = std::fs::remove_file(&missing);
    let err = request(&missing, &Request::Status).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("catbus99 daemon"), "unhelpful error: {msg}");
    assert!(
        msg.contains("catbus99 daemon`"),
        "should suggest starting it: {msg}"
    );
}

/// Two daemons must not both own the display: each keeps its own wear state, so neither
/// odometer would be correct, and the second would silently steal the socket leaving the
/// first running but unreachable.
#[tokio::test]
async fn a_second_daemon_refuses_to_steal_a_live_socket() {
    let d = TestDaemon::start("singleton").await;

    let paths = Paths {
        socket: d.socket.clone(),
        wear_state: d.dir.join("wear2.json"),
        sources: d.dir.join("sources2.toml"),
        layout: d.dir.join("layout2.json"),
    };
    let second = Daemon::new(Layout::new("second"), paths).unwrap();
    let err = Server::new(second)
        .with_socket(d.socket.clone())
        .run()
        .await
        .expect_err("a second daemon should refuse to start");

    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    assert!(err.to_string().contains("already running"), "{err}");

    // The original daemon must still be serving.
    match d.send(Request::Status).await {
        Response::Status(s) => assert_eq!(s.layout_id, "test"),
        other => panic!("original daemon stopped answering: {other:?}"),
    }
}

/// A socket file left by a crash must not block start-up forever.
#[tokio::test]
async fn a_stale_socket_file_is_reclaimed() {
    let dir = std::env::temp_dir().join(format!("catbus99-stale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("ctl.sock");
    // A regular file at the socket path: exists, but nothing is listening.
    std::fs::write(&socket, b"stale").unwrap();

    let paths = Paths {
        socket: socket.clone(),
        wear_state: dir.join("wear.json"),
        sources: dir.join("sources.toml"),
        layout: dir.join("layout.json"),
    };
    let daemon = Daemon::new(Layout::new("stale"), paths).unwrap();
    tokio::spawn(Server::new(daemon).with_socket(socket.clone()).run());

    for _ in 0..100 {
        if request(&socket, &Request::Status).await.is_ok() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("daemon did not reclaim a stale socket file");
}
