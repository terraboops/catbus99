//! Compositing: bounds safety, degraded-data treatment, and widget behaviour.

use catbus99_model::*;
use catbus99_render::compose;
use chrono::{Duration, Utc};
use image::Rgba;

const BG: Rgba<u8> = Rgba([0, 0, 0, 255]);

fn store_with(key: &str, value: Value, ttl: Option<u64>, age: i64) -> DataStore {
    let mut s = DataStore::new();
    s.insert(DataPoint {
        source: "s".into(),
        key: key.into(),
        value,
        unit: None,
        label: None,
        observed_at: Utc::now() - Duration::seconds(age),
        ttl_secs: ttl,
    });
    s
}

fn bind(key: &str) -> Binding {
    Binding::DataPoint {
        source: "s".into(),
        key: key.into(),
        scale: None,
    }
}

fn drawn(img: &image::RgbaImage) -> usize {
    img.pixels().filter(|p| **p != BG).count()
}

fn bar_layout(value: Binding, style: BarStyle) -> Layout {
    Layout::new("b").with_slot(
        "bar",
        Rect::new(0, 0, 100, 10),
        Widget::ProgressBar {
            value,
            label: None,
            style,
            color: Color::WHITE,
            track: Color::BLACK,
            show_value: false,
        },
    )
}

#[test]
fn output_is_always_panel_sized() {
    let img = compose(&Layout::new("empty"), &DataStore::new(), Utc::now());
    assert_eq!(img.dimensions(), (SCREEN_W, SCREEN_H));
}

#[test]
fn background_fills_an_empty_layout() {
    let mut l = Layout::new("bg");
    l.background = Color::new(10, 20, 30);
    let img = compose(&l, &DataStore::new(), Utc::now());
    assert!(img.pixels().all(|p| *p == Rgba([10, 20, 30, 255])));
}

/// A malformed layout may look wrong, but must never write out of bounds or panic.
#[test]
fn out_of_bounds_slots_are_clipped_not_fatal() {
    let l = Layout::new("oob")
        .with_slot(
            "far",
            Rect::new(150, 90, 400, 400),
            Widget::Fill {
                color: Color::WHITE,
            },
        )
        .with_slot(
            "way",
            Rect::new(1000, 1000, 10, 10),
            Widget::Fill {
                color: Color::WHITE,
            },
        );
    assert_eq!(
        compose(&l, &DataStore::new(), Utc::now()).dimensions(),
        (SCREEN_W, SCREEN_H)
    );
}

#[test]
fn later_slots_paint_over_earlier_ones() {
    let l = Layout::new("z")
        .with_slot(
            "under",
            Rect::new(0, 0, 40, 40),
            Widget::Fill {
                color: Color::new(255, 0, 0),
            },
        )
        .with_slot(
            "over",
            Rect::new(0, 0, 40, 40),
            Widget::Fill {
                color: Color::new(0, 255, 0),
            },
        );
    assert_eq!(
        *compose(&l, &DataStore::new(), Utc::now()).get_pixel(10, 10),
        Rgba([0, 255, 0, 255])
    );
}

#[test]
fn a_progress_bar_fills_proportionally() {
    let filled = |v: f64| {
        let img = compose(
            &bar_layout(Binding::literal_number(v), BarStyle::Solid),
            &DataStore::new(),
            Utc::now(),
        );
        (0..100)
            .filter(|&x| *img.get_pixel(x, 5) == Rgba([255, 255, 255, 255]))
            .count()
    };
    assert_eq!(filled(0.0), 0);
    assert_eq!(filled(1.0), 100);
    assert_eq!(filled(0.5), 50);
}

#[test]
fn progress_values_outside_zero_to_one_are_clamped() {
    for v in [-5.0, 5.0, f64::NAN] {
        let img = compose(
            &bar_layout(Binding::literal_number(v), BarStyle::Solid),
            &DataStore::new(),
            Utc::now(),
        );
        assert_eq!(img.dimensions(), (SCREEN_W, SCREEN_H));
    }
}

#[test]
fn a_segmented_bar_draws_discrete_cells() {
    let img = compose(
        &bar_layout(Binding::literal_number(1.0), BarStyle::Segmented),
        &DataStore::new(),
        Utc::now(),
    );
    let row: Vec<bool> = (0..100)
        .map(|x| *img.get_pixel(x, 5) == Rgba([255, 255, 255, 255]))
        .collect();
    // Gaps between cells mean the row is not a single unbroken run.
    let transitions = row.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(
        transitions > 2,
        "expected discrete cells, got {transitions} transitions"
    );
}

/// Stale data must look different from fresh, or a glanceable display quietly lies.
#[test]
fn stale_data_renders_dimmer_than_fresh() {
    let layout = bar_layout(bind("v"), BarStyle::Solid);
    let now = Utc::now();
    let bright = |age| {
        compose(
            &layout,
            &store_with("v", Value::Number(1.0), Some(60), age),
            now,
        )
        .get_pixel(50, 5)
        .0[0] as u32
    };
    assert!(bright(600) < bright(0), "stale should be dimmer than fresh");
}

