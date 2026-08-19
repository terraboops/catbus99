//! The catbus99 daemon: the single process that owns the display.
//!
//! # Why a daemon at all
//!
//! Every flash-endurance guarantee depends on all writes passing through one governor.
//! If the CLI, the scheduler, and the MCP server each wrote to the device directly, each
//! could satisfy its own rate limit while together tripling the write rate, and the
//! odometer would undercount. Centralising also gives one place that knows what is
//! currently on screen, which is what makes change-skip work across clients.
//!
//! Clients -- the CLI and the MCP server alike -- are thin wrappers over the Unix socket
//! protocol in [`protocol`].

pub mod protocol;
pub mod server;
pub mod sources;

use catbus99_device::{Decision, Governor, GovernorConfig, Lane};
use catbus99_device::{Device, Interface, UploadOutcome as DeviceOutcome};
use catbus99_model::{DataPoint, DataStore, Layout};
use catbus99_proto::container::{build_container, BLOCK_SIZE};
use catbus99_render::{compose, frames_to_container, load_frames, rgb565_to_rgba, to_rgb565, Fit};
use chrono::{DateTime, Utc};
use protocol::{Origin, Request, Response, SourceStatus, StatusReport, WriteOutcome};
use sources::{RunHistory, RunRecord, SourceRegistry};
use std::path::PathBuf;
use std::time::Duration;

/// Where the daemon keeps its files.
#[derive(Debug, Clone)]
pub struct Paths {
    pub socket: PathBuf,
    pub wear_state: PathBuf,
    pub sources: PathBuf,
    pub layout: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        let config = catbus99_device::paths::config_dir();
        Self {
            socket: protocol::default_socket_path(),
            wear_state: catbus99_device::default_state_path(),
            sources: config.join("sources.toml"),
            layout: config.join("layout.json"),
        }
    }
}

/// Daemon state. Not `Sync`; the server wraps it in a mutex so requests serialise.
pub struct Daemon {
    pub layout: Layout,
    pub store: DataStore,
    pub governor: Governor,
    pub registry: SourceRegistry,
    pub history: RunHistory,
    pub paths: Paths,
    /// Whether the panel was present at the last check, for reconnect recovery.
    ///
    /// Seeded by probing at construction. Starting this at `false` would make the first
    /// check look like a reappearance and spend a recovery upload on every daemon start —
    /// and recovery deliberately bypasses the interval, so it is the one lane that cannot
    /// be rate-limited away.
    device_was_present: bool,
    pub shutdown_requested: bool,
}

impl Daemon {
    pub fn new(layout: Layout, paths: Paths) -> Result<Self, Box<dyn std::error::Error>> {
        let governor = Governor::load(GovernorConfig::default(), &paths.wear_state)?;
        let registry = SourceRegistry::load(&paths.sources)?;
        let device_was_present = catbus99_device::probe()
            .map(|r| !r.tft_candidates.is_empty())
            .unwrap_or(false);

        Ok(Self {
            layout,
            store: DataStore::new(),
            governor,
            registry,
            history: RunHistory::new(),
            paths,
            device_was_present,
            shutdown_requested: false,
        })
    }

    fn lane_for(origin: Option<Origin>) -> Lane {
        match origin.unwrap_or(Origin::Interactive) {
            Origin::Interactive => Lane::Interactive,
            Origin::Scheduled => Lane::Scheduled,
        }
    }

    /// The outcome of a *hypothetical* write, for dry runs.
    ///
    /// `uploaded` must always mean "bytes actually reached the panel". A dry run that
    /// reported `uploaded: true` would mislead a person and would be acted on by an
    /// agent, so an approved-but-not-performed write is reported distinctly.
    fn dry_outcome(&self, decision: &Decision, bytes: Option<usize>) -> WriteOutcome {
        let mut o = self.outcome(decision, bytes);
        if o.uploaded {
            o.uploaded = false;
            o.reason = "would_upload".into();
            o.detail = "allowed, but not sent: pass execute to write it".into();
        } else {
            o.detail = format!("would not be sent -- {}", o.detail);
        }
        o
    }

