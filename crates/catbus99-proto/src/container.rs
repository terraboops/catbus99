//! The TH99 Pro TFT **container**: the byte blob that the AA 50 reports carry.
//!
//! Layout (all offsets in bytes):
//!
//! ```text
//! 0..256    metadata
//!   [0]       N = frame count (1..=254)
//!   [1..N]    N-1 per-frame timing bytes
//!   [N]       0x00 terminator
//!   [N+1..]   0xFF fill
//! 256..     N frames, each 160x96 RGB565 little-endian (30,720 bytes)
//! ..        zero padding up to a 4096-byte boundary
//! ```
//!
//! This module performs no I/O.

use crate::error::ProtoError;

pub const SCREEN_W: usize = 160;
pub const SCREEN_H: usize = 96;
pub const BYTES_PER_PIXEL: usize = 2;

/// One full-screen frame: 160 x 96 pixels, RGB565 little-endian.
pub const FRAME_BYTES: usize = SCREEN_W * SCREEN_H * BYTES_PER_PIXEL; // 30,720

pub const METADATA_SIZE: usize = 256;
pub const BLOCK_SIZE: usize = 4096;

/// The terminator must land inside the 256-byte metadata block, and the frame count
/// occupies byte 0, so at most 254 frames can be described.
pub const MAX_FRAMES: usize = 254;

/// A parsed container, borrowed from the payload it was decoded out of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container<'a> {
    pub timings: &'a [u8],
    pub frames: Vec<&'a [u8]>,
}

impl Container<'_> {
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// True when every frame is byte-identical, i.e. the animation is really a still.
    pub fn is_static(&self) -> bool {
        self.frames.windows(2).all(|w| w[0] == w[1])
    }
}

/// Number of AA 50 reports a payload of this size will occupy.
pub fn report_count_for(payload_len: usize) -> usize {
    payload_len.div_ceil(BLOCK_SIZE)
}

/// Round `len` up to the next 4096-byte boundary.
fn padded_len(len: usize) -> usize {
    len.div_ceil(BLOCK_SIZE) * BLOCK_SIZE
}

/// Build a container from raw RGB565 frames plus `frames.len() - 1` timing bytes.
///
/// A single-frame container takes an empty `timings` slice.
pub fn build_container(frames: &[&[u8]], timings: &[u8]) -> Result<Vec<u8>, ProtoError> {
    let n = frames.len();
    if !(1..=MAX_FRAMES).contains(&n) {
        return Err(ProtoError::FrameCount {
            got: n,
            max: MAX_FRAMES,
        });
    }
    let expected_timings = n - 1;
    if timings.len() != expected_timings {
        return Err(ProtoError::TimingCount {
            frames: n,
            expected: expected_timings,
            got: timings.len(),
        });
    }
    for (index, frame) in frames.iter().enumerate() {
        if frame.len() != FRAME_BYTES {
            return Err(ProtoError::FrameSize {
                index,
                got: frame.len(),
                expected: FRAME_BYTES,
            });
        }
    }

    let body_len = METADATA_SIZE + n * FRAME_BYTES;
    let mut payload = vec![0u8; padded_len(body_len)];

    // Metadata: 0xFF fill, then overwrite the header fields. Writing the fill first and
    // the terminator last keeps the N == 1 case (empty timings) correct without a branch.
    payload[..METADATA_SIZE].fill(0xFF);
    payload[0] = n as u8;
    payload[1..1 + expected_timings].copy_from_slice(timings);
    payload[n] = 0x00;

    let mut offset = METADATA_SIZE;
    for frame in frames {
        payload[offset..offset + FRAME_BYTES].copy_from_slice(frame);
        offset += FRAME_BYTES;
    }
    // Remainder of `payload` is already zero from the initial allocation.

    Ok(payload)
}

