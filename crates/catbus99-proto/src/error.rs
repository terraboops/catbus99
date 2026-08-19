//! Errors produced while encoding or decoding TH99 Pro wire formats.

use thiserror::Error;

/// Every way a TH99 Pro payload can be malformed.
///
/// These are all *structural* faults detectable without a device attached, which is
/// what lets the entire wire format be tested in CI on any machine.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtoError {
    #[error("frame count must be 1..={max}, got {got}")]
    FrameCount { got: usize, max: usize },

    #[error(
        "a container with {frames} frames requires exactly {expected} timing byte(s), got {got}"
    )]
    TimingCount {
        frames: usize,
        expected: usize,
        got: usize,
    },

    #[error("frame {index} is {got} bytes, expected {expected} (160x96 RGB565)")]
    FrameSize {
        index: usize,
        got: usize,
        expected: usize,
    },

    #[error("payload is {got} bytes; must be a non-empty multiple of {block}")]
    PayloadAlignment { got: usize, block: usize },

    #[error("metadata declares {declared} frames but payload only holds {available}")]
    DeclaredFramesExceedPayload { declared: usize, available: usize },

    #[error("metadata is missing its 0x00 terminator at offset {offset}")]
    MissingTerminator { offset: usize },

    #[error("metadata fill at offset {offset} is 0x{got:02x}, expected 0xFF")]
    BadMetadataFill { offset: usize, got: u8 },

    #[error("trailing padding is {len} bytes and must be under {block} and all zero")]
    BadPadding { len: usize, block: usize },

    #[error("report {index} is {got} bytes, expected {expected}")]
    ReportSize {
        index: usize,
        got: usize,
        expected: usize,
    },

    #[error("report {index} does not start with AA 50")]
    ReportMagic { index: usize },

    #[error("report {index} declares sequence {got}, expected {expected}")]
    ReportSequence {
        index: usize,
        got: u16,
        expected: u16,
    },

    #[error("report {index} declares count {got}, expected {expected}")]
    ReportCount {
        index: usize,
        got: u16,
        expected: u16,
    },

    #[error("report {index} transfer constant is 0x{got:04x}, expected 0x{expected:04x}")]
    TransferConstant {
        index: usize,
        got: u16,
        expected: u16,
    },

    #[error("report count {got} exceeds the 16-bit sequence field")]
    TooManyReports { got: usize },

    #[error("invalid date/time: {0}")]
    InvalidDateTime(&'static str),

    #[error("config packet must be {expected} bytes, got {got}")]
    ConfigPacketSize { got: usize, expected: usize },

    #[error("config packet header mismatch")]
    ConfigHeader,

    #[error("clock packet marker mismatch")]
    ClockMarker,

    #[error("clock packet padding is not zero")]
    ClockPadding,

    #[error("clock packet weekday {got} does not match the date's weekday {expected}")]
    ClockWeekday { got: u8, expected: u8 },
}