    fn outcome(&self, decision: &Decision, bytes: Option<usize>) -> WriteOutcome {
        let report = self.governor.report();
        let (reason, retry) = match decision {
            Decision::Upload => ("upload", None),
            Decision::SkipUnchanged => ("unchanged", None),
            Decision::RateLimited { retry_after_secs } => ("rate_limited", Some(*retry_after_secs)),
            Decision::BurstExhausted { retry_after_secs } => {
                ("burst_exhausted", Some(*retry_after_secs))
            }
        };
        WriteOutcome {
            uploaded: decision.will_upload(),
            reason: reason.to_string(),
            detail: decision.reason(),
            retry_after_secs: retry,
            bytes,
            reports: bytes.map(|b| b / BLOCK_SIZE),
            uploads_used: report.total_uploads,
            uploads_remaining: report.uploads_remaining,
        }
    }

    /// Send pixels to the panel.
    ///
    /// Delegates to [`Governor::upload_to_panel`], which is the only public write path in
    /// catbus99 — the transport's bulk upload is crate-private, so the daemon physically
    /// cannot bypass the rate limits or the odometer even if it wanted to.
    pub fn upload(&mut self, payload: &[u8], lane: Lane, now: DateTime<Utc>) -> WriteOutcome {
        let result = self
            .governor
            .upload_to_panel(payload, lane, now, Duration::from_millis(5000));

        match result {
            Err(e) => {
                self.device_was_present = false;
                self.device_error(payload.len(), e.to_string())
            }
            Ok(DeviceOutcome {
                device_error: Some(msg),
                ..
            }) => {
                self.device_was_present = false;
                self.device_error(payload.len(), msg)
            }
            Ok(o) => {
                if o.uploaded {
                    self.device_was_present = true;
                }
                self.outcome(&o.decision, Some(payload.len()))
            }
        }
    }

    fn device_error(&self, bytes: usize, detail: String) -> WriteOutcome {
        let report = self.governor.report();
        WriteOutcome {
            uploaded: false,
            reason: "device_error".into(),
            detail,
            retry_after_secs: None,
            bytes: Some(bytes),
            reports: None,
            uploads_used: report.total_uploads,
            uploads_remaining: report.uploads_remaining,
        }
    }

    /// Compose the active layout into an upload container.
    pub fn compose_payload(&self, now: DateTime<Utc>) -> Result<Vec<u8>, String> {
        let img = compose(&self.layout, &self.store, now);
        // Stills use a single frame: 8 reports instead of 16, half the bytes.
        let frame = to_rgb565(&img, false);
        build_container(&[&frame], &[]).map_err(|e| e.to_string())
    }

    pub fn render(
        &mut self,
        execute: bool,
        origin: Option<Origin>,
        now: DateTime<Utc>,
    ) -> Response {
        let payload = match self.compose_payload(now) {
            Ok(p) => p,
            Err(message) => return Response::Error { message },
        };
        if !execute {
            let decision = self.governor.decide(&payload, Self::lane_for(origin), now);
            return Response::Write(self.dry_outcome(&decision, Some(payload.len())));
        }
        let lane = Self::lane_for(origin);
        Response::Write(self.upload(&payload, lane, now))
    }

    /// Detect the panel returning after an absence and refresh it once.
    ///
    /// A power cycle restores the keyboard's native screen, so the custom image really is
    /// gone; the recovery lane exists to put it back without waiting for the interval.
    pub fn check_reconnect(&mut self, now: DateTime<Utc>) -> Option<WriteOutcome> {
        let present = catbus99_device::probe()
            .map(|r| !r.tft_candidates.is_empty())
            .unwrap_or(false);
        let reappeared = present && !self.device_was_present;
        self.device_was_present = present;

        if !reappeared {
            return None;
        }
        // The panel reverted to its native screen, so whatever we last sent is not shown.
        self.governor.invalidate_displayed();
        let payload = self.compose_payload(now).ok()?;
        Some(self.upload(&payload, Lane::Recovery, now))
    }

