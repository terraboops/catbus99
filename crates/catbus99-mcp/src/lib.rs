//! MCP server for the Epomaker TH99 Pro screen.
//!
//! Every tool is a thin client over the daemon's Unix socket. That is deliberate: the
//! daemon owns the device and the write governor lives inside the write path, so an agent
//! driving these tools inherits the flash-endurance limits automatically. There is no
//! path from a tool call to the panel that skips the governor, and no override parameter
//! anywhere in this surface.
//!
//! # Designing the surface for agents
//!
//! Two rules shape it:
//!
//! * **`preview_screen` costs nothing and is advertised first.** An agent can iterate on a
//!   layout indefinitely against a PNG without touching flash. Every tool description that
//!   can write points back at it.
//! * **Write tools report the governor's verdict, not just success.** A refusal comes back
//!   as `uploaded: false` with a reason and `retry_after_secs`, so a model learns to batch
//!   changes rather than retry blindly. `uploaded` always means bytes actually reached the
//!   panel — a dry run never claims otherwise.

use catbus99_daemon::protocol::{Origin, Request, Response};
use catbus99_daemon::server::request_default;
use catbus99_model::{DataPoint, Layout, Value, Widget};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, tool, tool_handler, tool_router, ServerHandler};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Default)]
pub struct Catbus99 {
    socket: Option<PathBuf>,
}

impl Catbus99 {
    pub fn new(socket: Option<PathBuf>) -> Self {
        Self { socket }
    }

    async fn call(&self, request: Request) -> String {
        let result = match &self.socket {
            Some(path) => catbus99_daemon::server::request(path, &request).await,
            None => request_default(&request).await,
        };
        match result {
            Ok(response) => render(&response),
            Err(e) => json_error(&e.to_string()),
        }
    }
}

fn json_error(message: &str) -> String {
    serde_json::json!({ "ok": false, "error": message }).to_string()
}

/// Turn a daemon response into the JSON an agent sees.
///
/// Public so the mapping can be tested directly: it is the contract an agent reasons
/// about, and a field quietly changing name or meaning would be invisible otherwise.
pub fn response_json(response: &Response) -> String {
    render(response)
}

fn render(response: &Response) -> String {
    let value = match response {
        Response::Ok => serde_json::json!({ "ok": true }),
        Response::Error { message } => serde_json::json!({ "ok": false, "error": message }),
        Response::Status(s) => serde_json::json!({ "ok": true, "status": s }),
        Response::Wear(w) => serde_json::json!({ "ok": true, "wear": w }),
        Response::Layout(l) => serde_json::json!({ "ok": true, "layout": l }),
        Response::DataPoints { points } => serde_json::json!({ "ok": true, "data_points": points }),
        Response::Sources { sources } => serde_json::json!({ "ok": true, "sources": sources }),
        Response::Preview {
            png_base64,
            width,
            height,
        } => serde_json::json!({
            "ok": true,
            "width": width,
            "height": height,
            "png_base64": png_base64,
            "note": "Rendered only. No flash write was performed."
        }),
        Response::Write(w) => serde_json::json!({
            "ok": true,
            "uploaded": w.uploaded,
            "reason": w.reason,
            "detail": w.detail,
            "retry_after_secs": w.retry_after_secs,
            "bytes": w.bytes,
            "uploads_used": w.uploads_used,
            "uploads_remaining": w.uploads_remaining,
        }),
    };
    serde_json::to_string_pretty(&value).unwrap_or_else(|e| json_error(&e.to_string()))
}

