//! Rendering images and animations for the TH99 Pro's 160x96 RGB565 panel.
//!
//! The panel is small and 16-bit, so two things matter more than they would on a normal
//! display: how an image is fitted into 160x96, and how 24-bit colour is reduced to
//! 5/6/5 without visible banding.

use catbus99_proto::container::{
    build_container, rgb565, BYTES_PER_PIXEL, FRAME_BYTES, SCREEN_H, SCREEN_W,
};
pub mod compose;
pub mod font5x8;
pub mod text;

pub use compose::compose;

use image::imageops::FilterType;
use image::{AnimationDecoder, ImageReader, Rgba, RgbaImage};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not decode image: {0}")]
    Decode(#[from] image::ImageError),
    #[error("image contains no frames")]
    Empty,
    #[error(transparent)]
    Proto(#[from] catbus99_proto::ProtoError),
}

/// How to fit a source image into the 160x96 panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Scale to fit entirely, letterboxing with the background colour. No cropping.
    #[default]
    Contain,
    /// Scale to fill, cropping the overflow. No letterboxing.
    Cover,
    /// Ignore aspect ratio.
    Stretch,
}

/// A rendered frame plus how long it should be shown, in milliseconds.
///
/// Duration is kept in milliseconds rather than as a raw delay byte because a frame may
/// need to be held far longer than one byte can express; see [`frames_to_container`].
pub struct Frame {
    pub pixels: Vec<u8>,
    pub duration_ms: u16,
}

/// 4x4 Bayer matrix for ordered dithering.
///
/// Reducing 8 bits per channel to 5/6/5 produces visible banding on gradients and on the
/// anti-aliased edges of text -- exactly what this panel mostly displays. A cheap ordered
/// dither trades that banding for high-frequency noise the eye reads as smoother.
const BAYER4: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

fn dither_offset(x: usize, y: usize, bits: u32) -> i32 {
    let levels = (1i32 << bits) - 1;
    let step = 255 / levels;
    // Centre the 0..15 Bayer value around zero, then scale to one quantisation step.
    (BAYER4[y % 4][x % 4] - 8) * step / 16
}

fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Convert an RGBA image to a packed RGB565 little-endian frame.
pub fn to_rgb565(img: &RgbaImage, dither: bool) -> Vec<u8> {
    let mut out = vec![0u8; FRAME_BYTES];
    for y in 0..SCREEN_H {
        for x in 0..SCREEN_W {
            let Rgba([r, g, b, _]) = *img.get_pixel(x as u32, y as u32);
            let (r, g, b) = if dither {
                (
                    clamp_u8(r as i32 + dither_offset(x, y, 5)),
                    clamp_u8(g as i32 + dither_offset(x, y, 6)),
                    clamp_u8(b as i32 + dither_offset(x, y, 5)),
                )
            } else {
                (r, g, b)
            };
            let off = (y * SCREEN_W + x) * BYTES_PER_PIXEL;
            out[off..off + 2].copy_from_slice(&rgb565(r, g, b).to_le_bytes());
        }
    }
    out
}

/// Fit an arbitrary image onto a 160x96 canvas.
pub fn fit(img: &RgbaImage, mode: Fit, background: [u8; 4]) -> RgbaImage {
    let (sw, sh) = (img.width() as f32, img.height() as f32);
    let (dw, dh) = (SCREEN_W as f32, SCREEN_H as f32);

    let (tw, th) = match mode {
        Fit::Stretch => (dw, dh),
        Fit::Contain => {
            let s = (dw / sw).min(dh / sh);
            (sw * s, sh * s)
        }
        Fit::Cover => {
            let s = (dw / sw).max(dh / sh);
            (sw * s, sh * s)
        }
    };

    let scaled = image::imageops::resize(
        img,
        tw.round().max(1.0) as u32,
        th.round().max(1.0) as u32,
        FilterType::Lanczos3,
    );

    let mut canvas = RgbaImage::from_pixel(SCREEN_W as u32, SCREEN_H as u32, Rgba(background));
    // Centre the scaled image; for Cover this crops symmetrically.
    let ox = (SCREEN_W as i64 - scaled.width() as i64) / 2;
    let oy = (SCREEN_H as i64 - scaled.height() as i64) / 2;
    image::imageops::overlay(&mut canvas, &scaled, ox, oy);
    canvas
}

