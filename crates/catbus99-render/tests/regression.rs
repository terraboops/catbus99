//! Visual regression harness.
//!
//! Each scene renders a layout at a **fixed timestamp** with **fixed data** and is compared
//! byte-for-byte against a committed golden PNG. Rendering is fully deterministic, so an
//! exact comparison is correct here and strictly stronger than a perceptual threshold: it
//! catches a single stray pixel, which at 160x96 is a real defect.
//!
//! # Regenerating
//!
//! ```sh
//! CATBUS99_BLESS=1 cargo test -p catbus99-render --test regression
//! ```
//!
//! Blessing rewrites every golden, so **review the diff before committing**. A blessed
//! regression is indistinguishable from a blessed fix in the fixture alone; the point of
//! the harness is that the change shows up in review.
//!
//! On failure the harness writes `<scene>.actual.png` and `<scene>.diff.png` beside the
//! golden — the diff highlights changed pixels in magenta so the regression is visible
//! rather than merely reported.

use catbus99_model::*;
use catbus99_render::{compose, rgb565_to_rgba, to_rgb565};
use chrono::{DateTime, TimeZone, Utc};
use image::{Rgba, RgbaImage};
use std::path::PathBuf;

/// Fixed render instant, so clocks and timers are stable across runs.
fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 19, 14, 37, 12).unwrap()
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn store(points: &[(&str, Value, Option<u64>, i64)]) -> DataStore {
    let mut s = DataStore::new();
    for (key, value, ttl, age) in points {
        s.insert(DataPoint {
            source: "s".into(),
            key: (*key).into(),
            value: value.clone(),
            unit: None,
            label: None,
            observed_at: at() - chrono::Duration::seconds(*age),
            ttl_secs: *ttl,
        });
    }
    s
}

fn bind(key: &str) -> Binding {
    Binding::DataPoint {
        source: "s".into(),
        key: key.into(),
        scale: None,
    }
}

fn render(layout: &Layout, data: &DataStore) -> RgbaImage {
    // Through RGB565 and back, so the golden reflects what the panel actually shows,
    // including the 5/6/5 quantisation -- not the full-colour intermediate.
    rgb565_to_rgba(&to_rgb565(&compose(layout, data, at()), false))
}

/// Compare against the golden, or write it when blessing.
fn check(name: &str, img: &RgbaImage) {
    let golden = fixtures().join(format!("{name}.png"));

    if std::env::var("CATBUS99_BLESS").is_ok() {
        std::fs::create_dir_all(fixtures()).unwrap();
        img.save(&golden).unwrap();
        eprintln!("blessed {name}");
        return;
    }

    let expected = image::open(&golden)
        .unwrap_or_else(|e| {
            panic!(
                "missing golden for {name} ({e}). Generate it with \
                 CATBUS99_BLESS=1 cargo test -p catbus99-render --test regression"
            )
        })
        .to_rgba8();

    assert_eq!(
        expected.dimensions(),
        img.dimensions(),
        "{name}: size changed"
    );

    let mut diff_count = 0usize;
    let mut diff_img = img.clone();
    let mut first: Option<(u32, u32)> = None;
    for y in 0..img.height() {
        for x in 0..img.width() {
            if img.get_pixel(x, y) != expected.get_pixel(x, y) {
                diff_count += 1;
                first.get_or_insert((x, y));
                diff_img.put_pixel(x, y, Rgba([255, 0, 255, 255]));
            }
        }
    }

    if diff_count > 0 {
        let actual_path = fixtures().join(format!("{name}.actual.png"));
        let diff_path = fixtures().join(format!("{name}.diff.png"));
        img.save(&actual_path).ok();
        diff_img.save(&diff_path).ok();
        let (x, y) = first.unwrap();
        panic!(
            "{name}: {diff_count} pixel(s) differ (first at {x},{y}).\n  \
             actual: {}\n  diff:   {} (changed pixels in magenta)\n  \
             If the change is intended: CATBUS99_BLESS=1 cargo test -p catbus99-render --test regression",
            actual_path.display(),
            diff_path.display()
        );
    }
}

// --- scenes ---

