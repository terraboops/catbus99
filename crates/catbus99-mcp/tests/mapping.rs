//! The JSON an agent actually sees.
//!
//! These fields are the agent-facing contract. A rename or a changed meaning would not
//! break any Rust caller, but would silently change how a model behaves — most
//! importantly `uploaded`, which is what tells it whether the screen changed.

use catbus99_daemon::protocol::{Response, SourceStatus, StatusReport, WriteOutcome};
use catbus99_mcp::response_json;

fn parse(r: &Response) -> serde_json::Value {
    serde_json::from_str(&response_json(r)).expect("tool output must be valid JSON")
}

#[test]
fn every_response_is_valid_json_with_an_ok_flag() {
    let responses = vec![
        Response::Ok,
        Response::Error {
            message: "boom".into(),
        },
        Response::DataPoints { points: vec![] },
        Response::Sources { sources: vec![] },
        Response::Preview {
            png_base64: "AAAA".into(),
            width: 160,
            height: 96,
        },
    ];
    for r in &responses {
        let v = parse(r);
        assert!(v.get("ok").is_some(), "missing ok flag: {v}");
    }
}

#[test]
fn an_error_is_reported_as_not_ok_with_the_message() {
    let v = parse(&Response::Error {
        message: "device absent".into(),
    });
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"], "device absent");
}

/// The single most important field: an agent uses it to decide whether the screen changed.
#[test]
fn uploaded_is_surfaced_verbatim_for_both_outcomes() {
    for uploaded in [true, false] {
        let v = parse(&Response::Write(WriteOutcome {
            uploaded,
            reason: if uploaded {
                "upload".into()
            } else {
                "unchanged".into()
            },
            detail: "d".into(),
            retry_after_secs: None,
            bytes: Some(32768),
            reports: Some(8),
            uploads_used: 5,
            uploads_remaining: 99_995,
        }));
        assert_eq!(v["uploaded"], uploaded);
        // `ok` means the call succeeded; `uploaded` means the panel changed. A refused
        // write is a successful call, and conflating them would make an agent retry.
        assert_eq!(v["ok"], true);
    }
}

#[test]
fn a_refusal_carries_the_retry_hint_so_an_agent_can_wait_instead_of_retrying() {
    let v = parse(&Response::Write(WriteOutcome {
        uploaded: false,
        reason: "rate_limited".into(),
        detail: "next write in 7m 12s".into(),
        retry_after_secs: Some(432),
        bytes: Some(32768),
        reports: Some(8),
        uploads_used: 5,
        uploads_remaining: 99_995,
    }));
    assert_eq!(v["uploaded"], false);
    assert_eq!(v["reason"], "rate_limited");
    assert_eq!(v["retry_after_secs"], 432);
    assert!(v["detail"].as_str().unwrap().contains("7m"));
}

#[test]
fn the_remaining_budget_is_always_visible_on_a_write() {
    let v = parse(&Response::Write(WriteOutcome {
        uploaded: true,
        reason: "upload".into(),
        detail: "upload".into(),
        retry_after_secs: None,
        bytes: Some(32768),
        reports: Some(8),
        uploads_used: 42,
        uploads_remaining: 99_958,
    }));
    assert_eq!(v["uploads_used"], 42);
    assert_eq!(v["uploads_remaining"], 99_958);
}

/// The note is how an agent learns previewing is free; without it the tool looks like any
/// other and gets used sparingly.
#[test]
fn a_preview_says_it_cost_nothing() {
    let v = parse(&Response::Preview {
        png_base64: "QUJD".into(),
        width: 160,
        height: 96,
    });
    assert_eq!(v["width"], 160);
    assert_eq!(v["height"], 96);
    assert_eq!(v["png_base64"], "QUJD");
    let note = v["note"].as_str().unwrap().to_lowercase();
    assert!(note.contains("no flash write"), "note was {note:?}");
}

#[test]
fn status_and_sources_are_passed_through_structurally() {
    let v = parse(&Response::Status(Box::new(StatusReport {
        version: "0.1.0".into(),
        layout_id: "demo".into(),
        slots: 7,
        data_points: 4,
        sources: 1,
        device_present: true,
        uploads_used: 20,
        last_upload_at: None,
    })));
    assert_eq!(v["status"]["layout_id"], "demo");
    assert_eq!(v["status"]["device_present"], true);

    let v = parse(&Response::Sources {
        sources: vec![SourceStatus {
            id: "demo".into(),
            schedule: "0 */1 * * * *".into(),
            last_run_at: None,
            last_ok: Some(false),
            last_error: Some("exit 3".into()),
            points_produced: 0,
        }],
    });
    assert_eq!(v["sources"][0]["id"], "demo");
    // A failing source must surface its error, or a user cannot tell why the screen stalled.
    assert_eq!(v["sources"][0]["last_error"], "exit 3");
}
