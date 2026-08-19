//! The write governor: the single place that decides whether pixels reach the panel.
//!
//! The display's flash is rated at 100,000 program/erase cycles and exposes no read-back,
//! so we cannot measure wear — we can only bound it. Every endurance guarantee catbus99
//! makes depends on all writes passing through [`Governor::decide`]:
//!
//! * **Change-skip** — a container byte-identical to the last upload is never sent.
//! * **Interval floor** — scheduled writes are spaced, with a hard 5-minute minimum that
//!   configuration cannot lower.
//! * **Interactive burst** — human-driven writes draw on a separate, bounded hourly
//!   allowance, because someone iterating on a layout needs faster feedback than five
//!   minutes but should not get an unlimited one.
//! * **Odometer** — every upload is counted and persisted, so the cost is visible rather
//!   than theoretical.
//!
//! [`Governor::decide`] is pure and fully testable. [`Governor::upload_to_panel`] is the
//! **only** public way to put pixels on the display anywhere in catbus99: the transport's
//! bulk-upload function is crate-private, so this is not a convention callers must
//! remember but the only route the compiler allows.

use crate::transport::{Device, HidError, Interface};
use catbus99_proto::wear;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GovernorError {
    #[error("could not read wear state at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write wear state at {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("wear state at {path} is corrupt: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// Which allowance a write draws on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    /// Automatic writes from the scheduler. Subject to the interval floor.
    Scheduled,
    /// A person asked for this now. Bounded hourly burst instead of the interval.
    Interactive,
    /// One upload after the display reappeared, because the custom image is genuinely
    /// gone: a power cycle returns the panel to its native screen.
    Recovery,
}

/// What the governor decided, and why.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Send it.
    Upload,
    /// Byte-identical to what is already displayed.
    SkipUnchanged,
    /// Too soon for this lane.
    RateLimited { retry_after_secs: u64 },
    /// The interactive hourly allowance is spent.
    BurstExhausted { retry_after_secs: u64 },
}

impl Decision {
    pub fn will_upload(&self) -> bool {
        matches!(self, Decision::Upload)
    }

    /// A short reason suitable for a CLI line or an MCP tool result.
    pub fn reason(&self) -> String {
        match self {
            Decision::Upload => "upload".into(),
            Decision::SkipUnchanged => "unchanged: the panel already shows this".into(),
            Decision::RateLimited { retry_after_secs } => {
                format!(
                    "rate limited: next scheduled write allowed in {}",
                    human(*retry_after_secs)
                )
            }
            Decision::BurstExhausted { retry_after_secs } => {
                format!(
                    "interactive burst spent: retry in {}",
                    human(*retry_after_secs)
                )
            }
        }
    }
}

fn human(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// Tunable limits. The hard floor is not among them by design.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernorConfig {
    /// Target spacing between scheduled writes.
    pub write_interval_secs: u64,
    /// Interactive writes allowed per rolling hour.
    pub interactive_per_hour: u32,
}

impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            write_interval_secs: wear::DEFAULT_WRITE_INTERVAL_SECS,
            interactive_per_hour: 12,
        }
    }
}

impl GovernorConfig {
    /// The effective interval, never below the hard floor.
    ///
    /// A user may opt into faster updates; they may not opt into a rate that destroys the
    /// panel inside a year, so this clamp is applied on read rather than trusted from the
    /// config file.
    pub fn effective_interval_secs(&self) -> u64 {
        self.write_interval_secs.max(wear::MIN_WRITE_INTERVAL_SECS)
    }
}

/// Persisted odometer and rate-limit state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WearState {
    pub total_uploads: u64,
    pub total_bytes: u64,
    pub last_upload_at: Option<DateTime<Utc>>,
    /// SHA-256 of the last container actually sent, for change-skip.
    pub last_hash: Option<String>,
    /// Timestamps of recent interactive writes, pruned to the last hour.
    #[serde(default)]
    pub interactive_recent: Vec<DateTime<Utc>>,
    /// Uploads per calendar day (UTC), for reporting.
    #[serde(default)]
    pub uploads_by_day: BTreeMap<String, u64>,
    #[serde(default)]
    pub uploads_by_lane: BTreeMap<String, u64>,
}

/// Hash a container the same way the change-skip rule does.
pub fn hash_payload(payload: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(payload);
    format!("{:x}", h.finalize())
}

