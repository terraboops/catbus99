//! The control protocol.
//!
//! Requests and responses cross a process boundary as newline-delimited JSON, so the
//! encoding is a compatibility surface: the CLI, the MCP server, and anything a user
//! writes against the socket all depend on it. These tests pin the wire shape, not just
//! round-tripping.

use catbus99_daemon::protocol::*;
use catbus99_model::{Align, Binding, Color, Layout, Rect, TextSize, Widget};

fn roundtrip(req: &Request) -> Request {
    let text = serde_json::to_string(req).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn requests_are_tagged_by_an_op_field() {
    // The tag is what makes the protocol readable with `nc` while debugging.
    let text = serde_json::to_string(&Request::Status).unwrap();
    assert_eq!(text, r#"{"op":"status"}"#);
    assert_eq!(
        serde_json::to_string(&Request::Wear).unwrap(),
        r#"{"op":"wear"}"#
    );
}

#[test]
fn a_minimal_request_needs_only_its_op() {
    // Optional fields must default, so a hand-written client stays simple.
    let r: Request = serde_json::from_str(r#"{"op":"render"}"#).unwrap();
    match r {
        Request::Render { execute, origin } => {
            assert!(
                !execute,
                "execute must default to false: writing by default would spend flash"
            );
            assert!(origin.is_none());
        }
        other => panic!("got {other:?}"),
    }
}

/// The safe default matters more than convenience here: a client that forgets `execute`
/// must not cause a flash write.
#[test]
fn write_flags_default_to_not_writing() {
    match serde_json::from_str::<Request>(r#"{"op":"show_image","path":"/x.png"}"#).unwrap() {
        Request::ShowImage {
            execute,
            max_frames,
            ..
        } => {
            assert!(!execute);
            assert!(max_frames.is_none());
        }
        other => panic!("got {other:?}"),
    }
    match serde_json::from_str::<Request>(r#"{"op":"set_widget","slot":"a","widget":null}"#)
        .unwrap()
    {
        Request::SetWidget { render, .. } => assert!(!render),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn every_request_variant_round_trips() {
    let layout = Layout::new("t").with_slot(
        "s",
        Rect::new(0, 0, 10, 10),
        Widget::Label {
            text: Binding::literal_text("x"),
            size: TextSize::Small,
            align: Align::Left,
            color: Color::WHITE,
        },
    );
    let variants = vec![
        Request::Status,
        Request::Wear,
        Request::GetLayout,
        Request::GetDataPoints,
        Request::Preview,
        Request::SyncClock,
        Request::ListSources,
        Request::Shutdown,
        Request::RunSource { id: "demo".into() },
        Request::Render {
            execute: true,
            origin: Some(Origin::Scheduled),
        },
        Request::ShowImage {
            path: "/tmp/a.gif".into(),
            execute: true,
            max_frames: Some(24),
            origin: Some(Origin::Interactive),
        },
        Request::SetLayout {
            layout: Box::new(layout),
            render: true,
            origin: None,
        },
        Request::SetWidget {
            slot: "s".into(),
            widget: Some(Box::new(Widget::Blank)),
            render: false,
            origin: None,
        },
    ];
    for v in &variants {
        let back = roundtrip(v);
        assert_eq!(
            serde_json::to_string(v).unwrap(),
            serde_json::to_string(&back).unwrap(),
            "variant did not round-trip: {v:?}"
        );
    }
}

#[test]
fn origin_serialises_in_snake_case() {
    assert_eq!(
        serde_json::to_string(&Origin::Interactive).unwrap(),
        r#""interactive""#
    );
    assert_eq!(
        serde_json::to_string(&Origin::Scheduled).unwrap(),
        r#""scheduled""#
    );
}

#[test]
fn responses_are_tagged_by_a_result_field() {
    assert_eq!(
        serde_json::to_string(&Response::Ok).unwrap(),
        r#"{"result":"ok"}"#
    );
    let e = Response::Error {
        message: "nope".into(),
    };
    assert_eq!(
        serde_json::to_string(&e).unwrap(),
        r#"{"result":"error","message":"nope"}"#
    );
}

/// `uploaded` is the field every client keys on, so its meaning must be stable: bytes
/// actually reached the panel.
#[test]
fn a_write_outcome_carries_the_reason_and_retry_hint() {
    let w = WriteOutcome {
        uploaded: false,
        reason: "rate_limited".into(),
        detail: "next write in 7m".into(),
        retry_after_secs: Some(432),
        bytes: Some(32768),
        reports: Some(8),
        uploads_used: 20,
        uploads_remaining: 99_980,
    };
    let text = serde_json::to_string(&Response::Write(w.clone())).unwrap();
    assert!(text.contains(r#""result":"write""#));
    assert!(text.contains(r#""uploaded":false"#));
    assert!(text.contains(r#""retry_after_secs":432"#));

    match serde_json::from_str::<Response>(&text).unwrap() {
        Response::Write(back) => assert_eq!(back, w),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn absent_optional_outcome_fields_are_omitted_not_null() {
    let w = WriteOutcome {
        uploaded: true,
        reason: "upload".into(),
        detail: "upload".into(),
        retry_after_secs: None,
        bytes: None,
        reports: None,
        uploads_used: 1,
        uploads_remaining: 2,
    };
    let text = serde_json::to_string(&w).unwrap();
    assert!(!text.contains("retry_after_secs"), "{text}");
    assert!(!text.contains("null"), "{text}");
}

#[test]
fn an_unknown_op_is_rejected_rather_than_silently_ignored() {
    assert!(serde_json::from_str::<Request>(r#"{"op":"launch_missiles"}"#).is_err());
    assert!(serde_json::from_str::<Request>(r#"{}"#).is_err());
}

#[test]
fn the_socket_path_is_under_the_xdg_runtime_or_state_dir() {
    let p = default_socket_path();
    assert!(p.ends_with("catbus99/ctl.sock"), "{}", p.display());
}
