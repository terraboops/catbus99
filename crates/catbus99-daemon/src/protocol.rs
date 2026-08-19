//! The control protocol spoken over the daemon's Unix socket.
//!
//! Newline-delimited JSON, one request per line, one response per line. Deliberately
//! plain text: the socket is the seam every client uses -- CLI and MCP server alike --
//! so being able to drive it with `nc` while debugging is worth more than a compact
//! binary encoding.

use catbus99_device::WearReport;
use catbus99_model::{DataPoint, Layout, Widget};
use serde::{Deserialize, Serialize};

/// Default socket path.
pub fn default_socket_path() -> std::path::PathBuf {
    catbus99_device::paths::runtime_dir().join("ctl.sock")
}

/// Why a write was requested. Chooses which governor allowance it draws on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// A person asked for it now.
    Interactive,
    /// The scheduler produced it.
    Scheduled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Liveness and a summary of daemon state.
    Status,
    /// The flash-endurance odometer.
    Wear,

    /// Replace the active layout.
    SetLayout {
        layout: Box<Layout>,
        #[serde(default)]
        render: bool,
        #[serde(default)]
        origin: Option<Origin>,
    },
    /// Fetch the active layout.
    GetLayout,
    /// Set or clear one slot's widget.
    SetWidget {
        slot: String,
        widget: Option<Box<Widget>>,
        #[serde(default)]
        render: bool,
        #[serde(default)]
        origin: Option<Origin>,
    },

    /// Insert or update a data point.
    PushDataPoint { point: Box<DataPoint> },
    /// All current data points.
    GetDataPoints,

    /// Compose the active layout and optionally send it to the panel.
    Render {
        /// False composes and reports the governor's verdict without writing.
        #[serde(default)]
        execute: bool,
        #[serde(default)]
        origin: Option<Origin>,
    },
    /// Render the active layout and return it as a PNG, never touching the device.
    Preview,

    /// Display an image or animation from a file.
    ShowImage {
        path: String,
        #[serde(default)]
        execute: bool,
        #[serde(default)]
        max_frames: Option<usize>,
        #[serde(default)]
        origin: Option<Origin>,
    },

    /// Set the keyboard's RTC. Config channel; not a flash write.
    SyncClock,

    /// Registered data sources.
    ListSources,
    /// Run one source now and merge its data points.
    RunSource { id: String },

    /// Stop the daemon.
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Ok,
    Status(Box<StatusReport>),
    Wear(Box<WearReport>),
    Layout(Box<Layout>),
    DataPoints {
        points: Vec<DataPoint>,
    },
    /// The outcome of a write request, including why it was refused.
    Write(WriteOutcome),
    /// A base64-encoded PNG of the composed screen.
    Preview {
        png_base64: String,
        width: u32,
        height: u32,
    },
    Sources {
        sources: Vec<SourceStatus>,
    },
    Error {
        message: String,
    },
}

/// What the governor decided about a write, in a form clients can act on.
///
/// Every write-capable operation returns this rather than a bare success flag, so a
/// caller -- a person or an agent -- learns *why* nothing appeared and when to try again,
/// instead of retrying blindly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WriteOutcome {
    pub uploaded: bool,
    /// `upload`, `unchanged`, `rate_limited`, or `burst_exhausted`.
    pub reason: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reports: Option<usize>,
    pub uploads_used: u64,
    pub uploads_remaining: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusReport {
    pub version: String,
    pub layout_id: String,
    pub slots: usize,
    pub data_points: usize,
    pub sources: usize,
    pub device_present: bool,
    pub uploads_used: u64,
    pub last_upload_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceStatus {
    pub id: String,
    pub schedule: String,
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_ok: Option<bool>,
    pub last_error: Option<String>,
    pub points_produced: usize,
}
