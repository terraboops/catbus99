//! The `MI_02` keymap table: reading and decoding what each key is bound to.
//!
//! Decoded from live capture of Epomaker's driver, which reads the table in 56-byte pages
//! (`AA 12`, 24-bit LE offset stepping by `0x38`). Each entry is four bytes:
//!
//! ```text
//! 02 00 <hid usage> 00     a standard HID Keyboard/Keypad usage
//! 03 <fn> 00 00            a firmware function (knob, layer, backlight, ...)
//! 00 00 00 00              unassigned
//! ```
//!
//! The type-`0x02` form is confirmed against a full stock keymap: `0x29` decodes to
//! Escape, `0x3A..=0x45` to F1..F12, `0x04` to A, `0xE1` to LeftShift, and so on across
//! all 112 entries. The type-`0x03` form carries a firmware-specific id in byte 1 whose
//! meanings are **not** established, so those entries are preserved verbatim rather than
//! guessed at — a backup that silently mistranslates a key is worse than one that admits
//! it does not know.

use crate::error::ProtoError;

/// Columns in the firmware's key matrix.
///
/// The table is a matrix, not the visual layout: each row of 16 holds a run of the main
/// block followed by that row's numpad keys. Decoding a full stock keymap makes this
/// unambiguous -- row 1 ends `NumLock KP/ KP*`, row 2 ends `KP7 KP8 KP9`, and so on.
pub const MATRIX_COLS: usize = 16;
/// Rows in the firmware's key matrix.
pub const MATRIX_ROWS: usize = 7;
/// Total addressable key positions.
pub const MATRIX_KEYS: usize = MATRIX_COLS * MATRIX_ROWS;

/// Entry size in the table.
pub const ENTRY_SIZE: usize = 4;
/// Bytes returned per `AA 12` page.
pub const PAGE_SIZE: usize = 56;
/// Entries per page.
pub const ENTRIES_PER_PAGE: usize = PAGE_SIZE / ENTRY_SIZE;

/// Read command for the basic (unshifted) keymap layer.
pub const CMD_READ_BASIC: u8 = 0x12;
/// Read command for the Fn layer.
pub const CMD_READ_FN: u8 = 0x16;

/// One key binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyBinding {
    /// Unassigned.
    None,
    /// A standard HID Keyboard/Keypad usage.
    Hid(u8),
    /// A firmware function; the id's meaning is not decoded.
    Function(u8),
    /// Something we do not recognise, kept byte-for-byte.
    Unknown([u8; ENTRY_SIZE]),
}

impl KeyBinding {
    /// Decode one entry.
    ///
    /// Takes a fixed-size array rather than a slice: the previous slice-taking version
    /// indexed `[..4]` unconditionally and panicked on short input, which is a poor
    /// failure mode for a public function parsing device data.
    pub fn decode(e: [u8; ENTRY_SIZE]) -> Self {
        match e {
            [0x00, 0x00, 0x00, 0x00] => KeyBinding::None,
            [0x02, 0x00, usage, 0x00] => KeyBinding::Hid(usage),
            [0x03, id, 0x00, 0x00] => KeyBinding::Function(id),
            other => KeyBinding::Unknown(other),
        }
    }

    /// Decode from a slice, returning `None` if it is too short.
    pub fn try_decode(bytes: &[u8]) -> Option<Self> {
        let e: [u8; ENTRY_SIZE] = bytes.get(..ENTRY_SIZE)?.try_into().ok()?;
        Some(Self::decode(e))
    }

    pub fn encode(self) -> [u8; ENTRY_SIZE] {
        match self {
            KeyBinding::None => [0, 0, 0, 0],
            KeyBinding::Hid(u) => [0x02, 0x00, u, 0x00],
            KeyBinding::Function(id) => [0x03, id, 0x00, 0x00],
            KeyBinding::Unknown(raw) => raw,
        }
    }