/// Load an image or animation and render every frame for the panel.
///
/// Animated GIFs keep their per-frame timing; still images yield a single frame.
pub fn load_frames(path: &Path, mode: Fit, dither: bool) -> Result<Vec<Frame>, RenderError> {
    let is_gif = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("gif"))
        .unwrap_or(false);

    if is_gif {
        let file = std::fs::File::open(path).map_err(|source| RenderError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(file))?;
        let frames = decoder.into_frames().collect_frames()?;
        if frames.is_empty() {
            return Err(RenderError::Empty);
        }
        return Ok(frames
            .into_iter()
            .map(|f| {
                let (num, den) = f.delay().numer_denom_ms();
                // A zero denominator means the GIF declared no delay; 100ms is the
                // conventional browser fallback for that case.
                let ms = num.checked_div(den).unwrap_or(100);
                let buf = f.into_buffer();
                Frame {
                    pixels: to_rgb565(&fit(&buf, mode, [0, 0, 0, 255]), dither),
                    duration_ms: ms.clamp(1, u16::MAX as u32) as u16,
                }
            })
            .collect());
    }

    let img = ImageReader::open(path)
        .map_err(|source| RenderError::Read {
            path: path.display().to_string(),
            source,
        })?
        .with_guessed_format()
        .map_err(|source| RenderError::Read {
            path: path.display().to_string(),
            source,
        })?
        .decode()?
        .to_rgba8();

    Ok(vec![Frame {
        pixels: to_rgb565(&fit(&img, mode, [0, 0, 0, 255]), dither),
        duration_ms: 100,
    }])
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// A timing plan: which source frames to send, how many times each, and the tick.
struct Plan {
    tick: u32,
    /// `(source frame index, repeat count)`, in order.
    slots: Vec<(usize, usize)>,
}

impl Plan {
    fn frame_count(&self) -> usize {
        self.slots.iter().map(|(_, n)| n).sum()
    }
}

/// Decide how to represent an animation within a frame budget.
///
/// Two rules, in priority order:
///
/// 1. **Never drop a frame from the loop** while the budget can hold them all. Truncating
///    would show only the beginning of the animation; coarsening the tick shows all of it
///    slightly less precisely, which is far better.
/// 2. **Duplicate as little as possible.** The tick starts at the GCD of the durations --
///    the largest tick that reproduces the source timing exactly -- because each extra
///    frame costs another 30,720 bytes of upload.
fn plan_timing(frames: &[Frame], cap: usize) -> Plan {
    let durations: Vec<u32> = frames.iter().map(|f| f.duration_ms.max(1) as u32).collect();
    let total: u32 = durations.iter().sum();

    // More source frames than the budget: decimate evenly so the whole loop is still
    // represented, rather than keeping only a prefix.
    if frames.len() >= cap {
        let slots = (0..cap).map(|i| (i * frames.len() / cap, 1)).collect();
        return Plan {
            tick: (total / cap.max(1) as u32).clamp(1, 255),
            slots,
        };
    }

    // Preferred: exact timing via the GCD tick.
    let gcd_tick = durations.iter().copied().fold(0u32, gcd).clamp(1, 255);
    let exact: Vec<usize> = durations
        .iter()
        .map(|&d| (((d + gcd_tick / 2) / gcd_tick).max(1)) as usize)
        .collect();
    if exact.iter().sum::<usize>() <= cap {
        return Plan {
            tick: gcd_tick,
            slots: exact.into_iter().enumerate().collect(),
        };
    }

    // Otherwise share the budget proportionally, guaranteeing every frame at least one
    // slot, then trim the greediest until it fits.
    let mut repeats: Vec<usize> = durations
        .iter()
        .map(|&d| ((d as u64 * cap as u64) / total.max(1) as u64).max(1) as usize)
        .collect();
    while repeats.iter().sum::<usize>() > cap {
        let (i, _) = repeats
            .iter()
            .enumerate()
            .max_by_key(|(_, &n)| n)
            .expect("non-empty");
        if repeats[i] <= 1 {
            break;
        }
        repeats[i] -= 1;
    }

    let used: usize = repeats.iter().sum();
    Plan {
        tick: (total / used.max(1) as u32).clamp(1, 255),
        slots: repeats.into_iter().enumerate().collect(),
    }
}

