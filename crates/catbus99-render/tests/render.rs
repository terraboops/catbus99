//! Rendering: colour conversion, fitting, and animation timing.

use catbus99_proto::container::{parse_container, FRAME_BYTES, SCREEN_H, SCREEN_W};
use catbus99_render::*;
use image::{Rgba, RgbaImage};

fn frame(duration_ms: u16, fill: u8) -> Frame {
    Frame {
        pixels: vec![fill; FRAME_BYTES],
        duration_ms,
    }
}

fn solid(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba(px))
}

#[test]
fn rgb565_round_trips_through_preview() {
    let img = solid(SCREEN_W as u32, SCREEN_H as u32, [255, 0, 0, 255]);
    let packed = to_rgb565(&img, false);
    let back = rgb565_to_rgba(&packed);
    // Pure red survives 5/6/5 exactly once channels are rescaled to full range.
    assert_eq!(*back.get_pixel(0, 0), Rgba([255, 0, 0, 255]));
}

#[test]
fn white_stays_white_and_black_stays_black() {
    for (px, expect) in [([255u8; 4], 255u8), ([0, 0, 0, 255], 0)] {
        let img = solid(SCREEN_W as u32, SCREEN_H as u32, px);
        let back = rgb565_to_rgba(&to_rgb565(&img, false));
        let Rgba([r, g, b, _]) = *back.get_pixel(5, 5);
        assert_eq!((r, g, b), (expect, expect, expect));
    }
}

#[test]
fn every_fit_mode_produces_a_full_panel() {
    for mode in [Fit::Contain, Fit::Cover, Fit::Stretch] {
        for (w, h) in [(10u32, 400u32), (400, 10), (160, 96), (1, 1)] {
            let out = fit(&solid(w, h, [10, 20, 30, 255]), mode, [0, 0, 0, 255]);
            assert_eq!(out.width(), SCREEN_W as u32);
            assert_eq!(out.height(), SCREEN_H as u32);
        }
    }
}

#[test]
fn contain_letterboxes_with_the_background() {
    // A tall source leaves background bars at the left and right edges.
    let out = fit(
        &solid(10, 400, [255, 255, 255, 255]),
        Fit::Contain,
        [0, 0, 0, 255],
    );
    assert_eq!(*out.get_pixel(0, SCREEN_H as u32 / 2), Rgba([0, 0, 0, 255]));
}

#[test]
fn cover_leaves_no_background_visible() {
    let out = fit(
        &solid(10, 400, [255, 255, 255, 255]),
        Fit::Cover,
        [0, 0, 0, 255],
    );
    for x in [0, SCREEN_W as u32 - 1] {
        assert_ne!(*out.get_pixel(x, SCREEN_H as u32 / 2), Rgba([0, 0, 0, 255]));
    }
}

#[test]
fn frames_are_the_exact_panel_size() {
    assert_eq!(
        to_rgb565(&solid(SCREEN_W as u32, SCREEN_H as u32, [1; 4]), true).len(),
        FRAME_BYTES
    );
}

/// The tick should be the GCD of the durations: the largest tick reproducing the source
/// timing exactly, so no frame is duplicated unnecessarily.
#[test]
fn tick_is_the_gcd_so_equal_frames_are_not_duplicated() {
    let f = vec![frame(100, 1), frame(100, 2)];
    let (total, tick, n) = timing_plan(&f, 24);
    assert_eq!((total, tick, n), (200, 100, 2));
}

#[test]
fn a_long_hold_is_expressed_by_duplicating_frames() {
    // 255 is the largest representable delay, so a 2s hold *must* duplicate.
    let f = vec![frame(2000, 1), frame(100, 2)];
    let (total, tick, n) = timing_plan(&f, 24);
    assert_eq!(total, 2100);
    assert_eq!(tick, 100);
    assert_eq!(n, 21); // 20 held frames + 1
}

#[test]
fn the_frame_budget_is_never_exceeded() {
    let f = vec![frame(5000, 1), frame(100, 2)];
    for cap in [1usize, 2, 5, 16, 24, 250] {
        let (_, _, n) = timing_plan(&f, cap);
        assert!(n <= cap, "cap {cap} produced {n} frames");
        let payload = frames_to_container(&f, cap).unwrap();
        assert_eq!(parse_container(&payload).unwrap().frame_count(), n.max(1));
    }
}

#[test]
fn coarsens_the_tick_rather_than_truncating_the_loop() {
    // A tight budget must still cover the whole animation, not just its beginning.
    let f = vec![frame(1000, 1), frame(1000, 2)];
    let payload = frames_to_container(&f, 4).unwrap();
    let parsed = parse_container(&payload).unwrap();
    assert!(parsed.frame_count() <= 4);
    // Both source frames are represented.
    assert!(parsed.frames.iter().any(|fr| fr[0] == 1));
    assert!(parsed.frames.iter().any(|fr| fr[0] == 2));
}

#[test]
fn a_single_frame_needs_no_delays() {
    let payload = frames_to_container(&[frame(100, 7)], 16).unwrap();
    let parsed = parse_container(&payload).unwrap();
    assert_eq!(parsed.frame_count(), 1);
    assert!(parsed.timings.is_empty());
    // One frame is 8 reports -- half the bytes of a two-frame still.
    assert_eq!(payload.len(), 32_768);
}

#[test]
fn all_delay_bytes_are_uniform() {
    // Uniform delays make the container's N-1 ambiguity irrelevant.
    let f = vec![frame(300, 1), frame(100, 2), frame(200, 3)];
    let payload = frames_to_container(&f, 24).unwrap();
    let parsed = parse_container(&payload).unwrap();
    assert!(parsed.timings.windows(2).all(|w| w[0] == w[1]));
}

#[test]
fn rejects_an_empty_animation() {
    assert!(frames_to_container(&[], 16).is_err());
}
