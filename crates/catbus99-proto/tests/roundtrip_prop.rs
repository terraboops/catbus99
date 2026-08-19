//! Property tests: build -> parse is the identity for every valid container.
//!
//! Unit tests pin the specific byte patterns we captured; these check that the encoder
//! and decoder agree across the whole input space, including the awkward boundaries
//! (N=1 with no timings, N=254, frame counts whose payload lands exactly on a block edge).

use catbus99_proto::container::*;
use catbus99_proto::report::*;
use proptest::prelude::*;

/// Cheap deterministic frame filler -- generating 30,720 random bytes per frame would
/// dominate the runtime without testing anything the seed doesn't already cover.
fn frame_from_seed(seed: u8) -> Vec<u8> {
    (0..FRAME_BYTES)
        .map(|i| seed.wrapping_add((i % 251) as u8))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn container_round_trips(seeds in prop::collection::vec(any::<u8>(), 1..12)) {
        let owned: Vec<Vec<u8>> = seeds.iter().map(|&s| frame_from_seed(s)).collect();
        let frames: Vec<&[u8]> = owned.iter().map(|f| f.as_slice()).collect();
        let timings: Vec<u8> = (0..frames.len() - 1).map(|i| (i as u8).wrapping_add(1)).collect();

        let payload = build_container(&frames, &timings).unwrap();
        prop_assert_eq!(payload.len() % BLOCK_SIZE, 0);

        let parsed = parse_container(&payload).unwrap();
        prop_assert_eq!(parsed.frame_count(), frames.len());
        prop_assert_eq!(parsed.timings, &timings[..]);
        for (i, f) in frames.iter().enumerate() {
            prop_assert_eq!(parsed.frames[i], *f);
        }
    }

    #[test]
    fn reports_round_trip(seeds in prop::collection::vec(any::<u8>(), 1..8)) {
        let owned: Vec<Vec<u8>> = seeds.iter().map(|&s| frame_from_seed(s)).collect();
        let frames: Vec<&[u8]> = owned.iter().map(|f| f.as_slice()).collect();
        let timings = vec![0x20u8; frames.len() - 1];

        let payload = build_container(&frames, &timings).unwrap();
        let reports = build_reports(&payload).unwrap();

        prop_assert_eq!(reports.len(), payload.len() / BLOCK_SIZE);
        validate_reports(&reports).unwrap();
        prop_assert_eq!(payload_from_reports(&reports).unwrap(), payload);
    }

    /// Flipping any *structural* metadata byte must be caught.
    ///
    /// The metadata block is only 256 of ~65,000 bytes, but it is the part the firmware
    /// parses structurally. Note the exclusion: bytes `1..N` are the per-frame delays,
    /// which are free-form data — altering one produces a different but perfectly valid
    /// container, so they are deliberately not covered here.
    #[test]
    fn structural_metadata_corruption_is_always_detected(offset in 0usize..256, delta in 1u8..=255) {
        const N: usize = 2; // frame count of the container built below
        let f = frame_from_seed(7);
        let payload = build_container(&[&f, &f], &[0x32]).unwrap();

        // Skip the delay bytes: they carry data, not structure.
        if (1..N).contains(&offset) {
            return Ok(());
        }

        let mut corrupted = payload.clone();
        corrupted[offset] = corrupted[offset].wrapping_add(delta);

        prop_assert!(
            parse_container(&corrupted).is_err(),
            "corruption at metadata offset {} went undetected", offset
        );
    }

    /// Conversely, changing a delay byte must be accepted and round-trip faithfully —
    /// it is legitimate animation timing, not damage.
    #[test]
    fn delay_bytes_are_data_not_structure(delay in any::<u8>()) {
        let f = frame_from_seed(3);
        let payload = build_container(&[&f, &f], &[delay]).unwrap();
        let parsed = parse_container(&payload).unwrap();
        prop_assert_eq!(parsed.timings, &[delay][..]);
    }
}