// --- tool parameter types ---

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetWidgetParams {
    /// Slot id to fill. Use `get_layout` to see the available slot ids.
    pub slot: String,
    /// The widget to place. Omit to clear the slot.
    pub widget: Option<Widget>,
    /// Send the result to the panel. Leave false to change the layout without a flash
    /// write, then call `preview_screen` to check it before committing.
    #[serde(default)]
    pub render: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetLayoutParams {
    /// The complete layout to install.
    pub layout: Layout,
    /// Send the result to the panel.
    #[serde(default)]
    pub render: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PushDataPointParams {
    /// Source name; groups related readings, e.g. "claude".
    pub source: String,
    /// Reading name, e.g. "session_pct".
    pub key: String,
    /// Numeric value. Progress bars expect 0.0 to 1.0.
    pub number: Option<f64>,
    /// Text value, if the reading is not numeric.
    pub text: Option<String>,
    /// Unit shown alongside the value, e.g. "%".
    pub unit: Option<String>,
    /// Seconds before the reading is considered stale and rendered dimmed.
    #[serde(default = "default_ttl")]
    pub ttl_secs: u64,
}

fn default_ttl() -> u64 {
    900
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RenderParams {
    /// Actually write to the panel. When false, reports what the governor *would* do
    /// without spending a flash cycle.
    #[serde(default)]
    pub execute: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ShowImageParams {
    /// Absolute path to a PNG, JPEG, or GIF. Animated GIFs keep their timing.
    pub path: String,
    /// Actually write to the panel.
    #[serde(default)]
    pub execute: bool,
    /// Maximum animation frames. Each frame costs 30,720 bytes of upload.
    pub max_frames: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunSourceParams {
    /// Source id, from `list_sources`.
    pub id: String,
}

#[tool_router]
impl Catbus99 {
    #[tool(
        description = "Render the current screen and return it as a PNG. Costs nothing and \
                       never touches the display's flash. Use this to check any change \
                       before writing it."
    )]
    async fn preview_screen(&self) -> String {
        self.call(Request::Preview).await
    }

    #[tool(
        description = "Daemon and keyboard status: active layout, number of data points and \
                       sources, whether the keyboard is connected, and uploads used."
    )]
    async fn get_status(&self) -> String {
        self.call(Request::Status).await
    }

    #[tool(
        description = "Flash-endurance budget. The display is rated for about 100,000 writes \
                       and cannot be replaced, so check this before making many changes."
    )]
    async fn get_wear_budget(&self) -> String {
        self.call(Request::Wear).await
    }

    #[tool(
        description = "The active layout, including every slot id you can target with \
                       set_widget."
    )]
    async fn get_layout(&self) -> String {
        self.call(Request::GetLayout).await
    }

    #[tool(description = "All current data points and how fresh they are.")]
    async fn get_data_points(&self) -> String {
        self.call(Request::GetDataPoints).await
    }

    #[tool(
        description = "Set or clear one slot's widget. Prefer render=false, then \
                       preview_screen to check the result, then render_screen to commit — \
                       that way iteration costs no flash writes."
    )]
    async fn set_widget(&self, Parameters(p): Parameters<SetWidgetParams>) -> String {
        self.call(Request::SetWidget {
            slot: p.slot,
            widget: p.widget.map(Box::new),
            render: p.render,
            origin: Some(Origin::Interactive),
        })
        .await
    }

    #[tool(description = "Replace the whole layout. The screen is 160x96 pixels.")]
    async fn set_layout(&self, Parameters(p): Parameters<SetLayoutParams>) -> String {
        self.call(Request::SetLayout {
            layout: Box::new(p.layout),
            render: p.render,
            origin: Some(Origin::Interactive),
        })
        .await
    }

    #[tool(
        description = "Publish a data point that widgets can bind to. Free — this does not \
                       write to the display by itself."
    )]
    async fn push_data_point(&self, Parameters(p): Parameters<PushDataPointParams>) -> String {
        let value = match (p.number, p.text) {
            (Some(n), _) => Value::Number(n),
            (None, Some(t)) => Value::Text(t),
            (None, None) => return json_error("provide either `number` or `text`"),
        };
        self.call(Request::PushDataPoint {
            point: Box::new(DataPoint {
                source: p.source,
                key: p.key,
                value,
                unit: p.unit,
                label: None,
                observed_at: chrono::Utc::now(),
                ttl_secs: Some(p.ttl_secs),
            }),
        })
        .await
    }

    #[tool(
        description = "Compose the active layout and optionally display it. With \
                       execute=false it reports what the governor would decide, spending \
                       nothing. A refusal returns uploaded=false with a reason and \
                       retry_after_secs — batch your changes rather than retrying."
    )]
    async fn render_screen(&self, Parameters(p): Parameters<RenderParams>) -> String {
        self.call(Request::Render {
            execute: p.execute,
            origin: Some(Origin::Interactive),
        })
        .await
    }

    #[tool(
        description = "Display an image or animated GIF from a file path. Scaled to fit \
                       160x96. Subject to the same write governor as everything else."
    )]
    async fn show_image(&self, Parameters(p): Parameters<ShowImageParams>) -> String {
        self.call(Request::ShowImage {
            path: p.path,
            execute: p.execute,
            max_frames: p.max_frames,
            origin: Some(Origin::Interactive),
        })
        .await
    }

    #[tool(
        description = "Clear the screen to its background. Note: the keyboard has no \
                       hardware clear — this uploads a blank image and costs a flash write \
                       like any other change."
    )]
    async fn clear_screen(&self) -> String {
        // Blank every slot, then draw. Modelled as an ordinary write because that is
        // exactly what it is on this hardware.
        let layout = match self.call(Request::GetLayout).await {
            s if s.contains("\"layout\"") => s,
            other => return other,
        };
        let parsed: serde_json::Value = match serde_json::from_str(&layout) {
            Ok(v) => v,
            Err(e) => return json_error(&e.to_string()),
        };
        let mut l: Layout = match serde_json::from_value(parsed["layout"].clone()) {
            Ok(v) => v,
            Err(e) => return json_error(&e.to_string()),
        };
        for slot in &mut l.slots {
            slot.widget = None;
        }
        self.call(Request::SetLayout {
            layout: Box::new(l),
            render: true,
            origin: Some(Origin::Interactive),
        })
        .await
    }

    #[tool(
        description = "Set the keyboard's real-time clock from this computer. Uses the \
                       config channel, so it does not cost a flash write."
    )]
    async fn sync_clock(&self) -> String {
        self.call(Request::SyncClock).await
    }

    #[tool(description = "Registered data sources and when each last ran.")]
    async fn list_sources(&self) -> String {
        self.call(Request::ListSources).await
    }

    #[tool(
        description = "Run one data source now and merge its readings. Does not write to the display."
    )]
    async fn run_source(&self, Parameters(p): Parameters<RunSourceParams>) -> String {
        self.call(Request::RunSource { id: p.id }).await
    }
}