fn scene_widget_gallery() -> (Layout, DataStore) {
    let layout = Layout {
        id: "gallery".into(),
        name: "every widget".into(),
        background: Color::new(0x0A, 0x0C, 0x10),
        slots: vec![],
    }
    .with_slot(
        "label",
        Rect::new(2, 1, 96, 8),
        Widget::Label {
            text: Binding::literal_text("CATBUS99"),
            size: TextSize::Small,
            align: Align::Left,
            color: Color::new(0x4A, 0xC8, 0xFF),
        },
    )
    .with_slot(
        "clock",
        Rect::new(100, 1, 58, 8),
        Widget::Clock {
            format: "%H:%M".into(),
            tz: Some("UTC".into()),
            quantize_minutes: 15,
            size: TextSize::Small,
            align: Align::Right,
            color: Color::WHITE,
        },
    )
    .with_slot(
        "bar",
        Rect::new(2, 12, 156, 17),
        Widget::ProgressBar {
            value: bind("session"),
            label: Some(Binding::literal_text("SESSION")),
            style: BarStyle::Solid,
            color: Color::new(0x4A, 0xC8, 0xFF),
            track: Color::new(0x1A, 0x20, 0x28),
            show_value: true,
        },
    )
    .with_slot(
        "seg",
        Rect::new(2, 31, 156, 17),
        Widget::ProgressBar {
            value: bind("weekly"),
            label: Some(Binding::literal_text("WEEKLY")),
            style: BarStyle::Segmented,
            color: Color::new(0xFF, 0xA5, 0x3A),
            track: Color::new(0x1A, 0x20, 0x28),
            show_value: false,
        },
    )
    .with_slot(
        "gauge",
        Rect::new(2, 51, 46, 44),
        Widget::Gauge {
            value: bind("cpu"),
            min: 0.0,
            max: 100.0,
            unit: Some("%".into()),
            color: Color::new(0x6E, 0xE7, 0xB7),
            track: Color::new(0x1A, 0x20, 0x28),
        },
    )
    .with_slot(
        "timer",
        Rect::new(52, 51, 54, 44),
        Widget::ResetTimer {
            deadline: bind("resets_at"),
            label: Some("RESETS".into()),
            format: TimerFormat::Compact,
            quantize_minutes: 15,
            color: Color::WHITE,
        },
    )
    .with_slot(
        "spark",
        Rect::new(110, 51, 48, 44),
        Widget::Sparkline {
            points: vec![
                3.0, 5.0, 4.0, 8.0, 6.0, 9.0, 7.0, 12.0, 10.0, 14.0, 11.0, 15.0,
            ],
            color: Color::new(0xC0, 0x8C, 0xFF),
        },
    );

    let data = store(&[
        ("session", Value::Number(0.62), Some(300), 0),
        ("weekly", Value::Number(0.35), Some(300), 0),
        ("cpu", Value::Number(73.0), Some(300), 0),
        (
            "resets_at",
            Value::Timestamp(at() + chrono::Duration::minutes(134)),
            Some(300),
            0,
        ),
    ]);
    (layout, data)
}

#[test]
fn widget_gallery() {
    let (l, d) = scene_widget_gallery();
    check("widget_gallery", &render(&l, &d));
}

/// Stale and missing data must remain *visibly* distinct from fresh data. This is the
/// scene most likely to regress silently, because the code path differs only by a colour
/// multiply and a placeholder string.
#[test]
fn degraded_data() {
    let (l, _) = scene_widget_gallery();
    let data = store(&[
        ("session", Value::Number(0.62), Some(60), 600), // stale
        ("cpu", Value::Number(73.0), Some(60), 600),     // stale
                                                         // "weekly" and "resets_at" are absent entirely.
    ]);
    check("degraded_data", &render(&l, &data));
}

#[test]
fn text_sizes_and_alignment() {
    let mut layout = Layout::new("text");
    layout.background = Color::BLACK;
    let layout = layout
        .with_slot(
            "s",
            Rect::new(0, 0, 160, 10),
            Widget::Label {
                text: Binding::literal_text("SMALL LEFT"),
                size: TextSize::Small,
                align: Align::Left,
                color: Color::WHITE,
            },
        )
        .with_slot(
            "m",
            Rect::new(0, 12, 160, 18),
            Widget::Label {
                text: Binding::literal_text("MED CENTER"),
                size: TextSize::Medium,
                align: Align::Center,
                color: Color::new(0x4A, 0xC8, 0xFF),
            },
        )
        .with_slot(
            "l",
            Rect::new(0, 32, 160, 26),
            Widget::Label {
                text: Binding::literal_text("LARGE"),
                size: TextSize::Large,
                align: Align::Right,
                color: Color::new(0x6E, 0xE7, 0xB7),
            },
        )
        .with_slot(
            "punct",
            Rect::new(0, 60, 160, 10),
            Widget::Label {
                text: Binding::literal_text("0123456789 %:.-/"),
                size: TextSize::Small,
                align: Align::Left,
                color: Color::WHITE,
            },
        )
        .with_slot(
            "shrink",
            Rect::new(0, 72, 60, 20),
            Widget::Label {
                // Too long for the slot at Large: must shrink rather than overflow.
                text: Binding::literal_text("SHRINKS TO FIT"),
                size: TextSize::Large,
                align: Align::Left,
                color: Color::new(0xFF, 0xA5, 0x3A),
            },
        );
    check("text_sizes", &render(&layout, &DataStore::new()));
}

