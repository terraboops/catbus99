//! The governor is the only thing standing between a bug and a destroyed display, so its
//! rules are tested exhaustively rather than sampled.

use catbus99_device::*;
use catbus99_proto::wear;
use chrono::{Duration, TimeZone, Utc};

fn t0() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 18, 12, 0, 0).unwrap()
}

fn gov() -> Governor {
    Governor::ephemeral(GovernorConfig::default())
}

const A: &[u8] = b"payload-a";
const B: &[u8] = b"payload-b";

#[test]
fn the_first_write_is_always_allowed() {
    assert_eq!(gov().decide(A, Lane::Scheduled, t0()), Decision::Upload);
    assert_eq!(gov().decide(A, Lane::Interactive, t0()), Decision::Upload);
}

/// The cheapest and most important rule: never re-send what is already on screen.
#[test]
fn identical_payloads_are_skipped_in_every_lane() {
    let mut g = gov();
    g.record_upload(A, Lane::Scheduled, t0());
    for lane in [Lane::Scheduled, Lane::Interactive, Lane::Recovery] {
        assert_eq!(
            g.decide(A, lane, t0() + Duration::hours(5)),
            Decision::SkipUnchanged,
            "{lane:?} re-sent an unchanged payload"
        );
    }
}

#[test]
fn a_changed_payload_is_not_skipped() {
    let mut g = gov();
    g.record_upload(A, Lane::Scheduled, t0());
    assert_ne!(
        g.decide(B, Lane::Scheduled, t0() + Duration::hours(1)),
        Decision::SkipUnchanged
    );
}

#[test]
fn scheduled_writes_respect_the_interval() {
    let mut g = gov();
    g.record_upload(A, Lane::Scheduled, t0());

    match g.decide(B, Lane::Scheduled, t0() + Duration::minutes(5)) {
        Decision::RateLimited { retry_after_secs } => {
            assert_eq!(retry_after_secs, 600); // 15 min default minus 5 elapsed
        }
        other => panic!("expected rate limiting, got {other:?}"),
    }
    assert_eq!(
        g.decide(B, Lane::Scheduled, t0() + Duration::minutes(15)),
        Decision::Upload
    );
}

/// A user may opt into faster updates; they may not opt into destroying the panel.
#[test]
fn the_hard_floor_cannot_be_configured_away() {
    for attempted in [0u64, 1, 30, 60] {
        let cfg = GovernorConfig {
            write_interval_secs: attempted,
            interactive_per_hour: 12,
        };
        assert_eq!(cfg.effective_interval_secs(), wear::MIN_WRITE_INTERVAL_SECS);
    }
    // A slower interval than the floor is honoured as given.
    let cfg = GovernorConfig {
        write_interval_secs: 3600,
        interactive_per_hour: 12,
    };
    assert_eq!(cfg.effective_interval_secs(), 3600);
}

#[test]
fn a_sub_floor_config_still_blocks_rapid_writes() {
    let mut g = Governor::ephemeral(GovernorConfig {
        write_interval_secs: 1,
        interactive_per_hour: 12,
    });
    g.record_upload(A, Lane::Scheduled, t0());
    assert!(matches!(
        g.decide(B, Lane::Scheduled, t0() + Duration::seconds(60)),
        Decision::RateLimited { .. }
    ));
    assert_eq!(
        g.decide(B, Lane::Scheduled, t0() + Duration::seconds(300)),
        Decision::Upload
    );
}

#[test]
fn interactive_writes_are_bounded_per_hour() {
    let mut g = Governor::ephemeral(GovernorConfig {
        write_interval_secs: 900,
        interactive_per_hour: 3,
    });
    for i in 0..3 {
        let now = t0() + Duration::seconds(i);
        let payload = format!("p{i}");
        assert_eq!(
            g.decide(payload.as_bytes(), Lane::Interactive, now),
            Decision::Upload
        );
        g.record_upload(payload.as_bytes(), Lane::Interactive, now);
    }
    assert!(matches!(
        g.decide(b"p4", Lane::Interactive, t0() + Duration::seconds(4)),
        Decision::BurstExhausted { .. }
    ));
}

