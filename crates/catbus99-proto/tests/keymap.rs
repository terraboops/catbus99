//! Keymap decoding, checked against a full stock keymap captured from a real TH99 Pro.

use catbus99_proto::keymap::*;

/// The stock keymap as Epomaker's driver read it: 8 pages of 56 bytes via `AA 12`.
const CAPTURED: &str = "\
02 00 29 00 02 00 3a 00 02 00 3b 00 02 00 3c 00 02 00 3d 00 02 00 3e 00 02 00 3f 00 02 00 40 00 02 00 41 00 02 00 42 00 02 00 43 00 02 00 44 00 02 00 45 00 03 e9 00 00 \
03 ea 00 00 02 00 a7 00 02 00 35 00 02 00 1e 00 02 00 1f 00 02 00 20 00 02 00 21 00 02 00 22 00 02 00 23 00 02 00 24 00 02 00 25 00 02 00 26 00 02 00 27 00 02 00 2d 00 \
02 00 2e 00 02 00 53 00 02 00 54 00 02 00 55 00 02 00 2b 00 02 00 14 00 02 00 1a 00 02 00 08 00 02 00 15 00 02 00 17 00 02 00 1c 00 02 00 18 00 02 00 0c 00 02 00 12 00 \
02 00 13 00 02 00 2f 00 02 00 30 00 02 00 5f 00 02 00 60 00 02 00 61 00 02 00 39 00 02 00 04 00 02 00 16 00 02 00 07 00 02 00 09 00 02 00 0a 00 02 00 0b 00 02 00 0d 00 \
02 00 0e 00 02 00 0f 00 02 00 33 00 02 00 34 00 02 00 31 00 02 00 5c 00 02 00 5d 00 02 00 5e 00 02 00 e1 00 02 00 1d 00 02 00 1b 00 02 00 06 00 02 00 19 00 02 00 05 00 \
02 00 11 00 02 00 10 00 02 00 36 00 02 00 37 00 02 00 38 00 02 00 e5 00 02 00 28 00 02 00 59 00 02 00 5a 00 02 00 5b 00 02 00 e0 00 02 00 e3 00 02 00 e2 00 02 00 2c 00 \
02 00 e6 00 02 00 af 00 02 00 65 00 02 00 e4 00 02 00 50 00 02 00 51 00 02 00 52 00 02 00 4f 00 02 00 2a 00 02 00 62 00 02 00 63 00 02 00 58 00 02 00 7d 00 02 00 32 00 \
02 00 64 00 02 00 46 00 02 00 47 00 02 00 e7 00 02 00 48 00 02 00 49 00 02 00 4a 00 02 00 4b 00 02 00 4c 00 02 00 4d 00 02 00 4e 00 02 00 56 00 02 00 57 00 02 00 7d 00";

