//! Container encoding/decoding, checked against the documented byte layout.

use catbus99_proto::container::*;
use catbus99_proto::error::ProtoError;

fn frame(byte: u8) -> Vec<u8> {
    vec![byte; FRAME_BYTES]
}

#[test]
fn frame_geometry_matches_the_panel() {
    assert_eq!(SCREEN_W, 160);
    assert_eq!(SCREEN_H, 96);
    assert_eq!(FRAME_BYTES, 30_720);
}

/// A two-frame container begins `<count> <delay> 00 FF...`. Our capture of the driver
/// writing a still image shows `02 19 00 ff`, so reproducing that shape with our own delay
/// byte is the strongest offline evidence that the encoder agrees with the firmware.
#[test]
fn two_frame_container_matches_the_documented_prefix() {
    let a = frame(0x00);
    let b = frame(0xFF);
    let payload = build_container(&[&a, &b], &[0x32]).unwrap();

    assert_eq!(&payload[..4], &[0x02, 0x32, 0x00, 0xFF]);
    assert!(payload[3..METADATA_SIZE].iter().all(|&x| x == 0xFF));
}

/// A two-frame upload is exactly one 64 KB flash erase block. This is the number the
/// entire wear budget is derived from, so it gets asserted rather than assumed.
#[test]
fn two_frame_container_is_exactly_one_64k_erase_block() {
    let a = frame(0x00);
    let b = frame(0xFF);
    let payload = build_container(&[&a, &b], &[0x32]).unwrap();

    assert_eq!(payload.len(), 65_536);
    assert_eq!(report_count_for(payload.len()), 16);
}

#[test]
fn single_frame_container_needs_no_timings() {
    let a = frame(0x41);
    let payload = build_container(&[&a], &[]).unwrap();

    // N=1: frame count, then the terminator immediately, then 0xFF fill.
    assert_eq!(&payload[..3], &[0x01, 0x00, 0xFF]);

    let parsed = parse_container(&payload).unwrap();
    assert_eq!(parsed.frame_count(), 1);
    assert!(parsed.timings.is_empty());
    assert_eq!(parsed.frames[0], &a[..]);
}

#[test]
fn round_trip_preserves_frames_and_timings() {
    let a = frame(0x11);
    let b = frame(0x22);
    let c = frame(0x33);
    let timings = [0x0F, 0x32];
    let payload = build_container(&[&a, &b, &c], &timings).unwrap();

    let parsed = parse_container(&payload).unwrap();
    assert_eq!(parsed.frame_count(), 3);
    assert_eq!(parsed.timings, &timings);
    assert_eq!(parsed.frames[0], &a[..]);
    assert_eq!(parsed.frames[1], &b[..]);
    assert_eq!(parsed.frames[2], &c[..]);
    assert!(!parsed.is_static());
}

#[test]
fn payload_is_always_block_aligned() {
    for n in [1usize, 2, 3, 7, 16, 254] {
        let f = frame(0xAB);
        let frames: Vec<&[u8]> = (0..n).map(|_| &f[..]).collect();
        let timings = vec![0x10u8; n - 1];
        let payload = build_container(&frames, &timings).unwrap();
        assert_eq!(payload.len() % BLOCK_SIZE, 0, "n={n} was not block aligned");
        parse_container(&payload).unwrap();
    }
}

#[test]
fn identical_frames_are_detected_as_static() {
    let a = frame(0x55);
    let payload = build_container(&[&a, &a], &[0x32]).unwrap();
    assert!(parse_container(&payload).unwrap().is_static());
}

#[test]
fn rejects_zero_frames() {
    assert_eq!(
        build_container(&[], &[]),
        Err(ProtoError::FrameCount { got: 0, max: 254 })
    );
}

#[test]
fn rejects_more_than_254_frames() {
    let f = frame(0);
    let frames: Vec<&[u8]> = (0..255).map(|_| &f[..]).collect();
    assert_eq!(
        build_container(&frames, &vec![0u8; 254]),
        Err(ProtoError::FrameCount { got: 255, max: 254 })
    );
}

#[test]
fn rejects_wrong_timing_count() {
    let a = frame(0);
    let b = frame(1);
    assert_eq!(
        build_container(&[&a, &b], &[]),
        Err(ProtoError::TimingCount {
            frames: 2,
            expected: 1,
            got: 0
        })
    );
}

#[test]
fn rejects_wrong_frame_size() {
    let a = frame(0);
    let short = vec![0u8; 100];
    assert_eq!(
        build_container(&[&a, &short], &[0x10]),
        Err(ProtoError::FrameSize {
            index: 1,
            got: 100,
            expected: FRAME_BYTES
        })
    );
}

#[test]
fn rejects_unaligned_payload() {
    assert!(matches!(
        parse_container(&[0u8; 100]),
        Err(ProtoError::PayloadAlignment { .. })
    ));
}

#[test]
fn rejects_missing_terminator() {
    let a = frame(0);
    let mut payload = build_container(&[&a], &[]).unwrap();
    payload[1] = 0xFF; // clobber the terminator
    assert!(matches!(
        parse_container(&payload),
        Err(ProtoError::MissingTerminator { offset: 1 })
    ));
}

#[test]
fn rejects_corrupted_metadata_fill() {
    let a = frame(0);
    let mut payload = build_container(&[&a], &[]).unwrap();
    payload[200] = 0x00;
    assert!(matches!(
        parse_container(&payload),
        Err(ProtoError::BadMetadataFill { offset: 200, .. })
    ));
}

#[test]
fn rejects_nonzero_padding() {
    let a = frame(0);
    let mut payload = build_container(&[&a], &[]).unwrap();
    let last = payload.len() - 1;
    payload[last] = 0x01;
    assert!(matches!(
        parse_container(&payload),
        Err(ProtoError::BadPadding { .. })
    ));
}

#[test]
fn solid_frame_is_little_endian() {
    let f = solid_frame(0xF800); // pure red in RGB565
    assert_eq!(f.len(), FRAME_BYTES);
    assert_eq!(&f[..4], &[0x00, 0xF8, 0x00, 0xF8]);
}