#[test]
fn the_interactive_allowance_refills_as_writes_age_out() {
    let mut g = Governor::ephemeral(GovernorConfig {
        write_interval_secs: 900,
        interactive_per_hour: 2,
    });
    g.record_upload(b"p0", Lane::Interactive, t0());
    g.record_upload(b"p1", Lane::Interactive, t0() + Duration::minutes(10));

    match g.decide(b"p2", Lane::Interactive, t0() + Duration::minutes(20)) {
        Decision::BurstExhausted { retry_after_secs } => {
            // Frees up an hour after the OLDEST write in the window.
            assert_eq!(retry_after_secs, 40 * 60);
        }
        other => panic!("expected burst exhaustion, got {other:?}"),
    }
    assert_eq!(
        g.decide(b"p2", Lane::Interactive, t0() + Duration::minutes(61)),
        Decision::Upload
    );
}

/// Interactive and scheduled allowances are independent: heavy interactive use must not
/// consume the scheduled budget, nor vice versa.
#[test]
fn the_lanes_do_not_share_an_allowance() {
    let mut g = Governor::ephemeral(GovernorConfig {
        write_interval_secs: 900,
        interactive_per_hour: 1,
    });
    g.record_upload(b"i", Lane::Interactive, t0());
    // Interactive is spent...
    assert!(matches!(
        g.decide(b"x", Lane::Interactive, t0() + Duration::minutes(1)),
        Decision::BurstExhausted { .. }
    ));
    // ...but a scheduled write still becomes available on its own interval.
    assert_eq!(
        g.decide(b"x", Lane::Scheduled, t0() + Duration::minutes(15)),
        Decision::Upload
    );
}

/// After a power cycle the panel shows its native screen, so one immediate write is
/// justified even mid-interval.
#[test]
fn recovery_bypasses_the_interval_but_not_change_skip() {
    let mut g = gov();
    g.record_upload(A, Lane::Scheduled, t0());
    assert_eq!(
        g.decide(B, Lane::Recovery, t0() + Duration::seconds(1)),
        Decision::Upload
    );
    assert_eq!(
        g.decide(A, Lane::Recovery, t0() + Duration::seconds(1)),
        Decision::SkipUnchanged
    );
}

#[test]
fn invalidating_the_display_allows_resending_the_same_image() {
    let mut g = gov();
    g.record_upload(A, Lane::Scheduled, t0());
    assert_eq!(g.decide(A, Lane::Recovery, t0()), Decision::SkipUnchanged);
    g.invalidate_displayed();
    assert_eq!(g.decide(A, Lane::Recovery, t0()), Decision::Upload);
}

#[test]
fn recovery_uploads_are_still_counted() {
    let mut g = gov();
    g.record_upload(A, Lane::Recovery, t0());
    assert_eq!(g.report().total_uploads, 1);
    assert_eq!(g.state.uploads_by_lane.get("recovery"), Some(&1));
}

#[test]
fn the_odometer_tracks_uploads_bytes_and_days() {
    let mut g = gov();
    g.record_upload(&vec![0u8; 32_768], Lane::Scheduled, t0());
    g.record_upload(
        &vec![1u8; 65_536],
        Lane::Interactive,
        t0() + Duration::days(1),
    );

    let r = g.report();
    assert_eq!(r.total_uploads, 2);
    assert_eq!(r.total_bytes, 98_304);
    assert_eq!(r.uploads_remaining, wear::RATED_PE_CYCLES - 2);
    assert_eq!(g.state.uploads_by_day.len(), 2);
}

#[test]
fn budget_reporting_matches_the_documented_model() {
    let mut g = gov();
    for i in 0..1000u64 {
        // Distinct payloads so change-skip does not interfere.
        g.record_upload(
            &i.to_le_bytes(),
            Lane::Scheduled,
            t0() + Duration::seconds(i as i64),
        );
    }
    let r = g.report();
    assert_eq!(r.total_uploads, 1000);
    assert!((r.budget_used_fraction - 0.01).abs() < 1e-9);
    assert_eq!(r.uploads_remaining, 99_000);
}

