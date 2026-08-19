//! The Unix-socket control server and the data-source scheduler.
//!
//! One process owns the display; everything else is a client. Requests are serialised
//! through a mutex, so the governor sees a single ordered stream of writes and the
//! "what is currently on screen" hash is always accurate.

use crate::protocol::{default_socket_path, Origin, Request, Response};
use crate::sources::{run_source, SourceSpec};
use crate::Daemon;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, Notify};

/// How often the scheduler wakes to look for due sources.
const TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// How often to check whether the panel has reappeared.
///
/// Polling this is cheap (an enumeration, no device open) and it is the only way to notice
/// a power cycle, after which the keyboard shows its native screen again.
const RECONNECT_CHECK_SECS: i64 = 20;

pub struct Server {
    daemon: Arc<Mutex<Daemon>>,
    socket: PathBuf,
    shutdown: Arc<Notify>,
}

impl Server {
    pub fn new(daemon: Daemon) -> Self {
        let socket = daemon.paths.socket.clone();
        Self {
            daemon: Arc::new(Mutex::new(daemon)),
            socket,
            shutdown: Arc::new(Notify::new()),
        }
    }

    pub fn with_socket(mut self, path: PathBuf) -> Self {
        self.socket = path;
        self
    }

    pub async fn run(self) -> std::io::Result<()> {
        if let Some(dir) = self.socket.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        // A socket file may be stale (left by a crash) or live (another daemon). Removing
        // it unconditionally would let a second daemon steal the path, leaving the first
        // running but unreachable -- and two daemons would each keep their own copy of the
        // wear state, so neither odometer would be right. Probe before removing.
        if self.socket.exists() {
            if UnixStream::connect(&self.socket).await.is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!(
                        "another catbus99 daemon is already running on {}. \
                         Only one may own the display: stop it first, or pass a different --socket.",
                        self.socket.display()
                    ),
                ));
            }
            tracing::warn!(socket = %self.socket.display(), "removing a stale socket");
            let _ = tokio::fs::remove_file(&self.socket).await;
        }

        let listener = UnixListener::bind(&self.socket)?;
        tracing::info!(socket = %self.socket.display(), "catbus99 daemon listening");

        let scheduler = tokio::spawn(scheduler_loop(self.daemon.clone(), self.shutdown.clone()));

        loop {
            tokio::select! {
                _ = self.shutdown.notified() => break,
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            let daemon = self.daemon.clone();
                            let shutdown = self.shutdown.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, daemon, shutdown).await {
                                    tracing::warn!(error = %e, "client connection failed");
                                }
                            });
                        }
                        Err(e) => tracing::warn!(error = %e, "accept failed"),
                    }
                }
            }
        }

        scheduler.abort();
        let _ = tokio::fs::remove_file(&self.socket).await;
        tracing::info!("catbus99 daemon stopped");
        Ok(())
    }
}

async fn handle_connection(
    stream: UnixStream,
    daemon: Arc<Mutex<Daemon>>,
    shutdown: Arc<Notify>,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Err(e) => Response::Error {
                message: format!("malformed request: {e}"),
            },
            Ok(request) => dispatch(request, &daemon, &shutdown).await,
        };
        let mut text = serde_json::to_string(&response).unwrap_or_else(|e| {
            format!(r#"{{"result":"error","message":"failed to encode response: {e}"}}"#)
        });
        text.push('\n');
        write_half.write_all(text.as_bytes()).await?;
        write_half.flush().await?;
    }
    Ok(())
}

/// Requests that need to await a subprocess are handled here; the rest go straight to
/// [`Daemon::handle`], which is synchronous and holds the lock only briefly.
async fn dispatch(
    request: Request,
    daemon: &Arc<Mutex<Daemon>>,
    shutdown: &Arc<Notify>,
) -> Response {
    let now = Utc::now();

    if let Request::RunSource { id } = &request {
        // Resolve the spec, then release the lock for the duration of the subprocess so a
        // slow or hung source cannot block every other client.
        let spec = {
            let d = daemon.lock().await;
            d.registry.get(id).cloned()
        };
        let Some(spec) = spec else {
            return Response::Error {
                message: format!("no source {id:?}"),
            };
        };
        return match run_source(&spec, now).await {
            Ok(points) => {
                let mut d = daemon.lock().await;
                d.absorb(&spec.id, points, now);
                Response::Ok
            }
            Err(e) => {
                let mut d = daemon.lock().await;
                d.record_source_failure(&spec.id, e.to_string(), now);
                Response::Error {
                    message: e.to_string(),
                }
            }
        };
    }

    let is_shutdown = matches!(request, Request::Shutdown);
    let response = {
        let mut d = daemon.lock().await;
        d.handle(request, now)
    };
    if is_shutdown {
        shutdown.notify_waiters();
    }
    response
}