/// Default location of the persisted odometer.
///
/// Migrates a file left by an earlier layout, because a counter that silently restarts
/// at zero would under-report how much of the panel's rated life has been used.
pub fn default_state_path() -> PathBuf {
    let path = crate::paths::state_dir().join("wear.json");
    if let Some(from) = crate::paths::migrate_legacy_file("wear.json", &path) {
        eprintln!("catbus99: moved wear odometer from {}", from.display());
    }
    path
}

/// Decides whether a write may proceed, and remembers what happened.
#[derive(Debug, Clone)]
pub struct Governor {
    pub config: GovernorConfig,
    pub state: WearState,
    path: Option<PathBuf>,
}

impl Governor {
    /// An in-memory governor that persists nothing. Used by tests and dry runs.
    pub fn ephemeral(config: GovernorConfig) -> Self {
        Self {
            config,
            state: WearState::default(),
            path: None,
        }
    }

    /// Load persisted state, or start fresh if there is none.
    pub fn load(config: GovernorConfig, path: &Path) -> Result<Self, GovernorError> {
        let state = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|source| GovernorError::Parse {
                path: path.to_path_buf(),
                source,
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => WearState::default(),
            Err(source) => {
                return Err(GovernorError::Read {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        Ok(Self {
            config,
            state,
            path: Some(path.to_path_buf()),
        })
    }

    pub fn save(&self) -> Result<(), GovernorError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| GovernorError::Write {
                path: path.clone(),
                source,
            })?;
        }
        let text = serde_json::to_string_pretty(&self.state).expect("state serialises");
        std::fs::write(path, text).map_err(|source| GovernorError::Write {
            path: path.clone(),
            source,
        })
    }

    /// Decide whether this container may be uploaded now. Does not mutate state.
    pub fn decide(&self, payload: &[u8], lane: Lane, now: DateTime<Utc>) -> Decision {
        let hash = hash_payload(payload);

        // Change-skip first: an unchanged image is free to refuse in every lane,
        // including recovery, where re-sending identical bytes buys nothing.
        if self.state.last_hash.as_deref() == Some(hash.as_str()) {
            return Decision::SkipUnchanged;
        }

        match lane {
            // Recovery deliberately bypasses the interval: the panel reverted to its
            // native screen, so the custom image is genuinely absent.
            Lane::Recovery => Decision::Upload,

            Lane::Scheduled => match self.state.last_upload_at {
                None => Decision::Upload,
                Some(last) => {
                    let interval = self.config.effective_interval_secs() as i64;
                    let elapsed = (now - last).num_seconds();
                    if elapsed >= interval {
                        Decision::Upload
                    } else {
                        Decision::RateLimited {
                            retry_after_secs: (interval - elapsed).max(1) as u64,
                        }
                    }
                }
            },

            Lane::Interactive => {
                let recent = self.interactive_within_hour(now);
                if (recent.len() as u32) < self.config.interactive_per_hour {
                    Decision::Upload
                } else {
                    // The allowance frees up when the oldest write in the window ages out.
                    let oldest = recent.first().copied().unwrap_or(now);
                    let retry = (oldest + Duration::hours(1) - now).num_seconds().max(1) as u64;
                    Decision::BurstExhausted {
                        retry_after_secs: retry,
                    }
                }
            }
        }
    }

    fn interactive_within_hour(&self, now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
        let cutoff = now - Duration::hours(1);
        let mut v: Vec<DateTime<Utc>> = self
            .state
            .interactive_recent
            .iter()
            .copied()
            .filter(|t| *t > cutoff)
            .collect();
        v.sort();
        v
    }

    /// Record an upload that actually happened.
    pub fn record_upload(&mut self, payload: &[u8], lane: Lane, now: DateTime<Utc>) {
        self.state.total_uploads += 1;
        self.state.total_bytes += payload.len() as u64;
        self.state.last_upload_at = Some(now);
        self.state.last_hash = Some(hash_payload(payload));

        let day = now.format("%Y-%m-%d").to_string();
        *self.state.uploads_by_day.entry(day).or_insert(0) += 1;

        let lane_key = match lane {
            Lane::Scheduled => "scheduled",
            Lane::Interactive => "interactive",
            Lane::Recovery => "recovery",
        };
        *self
            .state
            .uploads_by_lane
            .entry(lane_key.into())
            .or_insert(0) += 1;

        if lane == Lane::Interactive {
            self.state.interactive_recent.push(now);
        }
        // Keep the rolling window from growing without bound.
        self.state.interactive_recent = self.interactive_within_hour(now);
    }

    /// Record an upload that failed part-way through.
    ///
    /// A transfer that dies at report 9 of 16 has still written those nine reports to
    /// flash, so the wear happened and must be counted — an odometer that only counts
    /// successes drifts low over time, and drifting *low* is the dangerous direction.
    /// The displayed hash is cleared rather than set, because what is on the panel is now
    /// a torn image rather than the container we sent.
    pub fn record_failed_upload(&mut self, payload: &[u8], lane: Lane, now: DateTime<Utc>) {
        self.record_upload(payload, lane, now);
        self.invalidate_displayed();
        *self
            .state
            .uploads_by_lane
            .entry("failed".into())
            .or_insert(0) += 1;
    }

    /// Forget the last-uploaded hash, so the next write is not skipped as unchanged.
    ///
    /// Used when the panel is known to no longer show what we last sent — after a
    /// power cycle, for instance.
    pub fn invalidate_displayed(&mut self) {
        self.state.last_hash = None;
    }

    /// A human-readable odometer summary.
    pub fn report(&self) -> WearReport {
        let used = self.state.total_uploads;
        WearReport {
            total_uploads: used,
            total_bytes: self.state.total_bytes,
            budget_used_fraction: wear::budget_used(used),
            uploads_remaining: wear::budget_remaining(used),
            last_upload_at: self.state.last_upload_at,
            interval_secs: self.config.effective_interval_secs(),
            projected_years: wear::projected_years(self.config.effective_interval_secs()),
        }
    }
}

/// Odometer summary for `catbus99 wear` and the MCP `get_wear_budget` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WearReport {
    pub total_uploads: u64,
    pub total_bytes: u64,
    pub budget_used_fraction: f64,
    pub uploads_remaining: u64,
    pub last_upload_at: Option<DateTime<Utc>>,
    pub interval_secs: u64,
    pub projected_years: f64,
}