#[test]
fn a_missing_binding_draws_a_placeholder() {
    let l = Layout::new("m").with_slot(
        "lbl",
        Rect::new(0, 0, 160, 20),
        Widget::Label {
            text: bind("absent"),
            size: TextSize::Medium,
            align: Align::Left,
            color: Color::WHITE,
        },
    );
    assert!(drawn(&compose(&l, &DataStore::new(), Utc::now())) > 0);
}

#[test]
fn a_flat_sparkline_does_not_divide_by_zero() {
    let l = Layout::new("f").with_slot(
        "sp",
        Rect::new(0, 0, 60, 30),
        Widget::Sparkline {
            points: vec![5.0; 8],
            color: Color::WHITE,
        },
    );
    assert!(drawn(&compose(&l, &DataStore::new(), Utc::now())) > 0);
}

#[test]
fn a_sparkline_needs_at_least_two_points() {
    for pts in [vec![], vec![1.0]] {
        let l = Layout::new("f").with_slot(
            "sp",
            Rect::new(0, 0, 60, 30),
            Widget::Sparkline {
                points: pts,
                color: Color::WHITE,
            },
        );
        assert_eq!(drawn(&compose(&l, &DataStore::new(), Utc::now())), 0);
    }
}

#[test]
fn an_elapsed_timer_reads_now_rather_than_negative() {
    let l = Layout::new("t").with_slot(
        "tm",
        Rect::new(0, 0, 160, 40),
        Widget::ResetTimer {
            deadline: Binding::Literal {
                value: Value::Timestamp(Utc::now() - Duration::hours(2)),
            },
            label: None,
            format: TimerFormat::Compact,
            quantize_minutes: 15,
            color: Color::WHITE,
        },
    );
    assert!(drawn(&compose(&l, &DataStore::new(), Utc::now())) > 0);
}

#[test]
fn a_gauge_survives_a_degenerate_range() {
    let l = Layout::new("g").with_slot(
        "gg",
        Rect::new(0, 0, 48, 48),
        Widget::Gauge {
            value: Binding::literal_number(5.0),
            min: 3.0,
            max: 3.0,
            unit: None,
            color: Color::WHITE,
            track: Color::BLACK,
        },
    );
    assert_eq!(
        compose(&l, &DataStore::new(), Utc::now()).dimensions(),
        (SCREEN_W, SCREEN_H)
    );
}

#[test]
fn a_missing_image_draws_a_visible_placeholder() {
    let l = Layout::new("i").with_slot(
        "img",
        Rect::new(0, 0, 80, 60),
        Widget::Image {
            source: ImageSource::Path {
                path: "/nope/missing.png".into(),
            },
            fit: Fit::Contain,
        },
    );
    assert!(drawn(&compose(&l, &DataStore::new(), Utc::now())) > 0);
}

/// The whole endurance argument rests on this: within one quantisation step the rendered
/// pixels must be byte-identical, so the write governor can skip the upload.
#[test]
fn a_quantised_clock_renders_identically_within_a_step() {
    use chrono::TimeZone;
    let layout = Layout::new("c").with_slot(
        "clk",
        Rect::new(0, 0, 160, 20),
        Widget::Clock {
            format: "%H:%M".into(),
            tz: Some("UTC".into()),
            quantize_minutes: 15,
            size: TextSize::Medium,
            align: Align::Center,
            color: Color::WHITE,
        },
    );
    let render = |min, sec| {
        let t = Utc.with_ymd_and_hms(2026, 8, 18, 20, min, sec).unwrap();
        compose(&layout, &DataStore::new(), t).into_raw()
    };
    let base = render(15, 0);
    for (m, s) in [(15u32, 30u32), (20, 0), (29, 59)] {
        assert_eq!(
            render(m, s),
            base,
            "{m}:{s} rendered differently within its step"
        );
    }
    // The next step must actually differ, or the clock would never update at all.
    assert_ne!(render(30, 0), base);
}

#[test]
fn a_quantised_reset_timer_is_stable_within_a_step() {
    let deadline = Utc::now() + Duration::minutes(134);
    let layout = Layout::new("t").with_slot(
        "tm",
        Rect::new(0, 0, 160, 30),
        Widget::ResetTimer {
            deadline: Binding::Literal {
                value: Value::Timestamp(deadline),
            },
            label: None,
            format: TimerFormat::Compact,
            quantize_minutes: 15,
            color: Color::WHITE,
        },
    );
    let at = |offset_s| {
        compose(
            &layout,
            &DataStore::new(),
            Utc::now() + Duration::seconds(offset_s),
        )
        .into_raw()
    };
    assert_eq!(
        at(0),
        at(60),
        "a minute of drift must not change the rendering"
    );
}