fn captured_bytes() -> Vec<u8> {
    CAPTURED
        .split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

#[test]
fn the_capture_is_eight_full_pages() {
    let bytes = captured_bytes();
    assert_eq!(bytes.len(), 8 * PAGE_SIZE);
    assert_eq!(bytes.len() / ENTRY_SIZE, 112);
}

/// The decisive check: a stock keymap must decode to the keys actually printed on the
/// keyboard. The table is the firmware's **key matrix** -- 16 columns by 7 rows -- not
/// the visual layout, so the numpad is interleaved into each row rather than sitting at
/// the end.
#[test]
fn the_stock_keymap_decodes_to_the_real_layout() {
    let table = decode_table(&captured_bytes()).unwrap();
    let names: Vec<String> = table.iter().map(|b| b.name()).collect();
    assert_eq!(names.len(), MATRIX_COLS * MATRIX_ROWS);

    let row = |r: usize| &names[r * MATRIX_COLS..(r + 1) * MATRIX_COLS];

    // Row 0: Esc, F1..F12, two firmware functions, and the alternate Delete.
    assert_eq!(
        row(0)[..13],
        ["Esc", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12"]
    );

    // Row 1: number row, then the top of the numpad.
    assert_eq!(
        row(1)[..13],
        ["`", "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "-", "="]
    );
    assert_eq!(row(1)[13..], ["NumLock", "KP/", "KP*"]);

    // Row 2: QWERTY, then KP7-KP9.
    assert_eq!(
        row(2)[..11],
        ["Tab", "Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P"]
    );
    assert_eq!(row(2)[13..], ["KP7", "KP8", "KP9"]);

    // Row 3: home row, then KP4-KP6.
    assert_eq!(
        row(3)[..10],
        ["CapsLock", "A", "S", "D", "F", "G", "H", "J", "K", "L"]
    );
    assert_eq!(row(3)[13..], ["KP4", "KP5", "KP6"]);

    // Row 4: bottom letter row, then KP1-KP3.
    assert_eq!(
        row(4)[..8],
        ["LeftShift", "Z", "X", "C", "V", "B", "N", "M"]
    );
    assert_eq!(row(4)[13..], ["KP1", "KP2", "KP3"]);

    // Row 5: modifiers, space, arrows.
    assert_eq!(row(5)[..4], ["LeftCtrl", "LeftGUI", "LeftAlt", "Space"]);
    assert_eq!(row(5)[8..12], ["Left", "Down", "Up", "Right"]);

    // Every alphabetic key is present exactly once.
    for letter in ["A", "Q", "Z", "M", "P"] {
        assert_eq!(names.iter().filter(|n| *n == letter).count(), 1, "{letter}");
    }
}

/// Every entry must decode to *something*; a silently dropped key would corrupt a backup.
#[test]
fn no_entry_decodes_as_unknown() {
    let table = decode_table(&captured_bytes()).unwrap();
    let unknown: Vec<_> = table
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b, KeyBinding::Unknown(_)))
        .collect();
    assert!(unknown.is_empty(), "undecoded entries: {unknown:?}");
}

/// The two non-HID entries are firmware functions. Their meaning is *not* known, so they
/// must round-trip verbatim rather than being guessed at.
#[test]
fn firmware_function_entries_are_preserved_not_guessed() {
    let table = decode_table(&captured_bytes()).unwrap();
    let fns: Vec<_> = table
        .iter()
        .filter(|b| matches!(b, KeyBinding::Function(_)))
        .collect();
    assert_eq!(fns.len(), 2);
    assert_eq!(fns[0].name(), "FN(0xe9)");
    assert_eq!(fns[1].name(), "FN(0xea)");
}

/// A backup is only useful if it restores byte-for-byte.
#[test]
fn the_table_round_trips_exactly() {
    let bytes = captured_bytes();
    let table = decode_table(&bytes).unwrap();
    assert_eq!(encode_table(&table), bytes);
}

#[test]
fn page_requests_match_the_captured_offsets() {
    // The driver walked 0x000000, 0x000038, 0x000070, 0x0000a8, 0x0000e0, 0x000118, ...
    let expected: [u32; 6] = [0x00, 0x38, 0x70, 0xA8, 0xE0, 0x118];
    for (i, off) in expected.iter().enumerate() {
        let req = build_read_request(CMD_READ_BASIC, i as u32 * PAGE_SIZE as u32);
        assert_eq!(&req[..3], &[0xAA, CMD_READ_BASIC, PAGE_SIZE as u8]);
        let got = u32::from_le_bytes([req[3], req[4], req[5], 0]);
        assert_eq!(got, *off, "page {i}");
    }
}

#[test]
fn unassigned_and_unknown_entries_decode_distinctly() {
    assert_eq!(KeyBinding::decode([0, 0, 0, 0]), KeyBinding::None);
    assert_eq!(
        KeyBinding::decode([0x02, 0x00, 0x04, 0x00]),
        KeyBinding::Hid(0x04)
    );
    assert_eq!(
        KeyBinding::decode([0x03, 0xE9, 0x00, 0x00]),
        KeyBinding::Function(0xE9)
    );
    let weird = [0x09, 0x01, 0x02, 0x03];
    assert_eq!(KeyBinding::decode(weird), KeyBinding::Unknown(weird));
    assert!(KeyBinding::decode(weird).name().starts_with("RAW("));
}

#[test]
fn rejects_a_truncated_table() {
    assert!(decode_table(&[0x02, 0x00, 0x04]).is_err());
}

// --- regressions found in adversarial review ---

/// `decode` previously took a slice and indexed `[..4]`, panicking on short input — a poor
/// failure mode for a public function parsing bytes that came off a device.
#[test]
fn decoding_a_short_slice_returns_none_rather_than_panicking() {
    assert!(KeyBinding::try_decode(&[]).is_none());
    assert!(KeyBinding::try_decode(&[0x02]).is_none());
    assert!(KeyBinding::try_decode(&[0x02, 0x00, 0x04]).is_none());
    assert_eq!(
        KeyBinding::try_decode(&[0x02, 0x00, 0x04, 0x00]),
        Some(KeyBinding::Hid(0x04))
    );
    // Extra trailing bytes are ignored, not an error.
    assert_eq!(
        KeyBinding::try_decode(&[0x02, 0x00, 0x04, 0x00, 0xFF]),
        Some(KeyBinding::Hid(0x04))
    );
}
