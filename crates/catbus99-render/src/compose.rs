//! Compositing a [`Layout`] into a panel image.
//!
//! Widgets are drawn into their slot rectangles in declaration order, so a later slot
//! paints over an earlier one. Every draw is clipped to both the slot and the panel, so a
//! malformed layout can produce an ugly screen but never an out-of-bounds write.
//!
//! # Degraded data
//!
//! When a binding is missing or past its TTL the widget renders **dimmed**, with `--` in
//! place of text. On a glanceable, non-interactive display a number that has silently
//! stopped updating is worse than no number: the reader has no way to tell.

use catbus99_model::{
    Align, BarStyle, Binding, Color, DataStore, Fit as ModelFit, ImageSource, Layout, Rect,
    Resolved, TextSize, TimerFormat, Widget,
};
use chrono::{DateTime, Utc};
use image::{Rgba, RgbaImage};

use crate::text;
use crate::{fit as fit_image, Fit};

/// How much a stale or missing reading is dimmed.
const STALE_DIM: f32 = 0.35;

fn px(img: &mut RgbaImage, x: i64, y: i64, c: Color) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, Rgba([c.r, c.g, c.b, 255]));
    }
}

fn fill_rect(img: &mut RgbaImage, r: Rect, c: Color) {
    let r = r.clipped();
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            px(img, x as i64, y as i64, c);
        }
    }
}

fn stroke_rect(img: &mut RgbaImage, r: Rect, c: Color) {
    let r = r.clipped();
    if r.is_empty() {
        return;
    }
    for x in r.x..r.x + r.w {
        px(img, x as i64, r.y as i64, c);
        px(img, x as i64, (r.y + r.h - 1) as i64, c);
    }
    for y in r.y..r.y + r.h {
        px(img, r.x as i64, y as i64, c);
        px(img, (r.x + r.w - 1) as i64, y as i64, c);
    }
}

