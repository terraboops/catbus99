//! Classifying an upload's per-report timing to infer where the bytes actually land.
//!
//! # The question this answers
//!
//! catbus99's entire wear budget rests on one unverified assumption: that every screen
//! upload costs a program/erase cycle on the display's SPI flash. We never measured it.
//! The protocol has no read-back and no storage-commit command, so the storage behaviour
//! is not directly observable — but it is *indirectly* observable, because flash is slow
//! in a very particular way.
//!
//! A SPI NOR erase is orders of magnitude slower than a page program: a 4 KB sector erase
//! runs tens to hundreds of milliseconds, a 64 KB block erase can take over a second.
//! Streaming pixels straight to the panel controller's GRAM has no such step. So if the
//! firmware erases flash during an upload, one or more reports must stall.
//!
//! # Why total time does not answer it
//!
//! USB full-speed HID caps at roughly 64 KB/s, so a 16-report (64 KB) upload takes about a
//! second on the wire no matter what the firmware does with the bytes. The *total* is
//! dominated by the bus. The **distribution** is not: a stall shows up as one report taking
//! several times the median.
//!
//! # What a verdict is and is not
//!
//! This is inference from timing, not a storage trace. [`StorageVerdict::FlashLikely`] means
//! "something took far longer than the bus explains, and erase latency is the obvious
//! candidate". [`StorageVerdict::StreamingLikely`] means "nothing did". Neither is proof,
//! and the code says so — a wrong answer here would either waste display resolution or
//! destroy a panel, so it reports confidence rather than certainty.

use std::time::Duration;

/// A report is a stall if it takes at least this multiple of the median.
///
/// Chosen well above bus jitter: USB frame scheduling and host wake-up account for tens of
/// percent, not multiples. An erase is an order of magnitude.
const STALL_FACTOR: f64 = 3.0;

/// Below this many reports the distribution is too small to say anything.
const MIN_SAMPLES: usize = 6;

/// A stall must also exceed this in absolute terms, so a fast uniform upload with one
/// slightly-late report is not called flash.
const MIN_STALL_MS: f64 = 15.0;

/// What the timing distribution suggests about where uploaded bytes go.
#[derive(Debug, Clone, PartialEq)]
pub enum StorageVerdict {
    /// One or more reports stalled far beyond bus latency. Erase is the likely cause.
    FlashLikely {
        /// Indices of the reports that stalled.
        stalled_reports: Vec<usize>,
        median_ms: f64,
        worst_ms: f64,
    },
    /// No report stalled. Consistent with streaming straight to the panel controller.
    StreamingLikely { median_ms: f64, worst_ms: f64 },
    /// Not enough reports, or timings too degenerate, to distinguish.
    Inconclusive {
        reason: &'static str,
        samples: usize,
    },
}

impl StorageVerdict {
    /// A one-line summary for a human.
    pub fn summary(&self) -> String {
        match self {
            StorageVerdict::FlashLikely { stalled_reports, median_ms, worst_ms } => format!(
                "flash likely: {} report(s) stalled (worst {worst_ms:.0}ms vs {median_ms:.0}ms median) at {:?}",
                stalled_reports.len(),
                stalled_reports
            ),
            StorageVerdict::StreamingLikely { median_ms, worst_ms } => format!(
                "streaming likely: no stalls (worst {worst_ms:.0}ms vs {median_ms:.0}ms median)"
            ),
            StorageVerdict::Inconclusive { reason, samples } => {
                format!("inconclusive ({reason}, {samples} samples)")
            }
        }
    }

    /// True only for a positive flash finding. Callers must not treat
    /// [`StorageVerdict::Inconclusive`] as "no flash".
    pub fn suggests_flash(&self) -> bool {
        matches!(self, StorageVerdict::FlashLikely { .. })
    }
}

fn median_ms(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// Classify a sequence of per-report acknowledgement latencies.
///
/// The median is the baseline rather than the mean, because a single multi-hundred
/// millisecond erase would drag a mean far enough to hide itself.
pub fn classify(timings: &[Duration]) -> StorageVerdict {
    let ms: Vec<f64> = timings.iter().map(|d| d.as_secs_f64() * 1000.0).collect();

    if ms.len() < MIN_SAMPLES {
        return StorageVerdict::Inconclusive {
            reason: "too few reports to see a distribution",
            samples: ms.len(),
        };
    }

    let mut sorted = ms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = median_ms(&sorted);
    let worst = *sorted.last().unwrap_or(&0.0);

    if median <= 0.0 {
        return StorageVerdict::Inconclusive {
            reason: "median latency is zero; timer resolution too coarse",
            samples: ms.len(),
        };
    }

    let stalled: Vec<usize> = ms
        .iter()
        .enumerate()
        .filter(|(_, &t)| t >= median * STALL_FACTOR && t - median >= MIN_STALL_MS)
        .map(|(i, _)| i)
        .collect();

    if stalled.is_empty() {
        StorageVerdict::StreamingLikely {
            median_ms: median,
            worst_ms: worst,
        }
    } else {
        StorageVerdict::FlashLikely {
            stalled_reports: stalled,
            median_ms: median,
            worst_ms: worst,
        }
    }
}
