//! The storage-inference classifier.
//!
//! Fully offline: the input is a list of latencies, so every case a real keyboard could
//! produce can be constructed here. That matters because the verdict feeds a wear budget,
//! and a wrong verdict either wastes display resolution or destroys a panel.

use catbus99_device::{classify, StorageVerdict};
use std::time::Duration;

fn ms(values: &[f64]) -> Vec<Duration> {
    values
        .iter()
        .map(|v| Duration::from_secs_f64(v / 1000.0))
        .collect()
}

/// USB full-speed HID paces a 4104-byte report at roughly 65ms regardless of what the
/// firmware does with it, so a uniform run at bus speed means nothing stalled.
#[test]
fn a_uniform_upload_reads_as_streaming() {
    let v = classify(&ms(&[65.0, 66.0, 64.0, 65.0, 67.0, 65.0, 66.0, 64.0]));
    match v {
        StorageVerdict::StreamingLikely { median_ms, .. } => {
            assert!((median_ms - 65.0).abs() < 2.0)
        }
        other => panic!("expected streaming, got {}", other.summary()),
    }
    assert!(!v.suggests_flash());
}

/// The signature we expect if the firmware erases a block before programming it: one very
/// long report, then bus-speed for the rest.
#[test]
fn an_erase_at_the_start_reads_as_flash() {
    let v = classify(&ms(&[420.0, 65.0, 64.0, 66.0, 65.0, 65.0, 64.0, 66.0]));
    match &v {
        StorageVerdict::FlashLikely {
            stalled_reports,
            worst_ms,
            ..
        } => {
            assert_eq!(stalled_reports, &vec![0]);
            assert!(*worst_ms > 400.0);
        }
        other => panic!("expected flash, got {}", other.summary()),
    }
    assert!(v.suggests_flash());
}

/// Per-sector erase would stall periodically rather than once.
#[test]
fn periodic_stalls_read_as_flash_and_name_every_one() {
    let v = classify(&ms(&[
        200.0, 65.0, 64.0, 66.0, 200.0, 65.0, 64.0, 65.0, 210.0, 66.0, 65.0, 64.0,
    ]));
    match &v {
        StorageVerdict::FlashLikely {
            stalled_reports, ..
        } => {
            assert_eq!(stalled_reports, &vec![0, 4, 8]);
        }
        other => panic!("expected flash, got {}", other.summary()),
    }
}

/// Ordinary bus jitter must not be read as an erase. A 30% outlier is scheduling noise; an
/// erase is an order of magnitude.
#[test]
fn bus_jitter_is_not_mistaken_for_an_erase() {
    let v = classify(&ms(&[65.0, 88.0, 64.0, 71.0, 65.0, 92.0, 66.0, 64.0]));
    assert!(!v.suggests_flash(), "jitter misclassified: {}", v.summary());
}

/// A short relative spike on a very fast upload must also clear an absolute floor, or a
/// 1ms-vs-4ms blip would be called an erase.
#[test]
fn a_tiny_absolute_spike_does_not_count_even_if_relatively_large() {
    // 4x the median, but only 3ms above it: far too small to be an erase.
    let v = classify(&ms(&[1.0, 1.0, 4.0, 1.0, 1.0, 1.0, 1.0, 1.0]));
    assert!(
        !v.suggests_flash(),
        "sub-millisecond noise misclassified: {}",
        v.summary()
    );
}

/// An 8-report still is the smallest real upload; it must still be classifiable.
#[test]
fn the_smallest_real_upload_is_still_classifiable() {
    assert!(!matches!(
        classify(&ms(&[65.0; 8])),
        StorageVerdict::Inconclusive { .. }
    ));
}

#[test]
fn too_few_samples_is_inconclusive_rather_than_a_guess() {
    for n in 0..6 {
        match classify(&ms(&vec![65.0; n])) {
            StorageVerdict::Inconclusive { samples, .. } => assert_eq!(samples, n),
            other => panic!(
                "{n} samples should be inconclusive, got {}",
                other.summary()
            ),
        }
    }
}

/// Inconclusive must never be read as "no flash". Silently treating an unmeasurable result
/// as safe is exactly how a wear budget becomes fiction.
#[test]
fn inconclusive_does_not_suggest_flash_but_is_not_streaming_either() {
    let v = classify(&ms(&[65.0, 65.0]));
    assert!(!v.suggests_flash());
    assert!(!matches!(v, StorageVerdict::StreamingLikely { .. }));
}

#[test]
fn a_zero_median_is_reported_as_a_timer_resolution_problem() {
    match classify(&ms(&[0.0; 10])) {
        StorageVerdict::Inconclusive { reason, .. } => assert!(reason.contains("resolution")),
        other => panic!("expected inconclusive, got {}", other.summary()),
    }
}

/// The median, not the mean, is the baseline: one 2-second erase would drag a mean far
/// enough that the erase no longer looks like an outlier against it.
#[test]
fn a_huge_stall_cannot_hide_itself_by_moving_the_baseline() {
    let v = classify(&ms(&[2000.0, 65.0, 65.0, 65.0, 65.0, 65.0, 65.0, 65.0]));
    assert!(v.suggests_flash(), "a 2s erase was hidden: {}", v.summary());
}

#[test]
fn summaries_are_human_readable_and_name_the_verdict() {
    assert!(classify(&ms(&[65.0; 8])).summary().contains("streaming"));
    assert!(
        classify(&ms(&[400.0, 65.0, 65.0, 65.0, 65.0, 65.0, 65.0, 65.0]))
            .summary()
            .contains("flash")
    );
    assert!(classify(&ms(&[65.0])).summary().contains("inconclusive"));
}