    pub fn handle(&mut self, request: Request, now: DateTime<Utc>) -> Response {
        match request {
            Request::Status => Response::Status(Box::new(StatusReport {
                version: env!("CARGO_PKG_VERSION").into(),
                layout_id: self.layout.id.clone(),
                slots: self.layout.slots.len(),
                data_points: self.store.len(),
                sources: self.registry.sources.len(),
                device_present: catbus99_device::probe()
                    .map(|r| !r.tft_candidates.is_empty())
                    .unwrap_or(false),
                uploads_used: self.governor.report().total_uploads,
                last_upload_at: self.governor.state.last_upload_at,
            })),

            Request::Wear => Response::Wear(Box::new(self.governor.report())),

            Request::GetLayout => Response::Layout(Box::new(self.layout.clone())),

            Request::SetLayout {
                layout,
                render,
                origin,
            } => {
                self.layout = *layout;
                if render {
                    self.render(true, origin, now)
                } else {
                    Response::Ok
                }
            }

            Request::SetWidget {
                slot,
                widget,
                render,
                origin,
            } => match self.layout.slot_mut(&slot) {
                None => Response::Error {
                    message: format!(
                        "no slot {slot:?}; available: {}",
                        self.layout
                            .slots
                            .iter()
                            .map(|s| s.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                },
                Some(s) => {
                    s.widget = widget.map(|w| *w);
                    if render {
                        self.render(true, origin, now)
                    } else {
                        Response::Ok
                    }
                }
            },

            Request::PushDataPoint { point } => {
                self.store.insert(*point);
                Response::Ok
            }

            Request::GetDataPoints => Response::DataPoints {
                points: self.store.all().cloned().collect(),
            },

            Request::Render { execute, origin } => self.render(execute, origin, now),

            Request::Preview => {
                let img = compose(&self.layout, &self.store, now);
                let frame = to_rgb565(&img, false);
                match encode_png(&rgb565_to_rgba(&frame)) {
                    Ok(png) => {
                        use base64::Engine;
                        Response::Preview {
                            png_base64: base64::engine::general_purpose::STANDARD.encode(png),
                            width: catbus99_model::SCREEN_W,
                            height: catbus99_model::SCREEN_H,
                        }
                    }
                    Err(message) => Response::Error { message },
                }
            }

            Request::ShowImage {
                path,
                execute,
                max_frames,
                origin,
            } => {
                let frames = match load_frames(std::path::Path::new(&path), Fit::Contain, true) {
                    Ok(f) => f,
                    Err(e) => {
                        return Response::Error {
                            message: e.to_string(),
                        }
                    }
                };
                let payload = match frames_to_container(&frames, max_frames.unwrap_or(16)) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::Error {
                            message: e.to_string(),
                        }
                    }
                };
                if !execute {
                    let decision = self.governor.decide(&payload, Self::lane_for(origin), now);
                    return Response::Write(self.dry_outcome(&decision, Some(payload.len())));
                }
                let lane = Self::lane_for(origin);
                Response::Write(self.upload(&payload, lane, now))
            }

            Request::SyncClock => match sync_clock(now) {
                Ok(()) => Response::Ok,
                Err(message) => Response::Error { message },
            },

            Request::ListSources => Response::Sources {
                sources: self
                    .registry
                    .sources
                    .iter()
                    .map(|s| {
                        let h = self.history.get(&s.id).cloned().unwrap_or_default();
                        SourceStatus {
                            id: s.id.clone(),
                            schedule: s.schedule.clone(),
                            last_run_at: h.last_run_at,
                            last_ok: h.last_ok,
                            last_error: h.last_error,
                            points_produced: h.points_produced,
                        }
                    })
                    .collect(),
            },

            Request::RunSource { .. } => Response::Error {
                message: "run_source is handled by the server, which can await the process".into(),
            },

            Request::Shutdown => {
                self.shutdown_requested = true;
                Response::Ok
            }
        }
    }

    /// Merge a source's readings into the store and record the run.
    pub fn absorb(&mut self, id: &str, points: Vec<DataPoint>, now: DateTime<Utc>) {
        let count = points.len();
        for p in points {
            self.store.insert(p);
        }
        self.history.insert(
            id.to_string(),
            RunRecord {
                last_run_at: Some(now),
                last_ok: Some(true),
                last_error: None,
                points_produced: count,
            },
        );
    }

    pub fn record_source_failure(&mut self, id: &str, error: String, now: DateTime<Utc>) {
        let previous = self.history.get(id).cloned().unwrap_or_default();
        self.history.insert(
            id.to_string(),
            RunRecord {
                last_run_at: Some(now),
                last_ok: Some(false),
                last_error: Some(error),
                points_produced: previous.points_produced,
            },
        );
    }
}

fn encode_png(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(buf.into_inner())
}