    /// A human-readable name, or a raw form when the meaning is unknown.
    pub fn name(self) -> String {
        match self {
            KeyBinding::None => "--".into(),
            KeyBinding::Hid(u) => hid_usage_name(u)
                .map(str::to_string)
                .unwrap_or_else(|| format!("HID(0x{u:02x})")),
            KeyBinding::Function(id) => format!("FN(0x{id:02x})"),
            KeyBinding::Unknown(raw) => format!(
                "RAW({})",
                raw.iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        }
    }
}

/// Decode a concatenated keymap table.
pub fn decode_table(bytes: &[u8]) -> Result<Vec<KeyBinding>, ProtoError> {
    if bytes.len() % ENTRY_SIZE != 0 {
        return Err(ProtoError::InvalidDateTime(
            "keymap table is not a multiple of 4 bytes",
        ));
    }
    Ok(bytes
        .chunks_exact(ENTRY_SIZE)
        .map(|c| {
            KeyBinding::decode(
                c.try_into()
                    .expect("chunks_exact yields exactly ENTRY_SIZE"),
            )
        })
        .collect())
}

/// Encode bindings back into table bytes.
pub fn encode_table(bindings: &[KeyBinding]) -> Vec<u8> {
    bindings.iter().flat_map(|b| b.encode()).collect()
}

/// Build the 64-byte `AA 12`/`AA 16` page-read request for a byte offset.
pub fn build_read_request(command: u8, offset: u32) -> [u8; 64] {
    let mut packet = [0u8; 64];
    packet[0] = 0xAA;
    packet[1] = command;
    packet[2] = PAGE_SIZE as u8;
    packet[3..6].copy_from_slice(&offset.to_le_bytes()[..3]);
    // Byte 6 is the final-packet flag; paged reads leave it clear.
    packet
}

/// Names for the HID Keyboard/Keypad usage page (0x07).
pub fn hid_usage_name(usage: u8) -> Option<&'static str> {
    Some(match usage {
        0x04..=0x1D => return LETTERS.get((usage - 0x04) as usize).copied(),
        0x1E => "1",
        0x1F => "2",
        0x20 => "3",
        0x21 => "4",
        0x22 => "5",
        0x23 => "6",
        0x24 => "7",
        0x25 => "8",
        0x26 => "9",
        0x27 => "0",
        0x28 => "Enter",
        0x29 => "Esc",
        0x2A => "Backspace",
        0x2B => "Tab",
        0x2C => "Space",
        0x2D => "-",
        0x2E => "=",
        0x2F => "[",
        0x30 => "]",
        0x31 => "\\",
        0x32 => "#",
        0x33 => ";",
        0x34 => "'",
        0x35 => "`",
        0x36 => ",",
        0x37 => ".",
        0x38 => "/",
        0x39 => "CapsLock",
        0x3A..=0x45 => return F_KEYS.get((usage - 0x3A) as usize).copied(),
        0x46 => "PrintScreen",
        0x47 => "ScrollLock",
        0x48 => "Pause",
        0x49 => "Insert",
        0x4A => "Home",
        0x4B => "PageUp",
        0x4C => "Delete",
        0x4D => "End",
        0x4E => "PageDown",
        0x4F => "Right",
        0x50 => "Left",
        0x51 => "Down",
        0x52 => "Up",
        0x53 => "NumLock",
        0x54 => "KP/",
        0x55 => "KP*",
        0x56 => "KP-",
        0x57 => "KP+",
        0x58 => "KPEnter",
        0x59 => "KP1",
        0x5A => "KP2",
        0x5B => "KP3",
        0x5C => "KP4",
        0x5D => "KP5",
        0x5E => "KP6",
        0x5F => "KP7",
        0x60 => "KP8",
        0x61 => "KP9",
        0x62 => "KP0",
        0x63 => "KP.",
        0x64 => "\\|",
        0x65 => "Menu",
        0x67 => "KP=",
        0x7D => "Mute",
        0x7E => "VolUp",
        0x7F => "VolDown",
        0xA7 => "Delete(alt)",
        0xAF => "Fn",
        0xE0 => "LeftCtrl",
        0xE1 => "LeftShift",
        0xE2 => "LeftAlt",
        0xE3 => "LeftGUI",
        0xE4 => "RightCtrl",
        0xE5 => "RightShift",
        0xE6 => "RightAlt",
        0xE7 => "RightGUI",
        _ => return None,
    })
}

const LETTERS: [&str; 26] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z",
];

const F_KEYS: [&str; 12] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
];
