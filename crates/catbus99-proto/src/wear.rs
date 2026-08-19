//! Flash-endurance arithmetic for the display's SPI flash.
//!
//! The screen's storage is a Puya PY25Q128HA rated at 100,000 program/erase cycles per
//! erase block. The host protocol exposes no read-back or storage-commit operation, so
//! we cannot observe what an upload actually costs. We therefore assume, conservatively,
//! that **one upload costs one P/E cycle** and budget against that.

/// Datasheet endurance: program/erase cycles per erase block.
pub const RATED_PE_CYCLES: u64 = 100_000;

/// A two-frame container is 65,536 bytes -- exactly one 64 KB erase block.
pub const NOMINAL_UPLOAD_BYTES: usize = 65_536;

/// Default interval between scheduled screen writes.
pub const DEFAULT_WRITE_INTERVAL_SECS: u64 = 15 * 60;

/// Hard floor for *automatic* writes. Not overridable by config: a user can opt into
/// faster updates, but not into a rate that burns the display inside a year.
pub const MIN_WRITE_INTERVAL_SECS: u64 = 5 * 60;

/// Uploads per day at a given interval, worst case (value changes every interval).
pub fn uploads_per_day(interval_secs: u64) -> f64 {
    if interval_secs == 0 {
        return f64::INFINITY;
    }
    86_400.0 / interval_secs as f64
}

/// Projected years to exhaust the rated budget at a given interval, worst case.
pub fn projected_years(interval_secs: u64) -> f64 {
    let per_day = uploads_per_day(interval_secs);
    if per_day == 0.0 {
        return f64::INFINITY;
    }
    RATED_PE_CYCLES as f64 / per_day / 365.25
}

/// Fraction of the rated budget consumed after `uploads` writes.
pub fn budget_used(uploads: u64) -> f64 {
    uploads as f64 / RATED_PE_CYCLES as f64
}

/// Remaining uploads before reaching the conservative rated budget.
pub fn budget_remaining(uploads: u64) -> u64 {
    RATED_PE_CYCLES.saturating_sub(uploads)
}