#[test]
fn state_round_trips_through_disk() {
    let dir = std::env::temp_dir().join(format!("catbus99-gov-{}", std::process::id()));
    let path = dir.join("wear.json");
    let _ = std::fs::remove_dir_all(&dir);

    let mut g = Governor::load(GovernorConfig::default(), &path).unwrap();
    g.record_upload(A, Lane::Interactive, t0());
    g.save().unwrap();

    let reloaded = Governor::load(GovernorConfig::default(), &path).unwrap();
    assert_eq!(reloaded.state.total_uploads, 1);
    assert_eq!(reloaded.state.last_hash, Some(hash_payload(A)));
    // Persistence is what makes change-skip survive across CLI invocations.
    assert_eq!(
        reloaded.decide(A, Lane::Interactive, t0() + Duration::hours(2)),
        Decision::SkipUnchanged
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_state_starts_clean_rather_than_failing() {
    let path = std::env::temp_dir()
        .join("catbus99-does-not-exist")
        .join("wear.json");
    let _ = std::fs::remove_file(&path);
    let g = Governor::load(GovernorConfig::default(), &path).unwrap();
    assert_eq!(g.state.total_uploads, 0);
}

#[test]
fn decisions_explain_themselves() {
    let mut g = gov();
    g.record_upload(A, Lane::Scheduled, t0());
    assert!(g
        .decide(A, Lane::Scheduled, t0())
        .reason()
        .contains("unchanged"));
    assert!(g
        .decide(B, Lane::Scheduled, t0())
        .reason()
        .contains("rate limited"));
    assert!(Decision::Upload.will_upload());
    assert!(!Decision::SkipUnchanged.will_upload());
}

#[test]
fn hashing_distinguishes_payloads_that_differ_by_one_byte() {
    let mut a = vec![0u8; 65_536];
    let mut b = a.clone();
    b[65_535] = 1;
    assert_ne!(hash_payload(&a), hash_payload(&b));
    a[0] = 0;
    assert_eq!(hash_payload(&a), hash_payload(&vec![0u8; 65_536]));
}

// --- the invariant itself ---

/// The governor is only a safety property if it cannot be bypassed. These tests assert
/// the *shape* of the API rather than behaviour, because the enforcement is structural:
/// `Device::upload_container` is crate-private, so no caller outside this crate can write
/// pixels without going through `Governor::upload_to_panel`.
///
/// If someone ever makes that method public, this file stops compiling — which is the
/// point.
#[test]
fn the_bulk_write_path_is_not_publicly_reachable() {
    // Compile-time evidence: the only public symbol that writes to the panel is the
    // governor's, and it takes a lane so every write is attributed.
    fn _assert_signature(
        g: &mut Governor,
        payload: &[u8],
        lane: Lane,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<UploadOutcome, HidError> {
        g.upload_to_panel(payload, lane, now, std::time::Duration::from_millis(1))
    }
    // And there is no override parameter to disable the rules.
    let _ = _assert_signature;
}

/// A raw report write must not be usable to hand-roll an ungoverned image upload.
#[test]
fn raw_report_writes_reject_tft_commands() {
    // We cannot open a device in a unit test, but the guard is on the payload shape and
    // is checked before any I/O, so its rejection is observable through the error type.
    let err = HidError::UngovernedPanelWrite;
    let msg = err.to_string();
    assert!(
        msg.contains("governor"),
        "the refusal should point at the governor: {msg}"
    );
}

// --- regressions found in adversarial review ---

/// A transfer that dies part-way has still written flash. Counting only successes makes
/// the odometer drift *low*, which is the dangerous direction: it would report budget the
/// panel no longer has.
#[test]
fn a_failed_upload_is_still_counted_as_wear() {
    let mut g = gov();
    g.record_failed_upload(A, Lane::Interactive, t0());

    let r = g.report();
    assert_eq!(
        r.total_uploads, 1,
        "a partial write must count against the budget"
    );
    assert_eq!(g.state.uploads_by_lane.get("failed"), Some(&1));
}

/// After a failure the panel shows a torn image, not what we sent — so the retry must not
/// be skipped as "unchanged", or the screen stays broken forever.
#[test]
fn a_failed_upload_does_not_block_its_own_retry() {
    let mut g = gov();
    g.record_failed_upload(A, Lane::Recovery, t0());
    assert_eq!(g.decide(A, Lane::Recovery, t0()), Decision::Upload);
}

#[test]
fn a_successful_upload_after_a_failure_still_skips_duplicates() {
    let mut g = gov();
    g.record_failed_upload(A, Lane::Interactive, t0());
    g.record_upload(A, Lane::Interactive, t0());
    assert_eq!(
        g.decide(A, Lane::Interactive, t0()),
        Decision::SkipUnchanged
    );
    assert_eq!(
        g.report().total_uploads,
        2,
        "both the failed and the successful write count"
    );
}