fn line(img: &mut RgbaImage, x0: i64, y0: i64, x1: i64, y1: i64, c: Color) {
    // Bresenham; the panel is small enough that anti-aliasing would only add fringe
    // pixels for RGB565 to mangle.
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y, mut err) = (x0, y0, dx + dy);
    loop {
        px(img, x, y, c);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn maybe_dim(c: Color, degraded: bool) -> Color {
    if degraded {
        c.dim(STALE_DIM)
    } else {
        c
    }
}

fn scale_for(size: TextSize) -> u32 {
    size.scale()
}

/// Render a layout to a full-panel RGBA image.
pub fn compose(layout: &Layout, store: &DataStore, now: DateTime<Utc>) -> RgbaImage {
    let bg = layout.background;
    let mut img = RgbaImage::from_pixel(
        catbus99_model::SCREEN_W,
        catbus99_model::SCREEN_H,
        Rgba([bg.r, bg.g, bg.b, 255]),
    );

    for slot in &layout.slots {
        let rect = slot.rect.clipped();
        if rect.is_empty() {
            continue;
        }
        if let Some(widget) = &slot.widget {
            draw_widget(&mut img, widget, rect, store, now);
        }
    }
    img
}

fn resolve(store: &DataStore, b: &Binding, now: DateTime<Utc>) -> Resolved {
    store.resolve(b, now)
}

fn draw_widget(
    img: &mut RgbaImage,
    widget: &Widget,
    rect: Rect,
    store: &DataStore,
    now: DateTime<Utc>,
) {
    match widget {
        Widget::Blank => {}

        Widget::Fill { color } => fill_rect(img, rect, *color),

        Widget::Label {
            text: binding,
            size,
            align,
            color,
        } => {
            let r = resolve(store, binding, now);
            let s = r.text_or_placeholder();
            let scale = text::fit_scale(&s, rect.w, scale_for(*size));
            text::draw_centered_in(
                img,
                &s,
                rect,
                scale,
                *align,
                maybe_dim(*color, r.degraded()),
            );
        }

        Widget::Clock {
            format,
            tz,
            quantize_minutes,
            size,
            align,
            color,
        } => {
            let s = format_clock(format, tz.as_deref(), now, *quantize_minutes);
            let scale = text::fit_scale(&s, rect.w, scale_for(*size));
            text::draw_centered_in(img, &s, rect, scale, *align, *color);
        }

        Widget::ProgressBar {
            value,
            label,
            style,
            color,
            track,
            show_value,
        } => {
            let r = resolve(store, value, now);
            let frac = r.number_or(0.0).clamp(0.0, 1.0);
            let degraded = r.degraded();

            // An optional label sits above the bar, taking one small line.
            let mut bar = rect;
            if let Some(lb) = label {
                let lr = resolve(store, lb, now);
                let lh = text::line_height(1);
                if rect.h > lh + 2 {
                    text::draw_aligned(
                        img,
                        &lr.text_or_placeholder(),
                        Rect::new(rect.x, rect.y, rect.w, lh),
                        1,
                        Align::Left,
                        maybe_dim(Color::WHITE, lr.degraded()),
                    );
                    bar = Rect::new(rect.x, rect.y + lh + 1, rect.w, rect.h - lh - 1);
                }
            }

            fill_rect(img, bar, maybe_dim(*track, degraded));
            let filled = ((bar.w as f64) * frac).round() as u32;

            match style {
                BarStyle::Solid => {
                    fill_rect(
                        img,
                        Rect::new(bar.x, bar.y, filled, bar.h),
                        maybe_dim(*color, degraded),
                    );
                }
                BarStyle::Segmented => {
                    // Discrete cells read more accurately at a glance than a continuous
                    // edge, which is hard to judge against a 160px width.
                    let (cell, gap) = (4u32, 1u32);
                    let mut x = bar.x;
                    while x + cell <= bar.x + bar.w {
                        if x + cell <= bar.x + filled {
                            fill_rect(
                                img,
                                Rect::new(x, bar.y, cell, bar.h),
                                maybe_dim(*color, degraded),
                            );
                        }
                        x += cell + gap;
                    }
                }
            }

            if *show_value && bar.h >= text::line_height(1) {
                let s = format!("{}%", (frac * 100.0).round() as i64);
                text::draw_centered_in(img, &s, bar, 1, Align::Center, Color::WHITE);
            }
        }

        Widget::Gauge {
            value,
            min,
            max,
            unit,
            color,
            track,
        } => {
            let r = resolve(store, value, now);
            let v = r.number_or(*min);
            let span = (max - min).abs().max(f64::EPSILON);
            let frac = ((v - min) / span).clamp(0.0, 1.0);
            let degraded = r.degraded();
            draw_gauge(
                img,
                rect,
                frac,
                maybe_dim(*color, degraded),
                maybe_dim(*track, degraded),
            );

            let s = match unit {
                Some(u) => format!("{}{}", fmt_num(v), u),
                None => fmt_num(v),
            };
            let scale = text::fit_scale(&s, rect.w, 1);
            let y = rect.y + rect.h.saturating_sub(text::line_height(scale));
            text::draw_aligned(
                img,
                &s,
                Rect::new(rect.x, y, rect.w, text::line_height(scale)),
                scale,
                Align::Center,
                maybe_dim(Color::WHITE, degraded),
            );
        }

        Widget::ResetTimer {
            deadline,
            label,
            format,
            quantize_minutes,
            color,
        } => {
            let r = resolve(store, deadline, now);
            let text_str = match r.value.as_ref().and_then(|v| v.as_timestamp()) {
                Some(dl) => format_remaining(dl - now, *format, *quantize_minutes),
                None => "--".to_string(),
            };
            let degraded =
                r.degraded() || r.value.as_ref().and_then(|v| v.as_timestamp()).is_none();

            let mut y = rect.y;
            if let Some(l) = label {
                text::draw_aligned(
                    img,
                    l,
                    Rect::new(rect.x, y, rect.w, text::line_height(1)),
                    1,
                    Align::Center,
                    Color::WHITE.dim(0.6),
                );
                y += text::line_height(1) + 1;
            }
            let avail = rect.h.saturating_sub(y - rect.y);
            let scale = text::fit_scale(&text_str, rect.w, 2);
            let ty = y + avail.saturating_sub(text::line_height(scale)) / 2;
            text::draw_aligned(
                img,
                &text_str,
                Rect::new(rect.x, ty, rect.w, text::line_height(scale)),
                scale,
                Align::Center,
                maybe_dim(*color, degraded),
            );
        }

        Widget::Sparkline { points, color } => draw_sparkline(img, rect, points, *color),

        Widget::Image { source, fit } => {
            if let Some(src) = load_image(source) {
                let mode = match fit {
                    ModelFit::Contain => Fit::Contain,
                    ModelFit::Cover => Fit::Cover,
                    ModelFit::Stretch => Fit::Stretch,
                };
                // Fit to the panel, then blit only the slot's region.
                let fitted = fit_image(&src, mode, [0, 0, 0, 255]);
                for y in 0..rect.h {
                    for x in 0..rect.w {
                        let sx = x * catbus99_model::SCREEN_W / rect.w.max(1);
                        let sy = y * catbus99_model::SCREEN_H / rect.h.max(1);
                        let p = fitted
                            .get_pixel(sx.min(fitted.width() - 1), sy.min(fitted.height() - 1));
                        px(
                            img,
                            (rect.x + x) as i64,
                            (rect.y + y) as i64,
                            Color::new(p.0[0], p.0[1], p.0[2]),
                        );
                    }
                }
            } else {
                stroke_rect(img, rect, Color::WHITE.dim(0.4));
                text::draw_centered_in(
                    img,
                    "no image",
                    rect,
                    1,
                    Align::Center,
                    Color::WHITE.dim(0.5),
                );
            }
        }
    }
}

fn load_image(source: &ImageSource) -> Option<RgbaImage> {
    match source {
        ImageSource::Path { path } => image::open(path).ok().map(|i| i.to_rgba8()),
        ImageSource::Inline { .. } => None,
    }
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// A 240-degree arc gauge with a needle, the widest sweep that still reads unambiguously.
fn draw_gauge(img: &mut RgbaImage, rect: Rect, frac: f64, color: Color, track: Color) {
    let cx = rect.x as f64 + rect.w as f64 / 2.0;
    let cy = rect.y as f64 + rect.h as f64 * 0.62;
    let radius = (rect.w.min(rect.h) as f64 / 2.0 - 2.0).max(3.0);

    let start = 150.0f64.to_radians();
    let sweep = 240.0f64.to_radians();
    let steps = 96;

    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let a = start + sweep * t;
        let c = if t <= frac { color } else { track };
        for rr in [radius, radius - 1.0] {
            px(
                img,
                (cx + a.cos() * rr).round() as i64,
                (cy + a.sin() * rr).round() as i64,
                c,
            );
        }
    }

    let a = start + sweep * frac.clamp(0.0, 1.0);
    line(
        img,
        cx.round() as i64,
        cy.round() as i64,
        (cx + a.cos() * (radius - 3.0)).round() as i64,
        (cy + a.sin() * (radius - 3.0)).round() as i64,
        color,
    );
}

fn draw_sparkline(img: &mut RgbaImage, rect: Rect, points: &[f64], color: Color) {
    if points.len() < 2 {
        return;
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &p in points {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    // A flat series would divide by zero; draw it as a centred line instead.
    let span = if (hi - lo).abs() < f64::EPSILON {
        1.0
    } else {
        hi - lo
    };

    let to_xy = |i: usize, v: f64| -> (i64, i64) {
        let x = rect.x as f64 + (i as f64 / (points.len() - 1) as f64) * (rect.w - 1) as f64;
        let y = rect.y as f64 + (1.0 - (v - lo) / span) * (rect.h - 1) as f64;
        (x.round() as i64, y.round() as i64)
    };

    for i in 1..points.len() {
        let (x0, y0) = to_xy(i - 1, points[i - 1]);
        let (x1, y1) = to_xy(i, points[i]);
        line(img, x0, y0, x1, y1, color);
    }
}

/// Format the clock, rounded down to `quantize` minutes.
///
/// Quantisation happens here rather than at display time so the *rendered pixels* are
/// stable between steps -- that is what lets the write governor skip the upload.
fn format_clock(format: &str, tz: Option<&str>, now: DateTime<Utc>, quantize: u32) -> String {
    match tz.and_then(|t| t.parse::<chrono_tz::Tz>().ok()) {
        Some(zone) => catbus99_model::quantize_time(now.with_timezone(&zone), quantize)
            .format(format)
            .to_string(),
        None => catbus99_model::quantize_time(now.with_timezone(&chrono::Local), quantize)
            .format(format)
            .to_string(),
    }
}

fn format_remaining(delta: chrono::TimeDelta, format: TimerFormat, quantize: u32) -> String {
    let step = (quantize.max(1) as i64) * 60;
    // Round the remaining time down to whole quantisation steps so the rendering only
    // changes once per step, not once per second.
    let secs = (delta.num_seconds() / step) * step;
    if secs <= 0 {
        return "now".to_string();
    }
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    match format {
        TimerFormat::Clock => format!("{h:02}:{m:02}:{s:02}"),
        TimerFormat::Minutes => format!("{}m", secs / 60),
        TimerFormat::Compact => {
            if h > 0 {
                format!("{h}h {m}m")
            } else if m > 0 {
                format!("{m}m")
            } else {
                format!("{s}s")
            }
        }
    }
}
