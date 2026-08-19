//! AA 50 report framing for the MI_03 TFT channel.
//!
//! A container is sliced into 4096-byte blocks, each prefixed with an 8-byte header:
//!
//! ```text
//! AA 50 | seq:u16le | count:u16le | 0x0650:u16le | <4096-byte block>
//! ```
//!
//! The keyboard acknowledges every report with a fixed 64-byte reply.

use crate::container::BLOCK_SIZE;
use crate::error::ProtoError;

pub const CMD_TFT: u8 = 0x50;
pub const REPORT_HEADER_SIZE: usize = 8;
pub const REPORT_SIZE: usize = REPORT_HEADER_SIZE + BLOCK_SIZE; // 4104

/// Constant in header bytes 6..8. Observed in every captured upload; meaning unknown.
pub const TRANSFER_CONSTANT: u16 = 0x0650;

pub const ACK_SIZE: usize = 64;

/// The one acknowledgement the keyboard sends per accepted report: `55 41 00 01` then zeros.
pub const ACK_PREFIX: [u8; 4] = [0x55, 0x41, 0x00, 0x01];

/// Build the full acknowledgement we expect back after every report.
pub fn expected_ack() -> [u8; ACK_SIZE] {
    let mut ack = [0u8; ACK_SIZE];
    ack[..4].copy_from_slice(&ACK_PREFIX);
    ack
}

/// True if `raw` is the exact acknowledgement the firmware sends.
///
/// Deliberately strict: we accept only the byte pattern actually observed. A lenient
/// check here would let a malformed or partial transfer look successful, and since the
/// screen offers no read-back, the ACK is the only machine-verifiable signal we get.
pub fn is_valid_ack(raw: &[u8]) -> bool {
    raw.len() == ACK_SIZE && raw[..4] == ACK_PREFIX && raw[4..].iter().all(|&b| b == 0)
}

/// Slice a container payload into framed AA 50 reports.
pub fn build_reports(payload: &[u8]) -> Result<Vec<Vec<u8>>, ProtoError> {
    if payload.is_empty() || payload.len() % BLOCK_SIZE != 0 {
        return Err(ProtoError::PayloadAlignment {
            got: payload.len(),
            block: BLOCK_SIZE,
        });
    }
    let count = payload.len() / BLOCK_SIZE;
    if count > u16::MAX as usize {
        return Err(ProtoError::TooManyReports { got: count });
    }

    let reports = payload
        .chunks_exact(BLOCK_SIZE)
        .enumerate()
        .map(|(seq, block)| {
            let mut report = Vec::with_capacity(REPORT_SIZE);
            report.push(0xAA);
            report.push(CMD_TFT);
            report.extend_from_slice(&(seq as u16).to_le_bytes());
            report.extend_from_slice(&(count as u16).to_le_bytes());
            report.extend_from_slice(&TRANSFER_CONSTANT.to_le_bytes());
            report.extend_from_slice(block);
            report
        })
        .collect::<Vec<_>>();

    validate_reports(&reports)?;
    Ok(reports)
}

/// Check a report stream against every header invariant.
pub fn validate_reports(reports: &[Vec<u8>]) -> Result<(), ProtoError> {
    let count = reports.len();
    if count > u16::MAX as usize {
        return Err(ProtoError::TooManyReports { got: count });
    }
    for (index, report) in reports.iter().enumerate() {
        if report.len() != REPORT_SIZE {
            return Err(ProtoError::ReportSize {
                index,
                got: report.len(),
                expected: REPORT_SIZE,
            });
        }
        if report[0] != 0xAA || report[1] != CMD_TFT {
            return Err(ProtoError::ReportMagic { index });
        }
        let seq = u16::from_le_bytes([report[2], report[3]]);
        if seq as usize != index {
            return Err(ProtoError::ReportSequence {
                index,
                got: seq,
                expected: index as u16,
            });
        }
        let declared = u16::from_le_bytes([report[4], report[5]]);
        if declared as usize != count {
            return Err(ProtoError::ReportCount {
                index,
                got: declared,
                expected: count as u16,
            });
        }
        let constant = u16::from_le_bytes([report[6], report[7]]);
        if constant != TRANSFER_CONSTANT {
            return Err(ProtoError::TransferConstant {
                index,
                got: constant,
                expected: TRANSFER_CONSTANT,
            });
        }
    }
    Ok(())
}

/// Reassemble the container payload from a report stream.
pub fn payload_from_reports(reports: &[Vec<u8>]) -> Result<Vec<u8>, ProtoError> {
    validate_reports(reports)?;
    Ok(reports
        .iter()
        .flat_map(|r| r[REPORT_HEADER_SIZE..].iter().copied())
        .collect())
}
