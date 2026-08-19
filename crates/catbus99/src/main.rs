//! catbus99 -- control plane for the Epomaker TH99 Pro keyboard screen.

use anyhow::{bail, Result};
use catbus99_daemon::protocol::{default_socket_path, Origin, Request, Response};
use catbus99_daemon::server;
use catbus99_daemon::{Daemon, Paths};
use catbus99_device::{
    default_state_path, probe, Decision, Device, Governor, GovernorConfig, Interface, Lane,
};
use catbus99_model::{
    Align, BarStyle, Binding, Color, DataPoint, DataStore, Layout, Rect, TextSize, TimerFormat,
    Value, Widget,
};
use catbus99_proto::clock::{build_set_clock, is_clock_ack, ClockTime, CONFIG_PACKET_SIZE};
use catbus99_proto::container::{build_container, solid_frame, test_pattern};
use catbus99_proto::wear;
use catbus99_render::{
    compose, frames_to_container, load_frames, rgb565_to_rgba, timing_plan, to_rgb565, Fit,
};
use chrono::{Duration as ChronoDuration, Utc};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "catbus99", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Enumerate HID interfaces and report what was found. Opens nothing.
    Probe {
        /// Emit machine-readable JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Run the MCP server on stdio, so agents can drive the screen.
    Mcp {
        /// Override the daemon control socket.
        #[arg(long)]
        socket: Option<PathBuf>,
        /// Print the client configuration snippet and exit.
        #[arg(long)]
        install: bool,
    },
    /// Read the keyboard's keymap and print or save it. Read-only.
    Keymap {
        /// Read the Fn layer instead of the base layer.
        #[arg(long)]
        fn_layer: bool,
        /// Write the decoded keymap to a JSON file.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run the daemon: owns the display, runs data sources, serves the control socket.
    Daemon {
        /// Override the control socket path.
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Ask the running daemon for its status.
    Status,
    /// List registered data sources and their last run.
    Sources,
    /// Run one data source now and merge its readings.
    RunSource {
        id: String,
        /// Re-render and push the screen afterwards.
        #[arg(long)]
        render: bool,
    },
    /// Push a data point into the running daemon.
    Set {
        /// Source name.
        source: String,
        /// Data point key.
        key: String,
        /// Numeric value, or text if it does not parse as a number.
        value: String,
        #[arg(long)]
        unit: Option<String>,
        #[arg(long, default_value_t = 900)]
        ttl_secs: u64,
        /// Re-render and push the screen afterwards.
        #[arg(long)]
        render: bool,
    },
    /// Ask the daemon to compose and display the active layout.
    Render {
        /// Actually write to the panel. Without this, reports what would happen.
        #[arg(long)]
        execute: bool,
    },
    /// Show the flash-endurance odometer and budget table.
    Wear {
        /// Add this many uploads to the recorded total, for writes made before the
        /// odometer existed or by Epomaker's own driver. The count is a safety estimate,
        /// so it should err high rather than low.
        #[arg(long)]
        seed: Option<u64>,
    },
    /// Upload a two-frame black/white test pattern to verify the display path.
    ///
    /// This is the Phase 0 hardware gate. It costs one flash write, and there is no
    /// command to undo it -- the screen keeps the pattern until something else is
    /// uploaded or the keyboard is power-cycled.
    /// Render an image or animated GIF and push it to the screen.
    Image {
        /// Image or GIF to display.
        path: PathBuf,
        /// Actually write to the display.
        #[arg(long)]
        execute: bool,
        /// Write a PNG of what would be displayed (first frame).
        #[arg(long)]
        preview: Option<PathBuf>,
        /// How to fit the source into 160x96.
        #[arg(long, default_value = "contain")]
        fit: String,
        /// Maximum animation frames (each costs 30,720 bytes of upload).
        #[arg(long, default_value_t = 16)]
        max_frames: usize,
        /// Disable ordered dithering.
        #[arg(long)]
        no_dither: bool,
        /// Hold the first frame for this many ms. Use for blink/idle animations whose
        /// source gives every frame equal time, which reads as a flicker.
        #[arg(long)]
        hold_first: Option<u16>,
    },
    /// Render a layout (TOML or JSON) to the screen.
    Layout {
        /// Layout file. Omit to use the built-in demo layout.
        path: Option<PathBuf>,
        /// Actually write to the display.
        #[arg(long)]
        execute: bool,
        /// Write a PNG of the composed screen.
        #[arg(long)]
        preview: Option<PathBuf>,
        /// Print the layout as JSON and exit.
        #[arg(long)]
        dump: bool,
        /// Print the JSON Schema for a layout and exit.
        #[arg(long)]
        schema: bool,
    },
    /// Set the keyboard's real-time clock from this computer (MI_02, not a flash write).
    ClockSync {
        /// Actually send the command.
        #[arg(long)]
        execute: bool,
    },
    Selftest {
        /// Actually write to the display. Without this, only validates offline.
        #[arg(long)]
        execute: bool,
        /// Per-report acknowledgement timeout, in milliseconds.
        #[arg(long, default_value_t = 5000)]
        timeout_ms: u64,
        /// Use the static colour-band diagnostic pattern (two identical frames, like the
        /// official driver) instead of the black/white strobe.
        #[arg(long)]
        pattern: bool,
        /// Bytes reserved before frame data. 256 matches the documented container;
        /// 4096 tests the alternative layout implied by upstream's make_test_payload.
        #[arg(long, default_value_t = 256)]
        preamble_bytes: usize,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Probe { json } => cmd_probe(json),
        Command::Mcp { socket, install } => cmd_mcp(socket, install),
        Command::Keymap { fn_layer, out } => cmd_keymap(fn_layer, out),
        Command::Daemon { socket } => cmd_daemon(socket),
        Command::Status => client(Request::Status),
        Command::Sources => client(Request::ListSources),
        Command::RunSource { id, render } => {
            client(Request::RunSource { id })?;
            if render {
                client(Request::Render {
                    execute: true,
                    origin: Some(Origin::Interactive),
                })?;
            }
            Ok(())
        }
        Command::Set {
            source,
            key,
            value,
            unit,
            ttl_secs,
            render,
        } => cmd_set(source, key, value, unit, ttl_secs, render),
        Command::Render { execute } => client(Request::Render {
            execute,
            origin: Some(Origin::Interactive),
        }),
        Command::Wear { seed } => cmd_wear(seed),
        Command::Image {
            path,
            execute,
            preview,
            fit,
            max_frames,
            no_dither,
            hold_first,
        } => cmd_image(
            path, execute, preview, &fit, max_frames, !no_dither, hold_first,
        ),
        Command::Layout {
            path,
            execute,
            preview,
            dump,
            schema,
        } => cmd_layout(path, execute, preview, dump, schema),
        Command::ClockSync { execute } => cmd_clock_sync(execute),
        Command::Selftest {
            execute,
            timeout_ms,
            pattern,
            preamble_bytes,
        } => cmd_selftest(execute, timeout_ms, pattern, preamble_bytes),
    }
}

fn cmd_selftest(
    execute: bool,
    timeout_ms: u64,
    pattern: bool,
    preamble_bytes: usize,
) -> Result<()> {
    // Two IDENTICAL frames is what the official driver sends for a still image. Two
    // DIFFERENT frames at 50ms is a 20Hz strobe, not a legible still.
    let (a, b, delay) = if pattern {
        let p = test_pattern();
        (p.clone(), p, 0x19u8)
    } else {
        (solid_frame(0x0000), solid_frame(0xFFFF), 0x32u8)
    };
    let (black, white) = (a, b);
    let payload = if preamble_bytes == 256 {
        build_container(&[&black, &white], &[delay])?
    } else {
        // Alternative layout: same metadata fields, but frame data starts at
        // `preamble_bytes` instead of 256. Tests upstream's contradictory preamble size.
        let mut p = vec![0u8; preamble_bytes];
        p[..256].fill(0xFF);
        p[0] = 2;
        p[1] = delay;
        p[2] = 0x00;
        p.extend_from_slice(&black);
        p.extend_from_slice(&white);
        let pad = p.len().div_ceil(4096) * 4096 - p.len();
        p.extend(std::iter::repeat_n(0u8, pad));
        p
    };
    let reports = payload.len() / catbus99_proto::container::BLOCK_SIZE;

    println!("catbus99 selftest -- two-frame black/white pattern");
    println!("  container: {} bytes, {} reports", payload.len(), reports);
    println!("  prefix:    {:02x?}", &payload[..4]);
    println!("  frames at: offset {preamble_bytes}");
    println!(
        "  content:   {}",
        if pattern {
            "static colour-band pattern (2 identical frames, 25ms)"
        } else {
            "black/white STROBE (2 different frames, 50ms)"
        }
    );

    if !execute {
        println!();
        println!("  Validated offline. Nothing was sent, no HID handle was opened.");
        println!("  Re-run with --execute to write it to the display.");
        return Ok(());
    }

    if !upload_governed(&payload, Lane::Interactive, timeout_ms)? {
        return Ok(());
    }
    let _ = reports;

    println!();
    println!("  LOOK AT THE KEYBOARD SCREEN NOW.");
    if pattern {
        println!("  Expected: 4 horizontal bands red/green/blue/white top to bottom,");
        println!("            black square in the TOP-LEFT, thin white border.");
    } else {
        println!("  Expected: a 20Hz black/white STROBE (two differing frames at 50ms).");
        println!("            Use --pattern for a legible still image instead.");
    }
    println!("  There is no read-back command -- your eyes are the only ground truth.");
    println!("  Unchanged screen + 16/16 ACKs is a FAILURE, not a pass.");
    Ok(())
}

fn cmd_probe(json: bool) -> Result<()> {
    let report = probe()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("catbus99 probe");
    println!(
        "  looking for {:04x}:{:04x} among {} HID device(s)",
        report.target_vid, report.target_pid, report.total_hid_devices
    );
    println!();

    if report.matching_interfaces.is_empty() {
        println!("  no matching interfaces");
    } else {
        for iface in &report.matching_interfaces {
            println!(
                "  iface {:>2}  usage_page 0x{:04x}  usage 0x{:04x}  {}",
                iface.interface_number,
                iface.usage_page,
                iface.usage,
                iface.product.as_deref().unwrap_or("<no product string>")
            );
            println!("            path: {}", iface.path);
        }
    }

    println!();
    println!(
        "  config (MI_02) candidates: {}",
        report.config_candidates.len()
    );
    println!(
        "  TFT    (MI_03) candidates: {}",
        report.tft_candidates.len()
    );
    println!();
    println!("  {}", report.verdict);

    Ok(())
}

fn cmd_wear(seed: Option<u64>) -> Result<()> {
    let path = default_state_path();
    let mut governor = Governor::load(GovernorConfig::default(), &path)?;
    if let Some(n) = seed {
        governor.state.total_uploads += n;
        *governor
            .state
            .uploads_by_lane
            .entry("pre-odometer".into())
            .or_insert(0) += n;
        governor.save()?;
        println!("Added {n} uploads to the recorded total.\n");
    }
    let r = governor.report();

    println!("catbus99 wear odometer");
    println!("  state file: {}", path.display());
    println!();
    println!("  uploads so far:   {}", r.total_uploads);
    println!("  bytes written:    {}", r.total_bytes);
    println!(
        "  budget used:      {:.4}% of {} rated cycles",
        r.budget_used_fraction * 100.0,
        wear::RATED_PE_CYCLES
    );
    println!("  uploads left:     {}", r.uploads_remaining);
    match r.last_upload_at {
        Some(t) => println!("  last upload:      {}", t.to_rfc3339()),
        None => println!("  last upload:      never"),
    }
    if !governor.state.uploads_by_lane.is_empty() {
        println!("  by lane:          {:?}", governor.state.uploads_by_lane);
    }
    println!(
        "  write interval:   {}s (hard floor {}s)",
        r.interval_secs,
        wear::MIN_WRITE_INTERVAL_SECS
    );
    println!();
    println!("Projected lifetime by write interval, worst case:");
    println!();
    println!(
        "  {:<22} {:>12} {:>14}",
        "write interval", "uploads/day", "years to limit"
    );
    for secs in [300u64, 600, 900, 1800, 3600] {
        let label = if secs == wear::DEFAULT_WRITE_INTERVAL_SECS {
            format!("{} min (default)", secs / 60)
        } else if secs == wear::MIN_WRITE_INTERVAL_SECS {
            format!("{} min (floor)", secs / 60)
        } else {
            format!("{} min", secs / 60)
        };
        println!(
            "  {:<22} {:>12.0} {:>14.1}",
            label,
            wear::uploads_per_day(secs),
            wear::projected_years(secs)
        );
    }
    println!();
    println!("  Worst case: assumes the rendered image changes every interval.");
    println!("  Identical images are never re-uploaded, so real endurance is higher.");
    Ok(())
}

fn cmd_clock_sync(execute: bool) -> Result<()> {
    use chrono::{Datelike, Local, Timelike};

    let now = Local::now();
    let when = ClockTime::new(
        now.year() as u16,
        now.month() as u8,
        now.day() as u8,
        now.hour() as u8,
        now.minute() as u8,
        now.second() as u8,
    )?;
    let packet = build_set_clock(when)?;

    println!("catbus99 clock sync (MI_02 config channel -- no flash write)");
    println!("  setting: {when:?}");
    println!("  iso weekday: {}", when.iso_weekday());
    println!(
        "  packet: {}",
        packet[..18]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    if !execute {
        println!("\n  Validated offline. Re-run with --execute to send.");
        return Ok(());
    }

    let device = Device::open(Interface::Config)?;
    device.write_report(&packet)?;
    let response = device.read_report(CONFIG_PACKET_SIZE, Duration::from_millis(5000))?;

    if response.is_empty() {
        bail!("no acknowledgement from the config interface");
    }
    println!(
        "\n  reply:  {}",
        response[..18.min(response.len())]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    if is_clock_ack(&packet, &response) {
        println!("\n  ACK verified: request echoed back with AA -> 55.");
        println!("  The MI_02 config channel works and the keyboard accepted the command.");
        println!("\n  Check the keyboard's native clock display.");
    } else {
        println!("\n  Reply did not match the expected echo. Recording it is still useful.");
    }
    Ok(())
}

fn cmd_image(
    path: PathBuf,
    execute: bool,
    preview: Option<PathBuf>,
    fit: &str,
    max_frames: usize,
    dither: bool,
    hold_first: Option<u16>,
) -> Result<()> {
    let mode = match fit {
        "contain" => Fit::Contain,
        "cover" => Fit::Cover,
        "stretch" => Fit::Stretch,
        other => bail!("unknown fit mode {other:?} (contain|cover|stretch)"),
    };

    let mut frames = load_frames(&path, mode, dither)?;
    if let Some(ms) = hold_first {
        if let Some(f) = frames.first_mut() {
            f.duration_ms = ms;
        }
    }
    let (total_ms, tick, planned) = timing_plan(&frames, max_frames);
    let payload = frames_to_container(&frames, max_frames)?;
    let reports = payload.len() / catbus99_proto::container::BLOCK_SIZE;
    let shown = payload[0];

    println!("catbus99 image -- {}", path.display());
    println!("  source frames: {} ({} ms/loop)", frames.len(), total_ms);
    println!("  fit={fit}, dither={dither}");
    if frames.len() > 1 {
        println!("  timing:        tick {tick} ms -> {planned} frames after duplication");
    }
    println!(
        "  container:     {} bytes, {reports} reports, {shown} frame(s)",
        payload.len()
    );

    if let Some(out) = &preview {
        let frame0 = &payload[256..256 + catbus99_proto::container::FRAME_BYTES];
        rgb565_to_rgba(frame0).save(out)?;
        println!("  preview:       {}", out.display());
    }

    if !execute {
        println!();
        println!("  Rendered only. Nothing was sent. Use --execute to display it.");
        return Ok(());
    }

    if upload_governed(&payload, Lane::Interactive, 5000)? {
        println!("  Look at the keyboard screen.");
    }
    Ok(())
}

/// Every path that writes pixels goes through here.
///
/// The governor is the only thing bounding flash wear, and it is only sound if nothing
/// can route around it -- so there is deliberately no `--force`. A refusal explains
/// itself and says when the write will be allowed.
fn upload_governed(payload: &[u8], lane: Lane, timeout_ms: u64) -> Result<bool> {
    if daemon_is_running() {
        bail!(
            "the catbus99 daemon is running and owns the display.\n\
             Two writers would each keep their own copy of the wear state, so the counts \n\
             would be wrong and neither would know what the panel is showing.\n\
             Use `catbus99 render --execute`, or stop the daemon first."
        );
    }
    let path = default_state_path();
    let mut governor = Governor::load(GovernorConfig::default(), &path)?;
    let now = Utc::now();

    let decision = governor.decide(payload, lane, now);
    if !decision.will_upload() {
        println!();
        println!("  NOT WRITTEN -- {}", decision.reason());
        if let Decision::SkipUnchanged = decision {
            println!("  (this costs nothing; the panel already shows these exact pixels)");
        }
        return Ok(false);
    }

    println!();
    println!(
        "  WRITING TO FLASH ({} bytes, {} reports)",
        payload.len(),
        payload.len() / catbus99_proto::container::BLOCK_SIZE
    );
    let outcome =
        governor.upload_to_panel(payload, lane, now, Duration::from_millis(timeout_ms))?;
    if let Some(err) = outcome.device_error {
        bail!("panel write failed: {err}");
    }

    let report = governor.report();
    println!("  transfer acknowledged.");
    println!(
        "  wear: {} uploads used ({:.3}% of the rated budget), {} remaining",
        report.total_uploads,
        report.budget_used_fraction * 100.0,
        report.uploads_remaining
    );
    Ok(true)
}

/// A layout exercising every widget, used as the visual regression baseline.
fn demo_layout() -> Layout {
    Layout {
        id: "demo".into(),
        name: "catbus99 demo".into(),
        background: Color::new(0x0A, 0x0C, 0x10),
        slots: vec![],
    }
    .with_slot(
        "title",
        Rect::new(2, 1, 96, 8),
        Widget::Label {
            text: Binding::literal_text("CATBUS99"),
            size: TextSize::Small,
            align: Align::Left,
            color: Color::new(0x4A, 0xC8, 0xFF),
        },
    )
    .with_slot(
        "clock",
        Rect::new(100, 1, 58, 8),
        Widget::Clock {
            format: "%H:%M".into(),
            tz: None,
            quantize_minutes: 15,
            size: TextSize::Small,
            align: Align::Right,
            color: Color::WHITE,
        },
    )
    .with_slot(
        "session",
        Rect::new(2, 12, 156, 17),
        Widget::ProgressBar {
            value: Binding::DataPoint {
                source: "demo".into(),
                key: "session".into(),
                scale: None,
            },
            label: Some(Binding::literal_text("SESSION")),
            style: BarStyle::Solid,
            color: Color::new(0x4A, 0xC8, 0xFF),
            track: Color::new(0x1A, 0x20, 0x28),
            show_value: true,
        },
    )
    .with_slot(
        "weekly",
        Rect::new(2, 31, 156, 17),
        Widget::ProgressBar {
            value: Binding::DataPoint {
                source: "demo".into(),
                key: "weekly".into(),
                scale: None,
            },
            label: Some(Binding::literal_text("WEEKLY")),
            style: BarStyle::Segmented,
            color: Color::new(0xFF, 0xA5, 0x3A),
            track: Color::new(0x1A, 0x20, 0x28),
            show_value: false,
        },
    )
    .with_slot(
        "gauge",
        Rect::new(2, 51, 46, 44),
        Widget::Gauge {
            value: Binding::DataPoint {
                source: "demo".into(),
                key: "cpu".into(),
                scale: None,
            },
            min: 0.0,
            max: 100.0,
            unit: Some("%".into()),
            color: Color::new(0x6E, 0xE7, 0xB7),
            track: Color::new(0x1A, 0x20, 0x28),
        },
    )
    .with_slot(
        "timer",
        Rect::new(52, 51, 54, 44),
        Widget::ResetTimer {
            deadline: Binding::DataPoint {
                source: "demo".into(),
                key: "resets_at".into(),
                scale: None,
            },
            label: Some("RESETS".into()),
            format: TimerFormat::Compact,
            quantize_minutes: 15,
            color: Color::WHITE,
        },
    )
    .with_slot(
        "spark",
        Rect::new(110, 51, 48, 44),
        Widget::Sparkline {
            points: vec![
                3.0, 5.0, 4.0, 8.0, 6.0, 9.0, 7.0, 12.0, 10.0, 14.0, 11.0, 15.0,
            ],
            color: Color::new(0xC0, 0x8C, 0xFF),
        },
    )
}

fn demo_store() -> DataStore {
    let now = Utc::now();
    let mut store = DataStore::new();
    for (key, value) in [
        ("session", Value::Number(0.62)),
        ("weekly", Value::Number(0.35)),
        ("cpu", Value::Number(73.0)),
    ] {
        store.insert(DataPoint {
            source: "demo".into(),
            key: key.into(),
            value,
            unit: None,
            label: None,
            observed_at: now,
            ttl_secs: Some(300),
        });
    }
    store.insert(DataPoint {
        source: "demo".into(),
        key: "resets_at".into(),
        value: Value::Timestamp(now + ChronoDuration::minutes(134)),
        unit: None,
        label: None,
        observed_at: now,
        ttl_secs: Some(300),
    });
    store
}

fn cmd_layout(
    path: Option<PathBuf>,
    execute: bool,
    preview: Option<PathBuf>,
    dump: bool,
    schema: bool,
) -> Result<()> {
    if schema {
        println!(
            "{}",
            serde_json::to_string_pretty(&catbus99_model::layout_schema())?
        );
        return Ok(());
    }

    let (layout, store) = match &path {
        None => (demo_layout(), demo_store()),
        Some(p) => {
            let text = std::fs::read_to_string(p)?;
            let layout: Layout = if p.extension().is_some_and(|e| e == "json") {
                serde_json::from_str(&text)?
            } else {
                toml::from_str(&text)?
            };
            (layout, DataStore::new())
        }
    };

    if dump {
        println!("{}", serde_json::to_string_pretty(&layout)?);
        return Ok(());
    }

    println!("catbus99 layout -- {} ({})", layout.id, layout.name);
    println!("  slots: {}", layout.slots.len());
    for problem in layout.lint() {
        println!("  LINT: {problem}");
    }
    if store.is_empty() && path.is_some() {
        println!("  note: no data points bound; live widgets will render as --");
    }

    let img = compose(&layout, &store, Utc::now());
    let frame = to_rgb565(&img, false);

    if let Some(out) = &preview {
        rgb565_to_rgba(&frame).save(out)?;
        println!("  preview: {}", out.display());
    }

    if !execute {
        println!();
        println!("  Composed only. Nothing was sent. Use --execute to display it.");
        return Ok(());
    }

    let payload = catbus99_proto::container::build_container(&[&frame], &[])?;
    if upload_governed(&payload, Lane::Interactive, 5000)? {
        println!("  Look at the keyboard screen.");
    }
    Ok(())
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
}

fn cmd_daemon(socket: Option<PathBuf>) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "catbus99=info,catbus99_daemon=info".into()),
        )
        .init();

    let mut paths = Paths::default();
    if let Some(s) = socket {
        paths.socket = s;
    }
    // Start from a saved layout if there is one, otherwise the demo.
    let layout = std::fs::read_to_string(&paths.layout)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(demo_layout);

    println!("catbus99 daemon");
    println!("  socket:  {}", paths.socket.display());
    println!("  sources: {}", paths.sources.display());
    println!("  wear:    {}", paths.wear_state.display());
    println!("  layout:  {} ({} slots)", layout.id, layout.slots.len());
    println!();

    let daemon = Daemon::new(layout, paths.clone())
        .map_err(|e| anyhow::anyhow!("could not start daemon: {e}"))?;
    let server = server::Server::new(daemon).with_socket(paths.socket);
    runtime()?.block_on(server.run())?;
    Ok(())
}

/// True when a daemon is listening on the default socket.
///
/// Matters because the daemon and a direct CLI write would each hold their own copy of the
/// wear state: the second to save would clobber the first's counts, and neither would know
/// what the panel is really showing. One owner or the other, never both.
fn daemon_is_running() -> bool {
    let socket = default_socket_path();
    if !socket.exists() {
        return false;
    }
    std::os::unix::net::UnixStream::connect(&socket).is_ok()
}

fn client(request: Request) -> Result<()> {
    let response = runtime()?.block_on(server::request_default(&request))?;
    print_response(&response);
    match response {
        Response::Error { message } => bail!("{message}"),
        _ => Ok(()),
    }
}

fn cmd_set(
    source: String,
    key: String,
    value: String,
    unit: Option<String>,
    ttl_secs: u64,
    render: bool,
) -> Result<()> {
    let parsed = match value.parse::<f64>() {
        Ok(n) => catbus99_model::Value::Number(n),
        Err(_) => catbus99_model::Value::Text(value),
    };
    let point = DataPoint {
        source,
        key,
        value: parsed,
        unit,
        label: None,
        observed_at: Utc::now(),
        ttl_secs: Some(ttl_secs),
    };
    client(Request::PushDataPoint {
        point: Box::new(point),
    })?;
    if render {
        client(Request::Render {
            execute: true,
            origin: Some(Origin::Interactive),
        })?;
    }
    Ok(())
}

fn print_response(response: &Response) {
    match response {
        Response::Ok => println!("ok"),
        Response::Error { message } => println!("error: {message}"),
        Response::Status(s) => {
            println!("catbus99 daemon v{}", s.version);
            println!("  layout:       {} ({} slots)", s.layout_id, s.slots);
            println!("  data points:  {}", s.data_points);
            println!("  sources:      {}", s.sources);
            println!(
                "  device:       {}",
                if s.device_present {
                    "connected"
                } else {
                    "absent"
                }
            );
            println!("  uploads used: {}", s.uploads_used);
            match s.last_upload_at {
                Some(t) => println!("  last upload:  {}", t.to_rfc3339()),
                None => println!("  last upload:  never"),
            }
        }
        Response::Wear(w) => {
            println!(
                "uploads {} ({:.4}% of budget), {} remaining",
                w.total_uploads,
                w.budget_used_fraction * 100.0,
                w.uploads_remaining
            );
        }
        Response::Layout(l) => {
            println!("{}", serde_json::to_string_pretty(l).unwrap_or_default());
        }
        Response::DataPoints { points } => {
            for p in points {
                println!(
                    "  {}.{} = {}{}",
                    p.source,
                    p.key,
                    p.value.as_display(),
                    p.unit.as_deref().unwrap_or("")
                );
            }
        }
        Response::Sources { sources } => {
            if sources.is_empty() {
                println!("no sources registered");
            }
            for s in sources {
                let state = match (s.last_ok, &s.last_error) {
                    (Some(true), _) => format!("ok, {} points", s.points_produced),
                    (Some(false), Some(e)) => format!("FAILED: {e}"),
                    _ => "not run yet".into(),
                };
                println!("  {:<20} {:<18} {}", s.id, s.schedule, state);
            }
        }
        Response::Preview {
            width,
            height,
            png_base64,
        } => {
            println!(
                "preview {width}x{height}, {} base64 chars",
                png_base64.len()
            );
        }
        Response::Write(w) => {
            if w.uploaded {
                println!(
                    "written: {} bytes, {} reports",
                    w.bytes.unwrap_or(0),
                    w.reports.unwrap_or(0)
                );
            } else if w.reason == "would_upload" {
                println!(
                    "not written (dry run): {} bytes, {} reports would be sent",
                    w.bytes.unwrap_or(0),
                    w.reports.unwrap_or(0)
                );
            } else {
                println!("not written -- {}", w.detail);
            }
            println!(
                "  uploads used {} / remaining {}",
                w.uploads_used, w.uploads_remaining
            );
        }
    }
}

fn cmd_keymap(fn_layer: bool, out: Option<PathBuf>) -> Result<()> {
    use catbus99_proto::keymap::{
        decode_table, CMD_READ_BASIC, CMD_READ_FN, MATRIX_COLS, MATRIX_ROWS,
    };

    let command = if fn_layer {
        CMD_READ_FN
    } else {
        CMD_READ_BASIC
    };
    let layer = if fn_layer { "fn" } else { "base" };

    let device = Device::open(Interface::Config)?;
    let raw = device.read_keymap(command, Duration::from_millis(5000))?;
    let table = decode_table(&raw)?;

    println!(
        "catbus99 keymap -- {layer} layer ({} keys, {MATRIX_COLS}x{MATRIX_ROWS} matrix)",
        table.len()
    );
    println!();
    for row in 0..MATRIX_ROWS {
        let cells: Vec<String> = table[row * MATRIX_COLS..(row + 1) * MATRIX_COLS]
            .iter()
            .map(|b| format!("{:<10}", b.name()))
            .collect();
        println!("  r{row}  {}", cells.join(""));
    }

    if let Some(path) = out {
        let json = serde_json::json!({
            "layer": layer,
            "matrix": { "cols": MATRIX_COLS, "rows": MATRIX_ROWS },
            "keys": table.iter().map(|b| b.name()).collect::<Vec<_>>(),
            "raw_hex": raw.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(""),
        });
        std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;
        println!();
        println!("  saved to {}", path.display());
        println!("  (raw bytes are included so a restore can be byte-exact)");
    }
    Ok(())
}

fn cmd_mcp(socket: Option<PathBuf>, install: bool) -> Result<()> {
    if install {
        let exe = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "catbus99".into());
        println!("Register catbus99 with Claude Code:");
        println!();
        println!("  claude mcp add catbus99 -- {exe} mcp");
        println!();
        println!("Or add to your MCP client config:");
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "mcpServers": {
                    "catbus99": { "command": exe, "args": ["mcp"] }
                }
            }))?
        );
        println!();
        println!("The daemon must be running: `catbus99 daemon`.");
        return Ok(());
    }

    // stdout is the MCP transport, so diagnostics must go to stderr or they corrupt the
    // protocol stream.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();

    runtime()?.block_on(async move {
        use rmcp::ServiceExt;
        let service = catbus99_mcp::Catbus99::new(socket)
            .serve(rmcp::transport::stdio())
            .await?;
        service.waiting().await?;
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}
