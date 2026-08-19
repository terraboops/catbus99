//! MI_02 config channel: the AA 34 set-clock command.

use catbus99_proto::clock::*;
use catbus99_proto::error::ProtoError;

/// The documented example decodes to 2026-07-18 21:20:38, a Saturday.
#[test]
fn matches_the_documented_capture() {
    let when = ClockTime::new(2026, 7, 18, 21, 20, 38).unwrap();
    let packet = build_set_clock(when).unwrap();

    assert_eq!(
        &packet[..8],
        &[0xAA, 0x34, 0x38, 0x00, 0x00, 0x00, 0x01, 0x00]
    );
    // marker + yy mm dd HH MM SS weekday
    assert_eq!(
        &packet[8..18],
        &[0x5A, 0x01, 0x5A, 0x1A, 0x07, 0x12, 0x15, 0x14, 0x26, 0x06]
    );
    assert!(packet[18..].iter().all(|&b| b == 0));
}

/// Fields are plain binary, not BCD -- the firmware converts for the RTC itself.
/// Sending BCD would silently set the wrong time, so this is worth pinning.
#[test]
fn fields_are_plain_binary_not_bcd() {
    let when = ClockTime::new(2026, 12, 25, 23, 59, 59).unwrap();
    let packet = build_set_clock(when).unwrap();

    assert_eq!(packet[11], 26); // not 0x26
    assert_eq!(packet[12], 12); // not 0x12
    assert_eq!(packet[13], 25); // not 0x25
    assert_eq!(packet[14], 23);
    assert_eq!(packet[15], 59);
    assert_eq!(packet[16], 59);
}

#[test]
fn iso_weekday_is_monday_one_through_sunday_seven() {
    assert_eq!(iso_weekday(2026, 8, 17), 1); // Monday
    assert_eq!(iso_weekday(2026, 8, 18), 2); // Tuesday
    assert_eq!(iso_weekday(2026, 8, 22), 6); // Saturday
    assert_eq!(iso_weekday(2026, 8, 23), 7); // Sunday
    assert_eq!(iso_weekday(2026, 7, 18), 6); // the documented capture
    assert_eq!(iso_weekday(2024, 2, 29), 4); // leap day, Thursday
    assert_eq!(iso_weekday(2000, 1, 1), 6); // Saturday
}

#[test]
fn leap_years_follow_the_gregorian_rule() {
    assert!(is_leap_year(2024));
    assert!(is_leap_year(2000));
    assert!(!is_leap_year(2026));
    assert!(!is_leap_year(2100));
    assert_eq!(days_in_month(2024, 2), 29);
    assert_eq!(days_in_month(2026, 2), 28);
}

#[test]
fn round_trips_through_parse() {
    let when = ClockTime::new(2026, 8, 18, 14, 30, 5).unwrap();
    let packet = build_set_clock(when).unwrap();
    assert_eq!(parse_clock_packet(&packet, REQUEST_PREFIX).unwrap(), when);
}

/// The ACK is the request echoed back with 0xAA replaced by 0x55.
#[test]
fn recognises_the_acknowledgement() {
    let when = ClockTime::new(2026, 8, 18, 14, 30, 5).unwrap();
    let request = build_set_clock(when).unwrap();

    let mut ack = request;
    ack[0] = RESPONSE_PREFIX;
    assert!(is_clock_ack(&request, &ack));
    validate_clock_packet(&ack, RESPONSE_PREFIX).unwrap();

    let mut wrong = ack;
    wrong[16] = 0x00; // seconds differ
    assert!(!is_clock_ack(&request, &wrong));
}

#[test]
fn rejects_impossible_dates() {
    assert!(ClockTime::new(2026, 2, 30, 0, 0, 0).is_err());
    assert!(ClockTime::new(2026, 13, 1, 0, 0, 0).is_err());
    assert!(ClockTime::new(2026, 0, 1, 0, 0, 0).is_err());
    assert!(ClockTime::new(1999, 1, 1, 0, 0, 0).is_err());
    assert!(ClockTime::new(2100, 1, 1, 0, 0, 0).is_err());
    assert!(ClockTime::new(2026, 1, 1, 24, 0, 0).is_err());
    assert!(ClockTime::new(2026, 1, 1, 0, 60, 0).is_err());
    assert!(ClockTime::new(2024, 2, 29, 0, 0, 0).is_ok());
}

/// A packet whose weekday byte disagrees with its date is corrupt, and would put the
/// keyboard's native screen into a visibly wrong state.
#[test]
fn rejects_a_weekday_that_contradicts_the_date() {
    let when = ClockTime::new(2026, 8, 18, 12, 0, 0).unwrap();
    let mut packet = build_set_clock(when).unwrap();
    packet[17] = 5; // claims Friday; 2026-08-18 is a Tuesday
    assert!(matches!(
        validate_clock_packet(&packet, REQUEST_PREFIX),
        Err(ProtoError::ClockWeekday {
            got: 5,
            expected: 2
        })
    ));
}

#[test]
fn rejects_structural_damage() {
    let when = ClockTime::new(2026, 8, 18, 12, 0, 0).unwrap();

    let mut bad_marker = build_set_clock(when).unwrap();
    bad_marker[9] = 0x00;
    assert!(matches!(
        validate_clock_packet(&bad_marker, REQUEST_PREFIX),
        Err(ProtoError::ClockMarker)
    ));

    let mut bad_padding = build_set_clock(when).unwrap();
    bad_padding[40] = 0x01;
    assert!(matches!(
        validate_clock_packet(&bad_padding, REQUEST_PREFIX),
        Err(ProtoError::ClockPadding)
    ));

    assert!(matches!(
        validate_clock_packet(&[0u8; 32], REQUEST_PREFIX),
        Err(ProtoError::ConfigPacketSize { .. })
    ));
}