/// Decode and fully validate a container.
///
/// Every structural invariant is checked, so a successful parse of a captured payload is
/// strong evidence our encoder and the firmware agree.
pub fn parse_container(payload: &[u8]) -> Result<Container<'_>, ProtoError> {
    if payload.is_empty() || payload.len() % BLOCK_SIZE != 0 {
        return Err(ProtoError::PayloadAlignment {
            got: payload.len(),
            block: BLOCK_SIZE,
        });
    }
    if payload.len() < METADATA_SIZE {
        return Err(ProtoError::PayloadAlignment {
            got: payload.len(),
            block: BLOCK_SIZE,
        });
    }

    let n = payload[0] as usize;
    if !(1..=MAX_FRAMES).contains(&n) {
        return Err(ProtoError::FrameCount {
            got: n,
            max: MAX_FRAMES,
        });
    }
    if payload[n] != 0x00 {
        return Err(ProtoError::MissingTerminator { offset: n });
    }
    for (offset, &byte) in payload[n + 1..METADATA_SIZE].iter().enumerate() {
        if byte != 0xFF {
            return Err(ProtoError::BadMetadataFill {
                offset: n + 1 + offset,
                got: byte,
            });
        }
    }

    let frame_end = METADATA_SIZE + n * FRAME_BYTES;
    if frame_end > payload.len() {
        return Err(ProtoError::DeclaredFramesExceedPayload {
            declared: n,
            available: (payload.len() - METADATA_SIZE) / FRAME_BYTES,
        });
    }

    let padding = &payload[frame_end..];
    if padding.len() >= BLOCK_SIZE || padding.iter().any(|&b| b != 0) {
        return Err(ProtoError::BadPadding {
            len: padding.len(),
            block: BLOCK_SIZE,
        });
    }

    let frames = (0..n)
        .map(|i| &payload[METADATA_SIZE + i * FRAME_BYTES..METADATA_SIZE + (i + 1) * FRAME_BYTES])
        .collect();

    Ok(Container {
        timings: &payload[1..n],
        frames,
    })
}

/// A solid-colour frame, for test patterns and backgrounds.
pub fn solid_frame(rgb565: u16) -> Vec<u8> {
    rgb565
        .to_le_bytes()
        .iter()
        .copied()
        .cycle()
        .take(FRAME_BYTES)
        .collect()
}

/// Pack 8-bit RGB into RGB565.
pub fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// Write one RGB565 pixel into a frame buffer at (x, y), little-endian.
pub fn put_pixel(frame: &mut [u8], x: usize, y: usize, color: u16) {
    if x >= SCREEN_W || y >= SCREEN_H {
        return;
    }
    let off = (y * SCREEN_W + x) * BYTES_PER_PIXEL;
    frame[off..off + 2].copy_from_slice(&color.to_le_bytes());
}

/// A diagnostic test pattern that makes every geometry and colour fault visible at a glance.
///
/// * Four horizontal colour bands (red, green, blue, white) — a wrong row stride shears
///   them into diagonals; swapped RGB565 fields recolour them.
/// * A black square in the **top-left** — the pattern is asymmetric, so a flip or rotation
///   moves the square to a different corner.
/// * A one-pixel white border — reveals off-by-one row/column errors.
pub fn test_pattern() -> Vec<u8> {
    let mut frame = vec![0u8; FRAME_BYTES];
    let bands = [
        rgb565(255, 0, 0),
        rgb565(0, 255, 0),
        rgb565(0, 0, 255),
        rgb565(255, 255, 255),
    ];
    let band_h = SCREEN_H / 4;

    for y in 0..SCREEN_H {
        for x in 0..SCREEN_W {
            put_pixel(&mut frame, x, y, bands[(y / band_h).min(3)]);
        }
    }
    // Asymmetric orientation marker: black square in the top-left corner.
    for y in 4..28 {
        for x in 4..28 {
            put_pixel(&mut frame, x, y, 0x0000);
        }
    }
    // One-pixel white border.
    for x in 0..SCREEN_W {
        put_pixel(&mut frame, x, 0, 0xFFFF);
        put_pixel(&mut frame, x, SCREEN_H - 1, 0xFFFF);
    }
    for y in 0..SCREEN_H {
        put_pixel(&mut frame, 0, y, 0xFFFF);
        put_pixel(&mut frame, SCREEN_W - 1, y, 0xFFFF);
    }
    frame
}