/// The outcome of an attempted panel write.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadOutcome {
    pub decision: Decision,
    pub uploaded: bool,
    /// Set when the transport failed after the governor allowed the write.
    pub device_error: Option<String>,
    pub bytes: usize,
    pub uploads_used: u64,
    pub uploads_remaining: u64,
}

impl Governor {
    /// Put a container on the display, if the rules allow it.
    ///
    /// This is the single public write path in catbus99. It consults [`Self::decide`],
    /// performs the transfer only on approval, and records the upload against the
    /// odometer -- so no caller can write without being counted, and none can skip the
    /// rate limits by reaching past this function. There is deliberately no override
    /// parameter: a bound that can be routed around is not a bound.
    pub fn upload_to_panel(
        &mut self,
        payload: &[u8],
        lane: Lane,
        now: DateTime<Utc>,
        timeout: std::time::Duration,
    ) -> Result<UploadOutcome, HidError> {
        let decision = self.decide(payload, lane, now);
        let mut outcome = UploadOutcome {
            decision: decision.clone(),
            uploaded: false,
            device_error: None,
            bytes: payload.len(),
            uploads_used: self.state.total_uploads,
            uploads_remaining: wear::budget_remaining(self.state.total_uploads),
        };
        if !decision.will_upload() {
            return Ok(outcome);
        }

        let device = Device::open(Interface::Tft)?;
        match device.upload_container(payload, timeout, None) {
            Ok(_) => {
                self.record_upload(payload, lane, now);
                let _ = self.save();
                outcome.uploaded = true;
                outcome.uploads_used = self.state.total_uploads;
                outcome.uploads_remaining = wear::budget_remaining(self.state.total_uploads);
                Ok(outcome)
            }
            Err(e) => {
                // The transfer may have written some reports before failing: that flash
                // wear is real, so count it. Clearing the displayed hash also lets the
                // retry through instead of being skipped as "unchanged", which would
                // leave a torn image on screen.
                self.record_failed_upload(payload, lane, now);
                let _ = self.save();
                outcome.device_error = Some(e.to_string());
                outcome.uploads_used = self.state.total_uploads;
                outcome.uploads_remaining = wear::budget_remaining(self.state.total_uploads);
                Ok(outcome)
            }
        }
    }
}
