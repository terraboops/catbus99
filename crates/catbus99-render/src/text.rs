//! Crisp text for a 160x96 panel, drawn from an embedded 5x8 bitmap font.
//!
//! # Why a bitmap font rather than a scaled TTF
//!
//! An outline font was tried first (Silkscreen via `ab_glyph`) in two variants, and both
//! failed on the real panel:
//!
//! * **Thresholded coverage.** Measured across 220 sizes, Silkscreen never rasterised
//!   better than 74% "crisp" (coverage near 0 or 1), and only at ~20px where the cap
//!   height is 13px -- far too tall for a 96px screen. At 8px almost every pixel carried
//!   *partial* coverage, so thresholding deleted stems rather than sharpening them.
//! * **Alpha blending.** Legible in a magnified PNG preview, but on the physical display
//!   6px anti-aliased text is low-contrast grey mush.
//!
//! A bitmap font sidesteps the whole problem: glyphs are defined on the pixel grid, every
//! pixel is fully on or fully off, and integer scaling keeps stems exactly one, two, or
//! three pixels wide. On a panel this small, contrast beats letterform fidelity.
//!
//! The face is Spleen 5x8 (BSD-2-Clause); see `assets/fonts/LICENSE.spleen`.

use crate::font5x8::{FIRST_CHAR, GLYPHS, GLYPH_H, GLYPH_W, LAST_CHAR};
use catbus99_model::{Align, Color, Rect};
use image::{Rgba, RgbaImage};

/// Horizontal gap between glyph cells, in unscaled pixels.
pub const LETTER_SPACING: u32 = 1;

/// Largest useful scale. The panel is 96px tall, so anything beyond this cannot fit a
/// single line, and clamping keeps the metric arithmetic well away from overflow.
pub const MAX_SCALE: u32 = 12;

fn clamp_scale(scale: u32) -> u32 {
    scale.clamp(1, MAX_SCALE)
}

/// Advance per character at `scale`.
pub fn advance(scale: u32) -> u32 {
    (GLYPH_W + LETTER_SPACING) * clamp_scale(scale)
}

/// Line height at `scale`.
pub fn line_height(scale: u32) -> u32 {
    GLYPH_H * clamp_scale(scale)
}

/// Rendered width of `text` at `scale`, excluding the trailing letter gap.
///
/// Saturating throughout: a caller passing an absurd scale or a very long string should
/// get a large number, not an arithmetic panic in the middle of rendering a screen.
pub fn measure(text: &str, scale: u32) -> u32 {
    let n = text.chars().count() as u32;
    if n == 0 {
        return 0;
    }
    let s = clamp_scale(scale);
    n.saturating_mul(advance(s))
        .saturating_sub(LETTER_SPACING * s)
}

fn glyph_for(c: char) -> &'static [u8; 8] {
    let idx = match u8::try_from(c as u32) {
        Ok(b) if (FIRST_CHAR..=LAST_CHAR).contains(&b) => (b - FIRST_CHAR) as usize,
        // Anything outside the table renders as '?' rather than vanishing silently.
        _ => (b'?' - FIRST_CHAR) as usize,
    };
    &GLYPHS[idx]
}

/// Draw one line of text with its top-left at `x`, `y`. Returns the width drawn.
///
/// Pixels are written at full colour or not at all -- there is no blending, so text keeps
/// maximum contrast against the background after RGB565 quantisation.
pub fn draw_text(img: &mut RgbaImage, text: &str, x: i64, y: i64, scale: u32, color: Color) -> u32 {
    let s = clamp_scale(scale);
    let px = Rgba([color.r, color.g, color.b, 255]);
    let mut pen = x;

    for ch in text.chars() {
        let glyph = glyph_for(ch);
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..GLYPH_W {
                // Bit 7 is the leftmost column of the 5-pixel cell.
                if bits & (0x80 >> col) == 0 {
                    continue;
                }
                // Each source pixel becomes an s x s block, keeping stems even.
                for dy in 0..s {
                    for dx in 0..s {
                        let ix = pen + (col * s + dx) as i64;
                        let iy = y + (row as u32 * s + dy) as i64;
                        if ix >= 0
                            && iy >= 0
                            && (ix as u32) < img.width()
                            && (iy as u32) < img.height()
                        {
                            img.put_pixel(ix as u32, iy as u32, px);
                        }
                    }
                }
            }
        }
        pen += advance(s) as i64;
    }
    measure(text, s)
}

/// Draw text horizontally aligned within `area`, at `area`'s top edge.
pub fn draw_aligned(
    img: &mut RgbaImage,
    text: &str,
    area: Rect,
    scale: u32,
    align: Align,
    color: Color,
) {
    let w = measure(text, scale);
    let ox = match align {
        Align::Left => 0,
        Align::Center => (area.w as i64 - w as i64) / 2,
        Align::Right => area.w as i64 - w as i64,
    };
    draw_text(img, text, area.x as i64 + ox, area.y as i64, scale, color);
}

/// Draw text aligned horizontally and centred vertically within `area`.
pub fn draw_centered_in(
    img: &mut RgbaImage,
    text: &str,
    area: Rect,
    scale: u32,
    align: Align,
    color: Color,
) {
    let y = area.y + area.h.saturating_sub(line_height(scale)) / 2;
    draw_aligned(
        img,
        text,
        Rect::new(area.x, y, area.w, area.h),
        scale,
        align,
        color,
    );
}

/// Largest scale at or below `preferred` whose rendering of `text` fits `width`.
///
/// Overflowing text is worse than smaller text here: a clipped number can read as a
/// different, entirely plausible number.
pub fn fit_scale(text: &str, width: u32, preferred: u32) -> u32 {
    (1..=preferred.max(1))
        .rev()
        .find(|&s| measure(text, s) <= width)
        .unwrap_or(1)
}

/// Truncate `text` with an ellipsis so it fits `width` at `scale`.
pub fn truncate_to_fit(text: &str, width: u32, scale: u32) -> String {
    if measure(text, scale) <= width {
        return text.to_string();
    }
    let per = advance(scale).max(1);
    let max_chars = (width / per) as usize;
    if max_chars <= 1 {
        return String::new();
    }
    text.chars()
        .take(max_chars - 1)
        .chain(std::iter::once('~'))
        .collect()
}
