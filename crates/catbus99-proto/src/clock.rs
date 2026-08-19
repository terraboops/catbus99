//! The MI_02 config channel and the `AA 34` set-clock command.
//!
//! Config packets are a single 64-byte report:
//!
//! ```text
//! AA <cmd> <len> <off:u24le> <final> <rsvd> <payload...>
//! ```
//!
//! The keyboard acknowledges by echoing the same bytes with `0xAA` replaced by `0x55`.

use crate::error::ProtoError;

pub const CONFIG_PACKET_SIZE: usize = 64;
pub const CONFIG_HEADER_SIZE: usize = 8;
pub const REQUEST_PREFIX: u8 = 0xAA;
pub const RESPONSE_PREFIX: u8 = 0x55;

pub const CMD_SET_CLOCK: u8 = 0x34;
const CLOCK_DECLARED_LEN: u8 = 56;
const CLOCK_MARKER: [u8; 3] = [0x5A, 0x01, 0x5A];

/// A wall-clock instant as the keyboard expects it.
///
/// Fields are **plain binary, not BCD**. The keyboard's RTC (a PCF8563 clone) does use
/// BCD registers, but the firmware performs that conversion itself -- sending BCD here
/// would set the wrong time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl ClockTime {
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<Self, ProtoError> {
        if !(2000..=2099).contains(&year) {
            return Err(ProtoError::InvalidDateTime("year must be 2000..=2099"));
        }
        if !(1..=12).contains(&month) {
            return Err(ProtoError::InvalidDateTime("month must be 1..=12"));
        }
        if day < 1 || day > days_in_month(year, month) {
            return Err(ProtoError::InvalidDateTime("day out of range for month"));
        }
        if hour > 23 {
            return Err(ProtoError::InvalidDateTime("hour must be 0..=23"));
        }
        if minute > 59 {
            return Err(ProtoError::InvalidDateTime("minute must be 0..=59"));
        }
        if second > 59 {
            return Err(ProtoError::InvalidDateTime("second must be 0..=59"));
        }
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// ISO weekday: Monday = 1 ..= Sunday = 7.
    pub fn iso_weekday(&self) -> u8 {
        iso_weekday(self.year, self.month, self.day)
    }
}

pub fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Sakamoto's method, shifted from 0=Sunday to ISO 1=Monday..7=Sunday.
///
/// Implemented here rather than pulling in `chrono` so this crate stays dependency-free
/// and testable anywhere.
pub fn iso_weekday(year: u16, month: u8, day: u8) -> u8 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year as i32;
    if month < 3 {
        y -= 1;
    }
    let w = (y + y / 4 - y / 100 + y / 400 + T[(month - 1) as usize] + day as i32) % 7;
    if w == 0 {
        7
    } else {
        w as u8
    }
}

/// Build the 64-byte `AA 34` set-clock request.
pub fn build_set_clock(when: ClockTime) -> Result<[u8; CONFIG_PACKET_SIZE], ProtoError> {
    let mut packet = [0u8; CONFIG_PACKET_SIZE];
    packet[..CONFIG_HEADER_SIZE].copy_from_slice(&[
        REQUEST_PREFIX,
        CMD_SET_CLOCK,
        CLOCK_DECLARED_LEN,
        0x00,
        0x00,
        0x00,
        0x01, // final-packet flag
        0x00, // reserved
    ]);
    packet[8..11].copy_from_slice(&CLOCK_MARKER);
    packet[11] = (when.year - 2000) as u8;
    packet[12] = when.month;
    packet[13] = when.day;
    packet[14] = when.hour;
    packet[15] = when.minute;
    packet[16] = when.second;
    packet[17] = when.iso_weekday();

    validate_clock_packet(&packet, REQUEST_PREFIX)?;
    Ok(packet)
}

/// Validate a clock packet in either direction.
///
/// Pass [`REQUEST_PREFIX`] for something we built, [`RESPONSE_PREFIX`] for an ACK.
pub fn validate_clock_packet(packet: &[u8], prefix: u8) -> Result<(), ProtoError> {
    if packet.len() != CONFIG_PACKET_SIZE {
        return Err(ProtoError::ConfigPacketSize {
            got: packet.len(),
            expected: CONFIG_PACKET_SIZE,
        });
    }
    let expected_header = [
        prefix,
        CMD_SET_CLOCK,
        CLOCK_DECLARED_LEN,
        0x00,
        0x00,
        0x00,
        0x01,
        0x00,
    ];
    if packet[..CONFIG_HEADER_SIZE] != expected_header {
        return Err(ProtoError::ConfigHeader);
    }
    if packet[8..11] != CLOCK_MARKER {
        return Err(ProtoError::ClockMarker);
    }
    if packet[18..].iter().any(|&b| b != 0) {
        return Err(ProtoError::ClockPadding);
    }

    let when = ClockTime::new(
        2000 + packet[11] as u16,
        packet[12],
        packet[13],
        packet[14],
        packet[15],
        packet[16],
    )?;
    let expected = when.iso_weekday();
    if packet[17] != expected {
        return Err(ProtoError::ClockWeekday {
            got: packet[17],
            expected,
        });
    }
    Ok(())
}

/// Decode the datetime carried by a validated clock packet.
pub fn parse_clock_packet(packet: &[u8], prefix: u8) -> Result<ClockTime, ProtoError> {
    validate_clock_packet(packet, prefix)?;
    ClockTime::new(
        2000 + packet[11] as u16,
        packet[12],
        packet[13],
        packet[14],
        packet[15],
        packet[16],
    )
}

/// True when `response` is the exact ACK for `request` (same bytes, 0xAA -> 0x55).
pub fn is_clock_ack(request: &[u8], response: &[u8]) -> bool {
    request.len() == CONFIG_PACKET_SIZE
        && response.len() == CONFIG_PACKET_SIZE
        && response[0] == RESPONSE_PREFIX
        && response[1..] == request[1..]
}