/// Runs due sources, then lets the governor decide whether the result reaches the panel.
///
/// Polling and writing are deliberately separate. Polling is free, so sources may run
/// often and keep the data fresh; the governor decides independently whether the rendered
/// pixels are worth a flash write. A one-minute poll against a fifteen-minute write floor
/// is not waste — it means the image that *does* get written is seconds old, not fifteen
/// minutes old.
async fn scheduler_loop(daemon: Arc<Mutex<Daemon>>, shutdown: Arc<Notify>) {
    let mut ticker = tokio::time::interval(TICK);
    let mut last_checked: DateTime<Utc> = Utc::now();
    let mut last_reconnect_check = Utc::now();
    // Per-source: when we last fired it, so a restart does not replay the whole schedule.
    let mut fired: HashMap<String, DateTime<Utc>> = HashMap::new();

    loop {
        tokio::select! {
            _ = shutdown.notified() => break,
            _ = ticker.tick() => {}
        }
        let now = Utc::now();

        let specs: Vec<SourceSpec> = {
            let d = daemon.lock().await;
            d.registry.sources.clone()
        };

        let mut any_ran = false;
        for spec in specs {
            if !is_due(&spec, last_checked, now, &fired) {
                continue;
            }
            fired.insert(spec.id.clone(), now);
            match run_source(&spec, now).await {
                Ok(points) => {
                    tracing::debug!(source = %spec.id, points = points.len(), "source ran");
                    let mut d = daemon.lock().await;
                    d.absorb(&spec.id, points, now);
                    any_ran = true;
                }
                Err(e) => {
                    // A broken source must not take out the screen: its previous readings
                    // stay and go stale on their own TTL.
                    tracing::warn!(source = %spec.id, error = %e, "source failed");
                    let mut d = daemon.lock().await;
                    d.record_source_failure(&spec.id, e.to_string(), now);
                }
            }
        }
        last_checked = now;

        if any_ran {
            let mut d = daemon.lock().await;
            let outcome = d.render(true, Some(Origin::Scheduled), now);
            tracing::debug!(?outcome, "scheduled render");
        }

        if (now - last_reconnect_check).num_seconds() >= RECONNECT_CHECK_SECS {
            last_reconnect_check = now;
            let mut d = daemon.lock().await;
            if let Some(outcome) = d.check_reconnect(now) {
                tracing::info!(?outcome, "panel reappeared; refreshed");
            }
        }
    }
}

/// True when `spec`'s cron schedule has an occurrence in `(after, now]`.
pub(crate) fn is_due(
    spec: &SourceSpec,
    after: DateTime<Utc>,
    now: DateTime<Utc>,
    fired: &HashMap<String, DateTime<Utc>>,
) -> bool {
    let Ok(schedule) = spec.parsed_schedule() else {
        return false;
    };
    // Never fire twice for the same occurrence, even if ticks overlap.
    let since = fired.get(&spec.id).copied().unwrap_or(after).max(after);
    schedule
        .after(&since)
        .next()
        .map(|next| next <= now)
        .unwrap_or(false)
}

/// Convenience for clients: send one request and read one response.
pub async fn request(socket: &Path, req: &Request) -> std::io::Result<Response> {
    let stream = UnixStream::connect(socket).await.map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!(
                "could not reach the catbus99 daemon at {}: {e}. Is it running? Start it with `catbus99 daemon`.",
                socket.display()
            ),
        )
    })?;
    let (read_half, mut write_half) = stream.into_split();
    let mut text = serde_json::to_string(req)?;
    text.push('\n');
    write_half.write_all(text.as_bytes()).await?;
    write_half.flush().await?;

    let mut lines = BufReader::new(read_half).lines();
    match lines.next_line().await? {
        Some(line) => Ok(serde_json::from_str(&line)?),
        None => Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "daemon closed the connection without replying",
        )),
    }
}