/// Bar fills at the extremes are where off-by-one errors live.
#[test]
fn progress_bar_extremes() {
    let mut layout = Layout::new("bars");
    layout.background = Color::new(0x08, 0x08, 0x08);
    for (i, v) in [0.0f64, 0.01, 0.5, 0.99, 1.0].iter().enumerate() {
        layout = layout.with_slot(
            format!("b{i}"),
            Rect::new(2, 2 + i as u32 * 19, 156, 16),
            Widget::ProgressBar {
                value: Binding::literal_number(*v),
                label: None,
                style: if i % 2 == 0 {
                    BarStyle::Solid
                } else {
                    BarStyle::Segmented
                },
                color: Color::new(0x4A, 0xC8, 0xFF),
                track: Color::new(0x22, 0x2A, 0x33),
                show_value: true,
            },
        );
    }
    check("bar_extremes", &render(&layout, &DataStore::new()));
}

/// Degenerate inputs must render something sane rather than panicking or drawing nothing.
#[test]
fn degenerate_inputs() {
    let mut layout = Layout::new("edge");
    layout.background = Color::new(0x10, 0x00, 0x10);
    let layout = layout
        // Zero-span gauge.
        .with_slot(
            "g",
            Rect::new(0, 0, 48, 48),
            Widget::Gauge {
                value: Binding::literal_number(5.0),
                min: 3.0,
                max: 3.0,
                unit: None,
                color: Color::WHITE,
                track: Color::new(0x30, 0x30, 0x30),
            },
        )
        // Flat sparkline.
        .with_slot(
            "sp",
            Rect::new(50, 0, 50, 48),
            Widget::Sparkline {
                points: vec![5.0; 10],
                color: Color::new(0x6E, 0xE7, 0xB7),
            },
        )
        // Already-elapsed timer.
        .with_slot(
            "t",
            Rect::new(102, 0, 58, 48),
            Widget::ResetTimer {
                deadline: Binding::Literal {
                    value: Value::Timestamp(at() - chrono::Duration::hours(3)),
                },
                label: None,
                format: TimerFormat::Clock,
                quantize_minutes: 15,
                color: Color::WHITE,
            },
        )
        // Missing image.
        .with_slot(
            "img",
            Rect::new(0, 50, 78, 44),
            Widget::Image {
                source: ImageSource::Path {
                    path: "/definitely/missing.png".into(),
                },
                fit: Fit::Contain,
            },
        )
        // Slot extending past the panel edge: must clip, not panic.
        .with_slot(
            "oob",
            Rect::new(120, 60, 200, 200),
            Widget::Fill {
                color: Color::new(0xFF, 0xA5, 0x3A),
            },
        );
    check("degenerate", &render(&layout, &DataStore::new()));
}

/// The RGB565 round trip: primaries and near-black/near-white are where quantisation and
/// any future dithering change would show first.
#[test]
fn colour_quantisation() {
    let mut layout = Layout::new("colour");
    layout.background = Color::BLACK;
    #[allow(clippy::items_after_statements)]
    let swatches = [
        Color::new(255, 0, 0),
        Color::new(0, 255, 0),
        Color::new(0, 0, 255),
        Color::new(255, 255, 255),
        Color::new(1, 1, 1),
        Color::new(254, 254, 254),
        Color::new(0x4A, 0xC8, 0xFF),
        Color::new(0xFF, 0xA5, 0x3A),
    ];
    for (i, c) in swatches.iter().enumerate() {
        let (col, row) = (i as u32 % 4, i as u32 / 4);
        layout = layout.with_slot(
            format!("c{i}"),
            Rect::new(col * 40, row * 48, 40, 48),
            Widget::Fill { color: *c },
        );
    }
    check("colour_quantisation", &render(&layout, &DataStore::new()));
}
