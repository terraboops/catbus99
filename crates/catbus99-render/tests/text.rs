//! The embedded 5x8 bitmap font.

use catbus99_model::{Align, Color, Rect};
use catbus99_render::text;
use image::{Rgba, RgbaImage};

fn canvas() -> RgbaImage {
    RgbaImage::from_pixel(160, 96, Rgba([0, 0, 0, 255]))
}

fn lit(img: &RgbaImage) -> usize {
    img.pixels()
        .filter(|p| p.0[3] == 255 && p.0[..3] != [0, 0, 0])
        .count()
}

#[test]
fn metrics_are_exact_multiples_of_the_cell() {
    assert_eq!(text::line_height(1), 8);
    assert_eq!(text::line_height(2), 16);
    assert_eq!(text::advance(1), 6);
    // Five chars: 5 cells of 6px, minus the trailing gap.
    assert_eq!(text::measure("HELLO", 1), 29);
    assert_eq!(text::measure("HELLO", 2), 58);
    assert_eq!(text::measure("", 1), 0);
}

#[test]
fn measurement_grows_with_length_and_scale() {
    assert!(text::measure("AA", 1) > text::measure("A", 1));
    assert!(text::measure("A", 2) > text::measure("A", 1));
}

/// Every pixel must be the exact foreground colour: no blending, no grey fringe.
/// Anti-aliased text was tried and is unreadable on the physical panel.
#[test]
fn glyph_pixels_are_pure_foreground() {
    let mut img = canvas();
    let fg = Color::new(0x4A, 0xC8, 0xFF);
    text::draw_text(&mut img, "ABC", 0, 0, 1, fg);
    for p in img.pixels() {
        let c = p.0;
        assert!(
            c[..3] == [0, 0, 0] || c[..3] == [fg.r, fg.g, fg.b],
            "found a blended pixel {c:?}"
        );
    }
}

#[test]
fn scaling_multiplies_the_lit_pixel_count() {
    let mut a = canvas();
    let mut b = canvas();
    text::draw_text(&mut a, "8", 0, 0, 1, Color::WHITE);
    text::draw_text(&mut b, "8", 0, 0, 2, Color::WHITE);
    // A 2x scale turns each source pixel into a 2x2 block.
    assert_eq!(lit(&b), lit(&a) * 4);
}

#[test]
fn text_is_clipped_at_the_edges_without_panicking() {
    let mut img = canvas();
    text::draw_text(&mut img, "EDGE", -20, -4, 2, Color::WHITE);
    text::draw_text(&mut img, "EDGE", 150, 90, 3, Color::WHITE);
    assert_eq!(img.dimensions(), (160, 96));
}

#[test]
fn unknown_characters_render_as_a_visible_substitute() {
    let mut img = canvas();
    // A character outside ASCII must not silently vanish.
    text::draw_text(&mut img, "\u{4e2d}", 0, 0, 1, Color::WHITE);
    assert!(lit(&img) > 0);
}

#[test]
fn a_space_draws_nothing_but_still_advances() {
    let mut img = canvas();
    text::draw_text(&mut img, " ", 0, 0, 1, Color::WHITE);
    assert_eq!(lit(&img), 0);
    assert_eq!(text::measure(" ", 1), 5);
}

#[test]
fn alignment_places_text_within_the_span() {
    let pos = |align| {
        let mut img = canvas();
        text::draw_aligned(
            &mut img,
            "HI",
            Rect::new(0, 0, 160, 8),
            1,
            align,
            Color::WHITE,
        );
        (0..160)
            .find(|&x| (0..8).any(|y| img.get_pixel(x, y).0[..3] != [0, 0, 0]))
            .unwrap()
    };
    let (l, c, r) = (pos(Align::Left), pos(Align::Center), pos(Align::Right));
    assert!(l < c && c < r, "left {l} center {c} right {r}");
}

#[test]
fn fit_scale_shrinks_until_it_fits() {
    assert_eq!(text::fit_scale("X", 200, 3), 3);
    assert_eq!(text::fit_scale("A VERY LONG STRING INDEED", 40, 3), 1);
    // Never returns zero, even when nothing fits.
    assert_eq!(text::fit_scale("IMPOSSIBLY LONG", 1, 3), 1);
}

#[test]
fn truncation_fits_the_width_and_marks_the_cut() {
    let s = text::truncate_to_fit("ABCDEFGHIJKLMNOP", 30, 1);
    assert!(text::measure(&s, 1) <= 30);
    assert!(s.ends_with('~'));
    // Text that already fits is returned untouched.
    assert_eq!(text::truncate_to_fit("AB", 100, 1), "AB");
}

// --- regressions found in adversarial review ---

/// `measure` previously multiplied without bounds and panicked with an overflow on a large
/// scale. Rendering must not be able to abort on a bad number.
#[test]
fn extreme_scales_do_not_overflow() {
    for scale in [0u32, 1, 12, 1000, u32::MAX] {
        // Every metric must return a usable, non-zero value at any scale.
        assert!(
            text::measure("XXXX", scale) > 0,
            "scale {scale} measured zero"
        );
        assert!(
            text::line_height(scale) > 0,
            "scale {scale} had zero line height"
        );
        assert!(text::advance(scale) > 0, "scale {scale} had zero advance");
    }
    // Clamped, so an absurd scale behaves like the largest sensible one.
    assert_eq!(
        text::measure("A", u32::MAX),
        text::measure("A", text::MAX_SCALE)
    );
    // Scale 0 is treated as 1 rather than producing a zero-width glyph.
    assert_eq!(text::measure("A", 0), text::measure("A", 1));
}

#[test]
fn a_very_long_string_measures_without_panicking() {
    let long = "X".repeat(100_000);
    let _ = text::measure(&long, text::MAX_SCALE);
    let mut img = RgbaImage::from_pixel(160, 96, Rgba([0, 0, 0, 255]));
    // Drawing it must clip rather than run away.
    text::draw_text(&mut img, &long, 0, 0, 1, Color::WHITE);
}