/// Send a request using the default socket path.
pub async fn request_default(req: &Request) -> std::io::Result<Response> {
    request(&default_socket_path(), req).await
}

#[cfg(test)]
mod scheduler_tests {
    use super::*;
    use crate::sources::SourceSpec;
    use chrono::TimeZone;

    fn spec(schedule: &str) -> SourceSpec {
        SourceSpec {
            id: "s".into(),
            command: vec!["/bin/true".into()],
            schedule: schedule.into(),
            timeout_ms: 1000,
            ttl_secs: 60,
        }
    }

    fn t(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 19, h, m, s).unwrap()
    }

    #[test]
    fn fires_when_an_occurrence_falls_in_the_window() {
        // Every minute at second 0; the window 11:59:59 -> 12:00:01 contains 12:00:00.
        let fired = HashMap::new();
        assert!(is_due(
            &spec("0 * * * * *"),
            t(11, 59, 59),
            t(12, 0, 1),
            &fired
        ));
    }

    #[test]
    fn does_not_fire_when_no_occurrence_falls_in_the_window() {
        let fired = HashMap::new();
        assert!(!is_due(
            &spec("0 * * * * *"),
            t(12, 0, 1),
            t(12, 0, 30),
            &fired
        ));
    }

    /// The guard that stops a one-second tick firing the same cron minute sixty times.
    #[test]
    fn does_not_fire_twice_for_the_same_occurrence() {
        let mut fired = HashMap::new();
        let after = t(11, 59, 59);
        assert!(is_due(&spec("0 * * * * *"), after, t(12, 0, 1), &fired));

        fired.insert("s".to_string(), t(12, 0, 1));
        for second in 2..60 {
            assert!(
                !is_due(&spec("0 * * * * *"), after, t(12, 0, second), &fired),
                "re-fired at 12:00:{second}"
            );
        }
        // ...but the next minute does fire.
        assert!(is_due(&spec("0 * * * * *"), after, t(12, 1, 0), &fired));
    }

    /// After a long stall (laptop asleep, daemon paused) a source should run *once* to
    /// catch up, not once for every occurrence it missed.
    #[test]
    fn a_long_gap_produces_a_single_catch_up_run() {
        let mut fired = HashMap::new();
        let after = t(9, 0, 0);
        let now = t(12, 0, 0); // three hours of missed minutes

        assert!(is_due(&spec("0 * * * * *"), after, now, &fired));
        fired.insert("s".to_string(), now);
        assert!(!is_due(&spec("0 * * * * *"), after, now, &fired));
    }

    /// An unparseable schedule must disable that source quietly rather than panic the
    /// scheduler loop and take every other source down with it.
    #[test]
    fn an_invalid_schedule_never_fires_and_never_panics() {
        let fired = HashMap::new();
        assert!(!is_due(
            &spec("every so often"),
            t(0, 0, 0),
            t(23, 59, 59),
            &fired
        ));
    }

    #[test]
    fn a_five_minute_schedule_only_fires_on_its_boundaries() {
        let fired = HashMap::new();
        let s = spec("0 */5 * * * *");
        assert!(is_due(&s, t(12, 4, 59), t(12, 5, 1), &fired));
        assert!(!is_due(&s, t(12, 5, 1), t(12, 6, 0), &fired));
        assert!(!is_due(&s, t(12, 6, 0), t(12, 9, 59), &fired));
        assert!(is_due(&s, t(12, 9, 59), t(12, 10, 0), &fired));
    }

    /// `fired` is keyed per source, so one source firing must not suppress another.
    #[test]
    fn sources_do_not_suppress_each_other() {
        let mut fired = HashMap::new();
        fired.insert("s".to_string(), t(12, 0, 1));

        let mut other = spec("0 * * * * *");
        other.id = "other".into();
        assert!(is_due(&other, t(11, 59, 59), t(12, 0, 1), &fired));
    }
}