fn sync_clock(now: DateTime<Utc>) -> Result<(), String> {
    use catbus99_proto::clock::{build_set_clock, is_clock_ack, ClockTime, CONFIG_PACKET_SIZE};
    use chrono::{Datelike, Timelike};

    let local = now.with_timezone(&chrono::Local);
    let when = ClockTime::new(
        local.year() as u16,
        local.month() as u8,
        local.day() as u8,
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
    )
    .map_err(|e| e.to_string())?;
    let packet = build_set_clock(when).map_err(|e| e.to_string())?;

    let device = Device::open(Interface::Config).map_err(|e| e.to_string())?;
    device.write_report(&packet).map_err(|e| e.to_string())?;
    let reply = device
        .read_report(CONFIG_PACKET_SIZE, Duration::from_millis(5000))
        .map_err(|e| e.to_string())?;
    if is_clock_ack(&packet, &reply) {
        Ok(())
    } else {
        Err("clock command was not acknowledged".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catbus99_model::Layout;

    fn paths_in(dir: &std::path::Path) -> Paths {
        Paths {
            socket: dir.join("ctl.sock"),
            wear_state: dir.join("wear.json"),
            sources: dir.join("sources.toml"),
            layout: dir.join("layout.json"),
        }
    }

    /// Starting the daemon must not look like a reconnect. Recovery bypasses the write
    /// interval, so a false positive here would burn a flash cycle every start-up.
    #[test]
    fn a_fresh_daemon_does_not_report_a_reconnect_for_an_already_present_panel() {
        let dir = std::env::temp_dir().join(format!("catbus99-d-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = Daemon::new(Layout::new("t"), paths_in(&dir)).unwrap();

        // Whatever the probe found, the recorded state must match it, so the first check
        // sees no transition.
        let present_now = catbus99_device::probe()
            .map(|r| !r.tft_candidates.is_empty())
            .unwrap_or(false);
        assert_eq!(d.device_was_present, present_now);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_widget_on_an_unknown_slot_lists_the_real_ones() {
        let dir = std::env::temp_dir().join(format!("catbus99-d2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let layout = Layout::new("t").with_slot(
            "known",
            catbus99_model::Rect::new(0, 0, 10, 10),
            catbus99_model::Widget::Blank,
        );
        let mut d = Daemon::new(layout, paths_in(&dir)).unwrap();

        let r = d.handle(
            Request::SetWidget {
                slot: "nope".into(),
                widget: None,
                render: false,
                origin: None,
            },
            Utc::now(),
        );
        match r {
            Response::Error { message } => {
                assert!(message.contains("nope"));
                assert!(
                    message.contains("known"),
                    "should name the slots that do exist: {message}"
                );
            }
            other => panic!("expected an error, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::*;
    use catbus99_model::Layout;

    fn daemon() -> Daemon {
        let dir = std::env::temp_dir().join(format!("catbus99-o-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Daemon::new(
            Layout::new("t"),
            Paths {
                socket: dir.join("s"),
                wear_state: dir.join("w.json"),
                sources: dir.join("src.toml"),
                layout: dir.join("l.json"),
            },
        )
        .unwrap()
    }

    /// `uploaded` means bytes reached the panel. A dry run must never claim otherwise:
    /// a person would be confused and an agent would act on it.
    #[test]
    fn a_dry_run_never_claims_it_uploaded() {
        let mut d = daemon();
        let r = d.handle(
            Request::Render {
                execute: false,
                origin: None,
            },
            Utc::now(),
        );
        match r {
            Response::Write(w) => {
                assert!(!w.uploaded, "a dry run reported uploaded=true");
                assert_eq!(w.reason, "would_upload");
                assert!(
                    w.bytes.unwrap_or(0) > 0,
                    "it should still say how big the write would be"
                );
            }
            other => panic!("expected a write outcome, got {other:?}"),
        }
    }

    #[test]
    fn a_dry_run_still_surfaces_a_refusal() {
        let mut d = daemon();
        // Record the exact payload the layout composes, so the next decision is a skip.
        let now = Utc::now();
        let payload = d.compose_payload(now).unwrap();
        d.governor
            .record_upload(&payload, catbus99_device::Lane::Interactive, now);

        match d.handle(
            Request::Render {
                execute: false,
                origin: None,
            },
            now,
        ) {
            Response::Write(w) => {
                assert!(!w.uploaded);
                assert_eq!(w.reason, "unchanged");
                assert!(w.detail.contains("would not be sent"));
            }
            other => panic!("expected a write outcome, got {other:?}"),
        }
    }
}
