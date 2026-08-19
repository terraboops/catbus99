//! Data sources: user programs that produce readings for the screen.
//!
//! A source is **any executable**, in any language. It is run on a schedule, must exit 0,
//! and must print one JSON object on stdout:
//!
//! ```json
//! {"datapoints": [
//!   {"key": "session_pct", "value": 0.62, "unit": "ratio", "label": "Session"},
//!   {"key": "resets_at",  "value": "2026-08-18T22:00:00Z"}
//! ]}
//! ```
//!
//! Choosing a subprocess contract over an in-process plugin API is deliberate: a Rust
//! plugin interface would have restricted contributors to Rust, where this lets a user's
//! existing shell, Python, or Swift script work untouched.

use catbus99_model::{DataPoint, Value};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("source {id}: failed to start {command:?}: {source}")]
    Spawn {
        id: String,
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("source {id}: timed out after {timeout_ms}ms")]
    Timeout { id: String, timeout_ms: u64 },
    #[error("source {id}: exited with {code}: {stderr}")]
    ExitStatus {
        id: String,
        code: String,
        stderr: String,
    },
    #[error("source {id}: output was not valid JSON: {source}")]
    Json {
        id: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("source {id}: invalid schedule {schedule:?}: {message}")]
    Schedule {
        id: String,
        schedule: String,
        message: String,
    },
}

fn default_timeout() -> u64 {
    10_000
}
fn default_ttl() -> u64 {
    900
}

/// One registered source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSpec {
    pub id: String,
    /// Program and arguments. The first element is the executable.
    pub command: Vec<String>,
    /// A 6-field cron expression (with seconds), e.g. `0 */5 * * * *`.
    pub schedule: String,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    /// How long this source's readings stay fresh. Past it they render as stale.
    #[serde(default = "default_ttl")]
    pub ttl_secs: u64,
}

impl SourceSpec {
    /// Parse the cron expression, reporting which source is at fault.
    pub fn parsed_schedule(&self) -> Result<cron::Schedule, SourceError> {
        use std::str::FromStr;
        cron::Schedule::from_str(&self.schedule).map_err(|e| SourceError::Schedule {
            id: self.id.clone(),
            schedule: self.schedule.clone(),
            message: e.to_string(),
        })
    }

    /// Expand a leading `~` so config files can use home-relative paths.
    fn resolved_program(&self) -> PathBuf {
        let raw = self.command.first().cloned().unwrap_or_default();
        if let Some(rest) = raw.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest);
            }
        }
        PathBuf::from(raw)
    }
}

/// The `sources.toml` registry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceRegistry {
    #[serde(default, rename = "source")]
    pub sources: Vec<SourceSpec>,
}

impl SourceRegistry {
    pub fn load(path: &Path) -> Result<Self, SourceError> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|source| SourceError::Parse {
                path: path.to_path_buf(),
                source,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(SourceError::Read {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    pub fn get(&self, id: &str) -> Option<&SourceSpec> {
        self.sources.iter().find(|s| s.id == id)
    }
}

/// One reading as a source reports it, before the daemon stamps it.
#[derive(Debug, Clone, Deserialize)]
pub struct RawPoint {
    pub key: String,
    pub value: Value,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    /// Overrides the source's default TTL for this reading.
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

/// The document a source prints.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceOutput {
    pub datapoints: Vec<RawPoint>,
}

/// Outcome of one source run.
#[derive(Debug, Clone, Default)]
pub struct RunRecord {
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_ok: Option<bool>,
    pub last_error: Option<String>,
    pub points_produced: usize,
}

/// Tracks per-source run history.
pub type RunHistory = HashMap<String, RunRecord>;

/// Run a source and convert its output into stamped data points.
///
/// Failure is contained: a source that crashes, hangs, or prints nonsense produces an
/// error for the log and leaves the previous readings in place to go stale on their own.
/// One broken source must not take out the screen.
pub async fn run_source(
    spec: &SourceSpec,
    now: DateTime<Utc>,
) -> Result<Vec<DataPoint>, SourceError> {
    let program = spec.resolved_program();
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(spec.command.iter().skip(1));
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let child = cmd.spawn().map_err(|source| SourceError::Spawn {
        id: spec.id.clone(),
        command: program.display().to_string(),
        source,
    })?;

    let output = tokio::time::timeout(
        Duration::from_millis(spec.timeout_ms),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| SourceError::Timeout {
        id: spec.id.clone(),
        timeout_ms: spec.timeout_ms,
    })?
    .map_err(|source| SourceError::Spawn {
        id: spec.id.clone(),
        command: program.display().to_string(),
        source,
    })?;

    if !output.status.success() {
        return Err(SourceError::ExitStatus {
            id: spec.id.clone(),
            code: output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into()),
            stderr: String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(200)
                .collect(),
        });
    }

    let parsed: SourceOutput =
        serde_json::from_slice(&output.stdout).map_err(|source| SourceError::Json {
            id: spec.id.clone(),
            source,
        })?;

    Ok(parsed
        .datapoints
        .into_iter()
        .map(|p| DataPoint {
            source: spec.id.clone(),
            key: p.key,
            value: p.value,
            unit: p.unit,
            label: p.label,
            observed_at: now,
            ttl_secs: Some(p.ttl_secs.unwrap_or(spec.ttl_secs)),
        })
        .collect())
}