/// Build an upload container from rendered frames, honouring their durations.
///
/// # Why frames get duplicated
///
/// The container carries `N - 1` single-byte delays, so no frame can be held longer than
/// 255 delay units. Real animations routinely need longer holds -- a cat that blinks once
/// every two seconds cannot be expressed as two frames at any delay value.
///
/// Duration is therefore handled here rather than in the protocol: pick one uniform tick
/// and repeat each frame `duration / tick` times. Because every delay byte then carries
/// the same value, it also stops mattering *which* frame an `N - 1` delay applies to --
/// an ambiguity we have not resolved from captures.
pub fn frames_to_container(frames: &[Frame], max_frames: usize) -> Result<Vec<u8>, RenderError> {
    if frames.is_empty() {
        return Err(RenderError::Empty);
    }
    let cap = max_frames.clamp(1, 250);
    if frames.len() == 1 || cap == 1 {
        return Ok(build_container(&[&frames[0].pixels], &[])?);
    }

    let plan = plan_timing(frames, cap);
    let mut expanded: Vec<&[u8]> = Vec::new();
    for (idx, n) in &plan.slots {
        for _ in 0..*n {
            expanded.push(&frames[*idx].pixels);
        }
    }
    let delays = vec![plan.tick as u8; expanded.len().saturating_sub(1)];
    Ok(build_container(&expanded, &delays)?)
}

/// Report what [`frames_to_container`] will do: (loop ms, tick, final frame count).
pub fn timing_plan(frames: &[Frame], max_frames: usize) -> (u32, u32, usize) {
    let total: u32 = frames.iter().map(|f| f.duration_ms.max(1) as u32).sum();
    let cap = max_frames.clamp(1, 250);
    if frames.is_empty() || frames.len() == 1 || cap == 1 {
        return (total, 0, 1);
    }
    let plan = plan_timing(frames, cap);
    (total, plan.tick, plan.frame_count())
}

/// Expand a packed RGB565 frame back to RGBA, for PNG previews.
///
/// The 5- and 6-bit channels are scaled so full-scale maps to 255 rather than 248/252,
/// which keeps whites white instead of slightly grey.
pub fn rgb565_to_rgba(frame: &[u8]) -> RgbaImage {
    let mut img = RgbaImage::new(SCREEN_W as u32, SCREEN_H as u32);
    for y in 0..SCREEN_H {
        for x in 0..SCREEN_W {
            let off = (y * SCREEN_W + x) * BYTES_PER_PIXEL;
            let v = u16::from_le_bytes([frame[off], frame[off + 1]]);
            let r = ((v >> 11) & 0x1F) as u32;
            let g = ((v >> 5) & 0x3F) as u32;
            let b = (v & 0x1F) as u32;
            img.put_pixel(
                x as u32,
                y as u32,
                Rgba([
                    ((r * 255 + 15) / 31) as u8,
                    ((g * 255 + 31) / 63) as u8,
                    ((b * 255 + 15) / 31) as u8,
                    255,
                ]),
            );
        }
    }
    img
}