/// Server-level instructions, read once before any tool is chosen.
///
/// This is the highest-leverage place to shape agent behaviour, so it is where the
/// hardware's one irreversible property belongs: a finite, unreplaceable write budget.
#[tool_handler(
    name = "catbus99",
    version = "0.1.0",
    instructions = "Controls the 160x96 screen on an Epomaker TH99 Pro keyboard.

IMPORTANT - the display's flash is rated for roughly 100,000 writes and cannot be replaced. Every visible change spends one. Work accordingly:

1. Iterate with `preview_screen`. It renders a PNG and costs nothing.
2. Change state with `set_widget` / `set_layout` / `push_data_point` using render=false. These do not write to the panel.
3. Commit once with `render_screen` execute=true when the result looks right.

Writes are rate limited. A refusal comes back as uploaded=false with a reason and retry_after_secs - batch your changes and wait rather than retrying. An identical image is skipped for free, so re-rendering unchanged content is harmless.

The screen is 160x96 pixels and 16-bit colour: prefer a few large, high-contrast elements over many small ones. Round values coarsely (a clock to 15 minutes, a bar to 5% steps) - finer precision changes the image more often and spends the budget faster for a difference nobody can see at this size.

There is no hardware clear: `clear_screen` uploads a blank image and costs a write like any other change."
)]
impl ServerHandler for Catbus99 {}
