//! AA 50 report framing for the MI_03 TFT channel.

use catbus99_proto::container::*;
use catbus99_proto::error::ProtoError;
use catbus99_proto::report::*;

fn two_frame_payload() -> Vec<u8> {
    let a = solid_frame(0x0000);
    let b = solid_frame(0xFFFF);
    build_container(&[&a, &b], &[0x32]).unwrap()
}

#[test]
fn report_size_matches_the_captured_wire_format() {
    assert_eq!(REPORT_HEADER_SIZE, 8);
    assert_eq!(REPORT_SIZE, 4104);
}

#[test]
fn two_frame_upload_is_sixteen_reports() {
    let reports = build_reports(&two_frame_payload()).unwrap();
    assert_eq!(reports.len(), 16);
    assert!(reports.iter().all(|r| r.len() == REPORT_SIZE));
}

/// Header layout: AA 50 | seq:u16le | count:u16le | 0650:u16le
#[test]
fn headers_carry_sequence_count_and_constant() {
    let reports = build_reports(&two_frame_payload()).unwrap();

    assert_eq!(
        &reports[0][..8],
        &[0xAA, 0x50, 0x00, 0x00, 0x10, 0x00, 0x50, 0x06]
    );
    assert_eq!(
        &reports[15][..8],
        &[0xAA, 0x50, 0x0F, 0x00, 0x10, 0x00, 0x50, 0x06]
    );

    for (i, r) in reports.iter().enumerate() {
        assert_eq!(u16::from_le_bytes([r[2], r[3]]), i as u16);
        assert_eq!(u16::from_le_bytes([r[4], r[5]]), 16);
        assert_eq!(u16::from_le_bytes([r[6], r[7]]), TRANSFER_CONSTANT);
    }
}

#[test]
fn reports_reassemble_into_the_original_payload() {
    let payload = two_frame_payload();
    let reports = build_reports(&payload).unwrap();
    assert_eq!(payload_from_reports(&reports).unwrap(), payload);
}

#[test]
fn the_expected_ack_is_55_41_00_01_then_zeros() {
    let ack = expected_ack();
    assert_eq!(ack.len(), 64);
    assert_eq!(&ack[..4], &[0x55, 0x41, 0x00, 0x01]);
    assert!(ack[4..].iter().all(|&b| b == 0));
    assert!(is_valid_ack(&ack));
}

/// The ACK is the only machine-verifiable signal the screen gives us -- there is no
/// read-back command. So near-misses must be rejected, not tolerated.
#[test]
fn ack_validation_rejects_near_misses() {
    let mut wrong_prefix = expected_ack();
    wrong_prefix[1] = 0x42;
    assert!(!is_valid_ack(&wrong_prefix));

    let mut dirty_tail = expected_ack();
    dirty_tail[63] = 0x01;
    assert!(!is_valid_ack(&dirty_tail));

    assert!(!is_valid_ack(&expected_ack()[..63]));
    assert!(!is_valid_ack(&[]));
}

#[test]
fn rejects_unaligned_payload() {
    assert!(matches!(
        build_reports(&[0u8; 5000]),
        Err(ProtoError::PayloadAlignment { .. })
    ));
    assert!(matches!(
        build_reports(&[]),
        Err(ProtoError::PayloadAlignment { .. })
    ));
}

#[test]
fn validation_catches_a_tampered_sequence_number() {
    let mut reports = build_reports(&two_frame_payload()).unwrap();
    reports[7][2] = 0xFF;
    assert!(matches!(
        validate_reports(&reports),
        Err(ProtoError::ReportSequence { index: 7, .. })
    ));
}

#[test]
fn validation_catches_a_tampered_magic() {
    let mut reports = build_reports(&two_frame_payload()).unwrap();
    reports[3][1] = 0x51;
    assert!(matches!(
        validate_reports(&reports),
        Err(ProtoError::ReportMagic { index: 3 })
    ));
}
