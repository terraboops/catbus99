//! The screen model: serialisation, binding resolution, staleness, and layout linting.

use catbus99_model::*;
use chrono::{Duration, Utc};

fn point(key: &str, value: Value, ttl: Option<u64>, age_secs: i64) -> DataPoint {
    DataPoint {
        source: "s".into(),
        key: key.into(),
        value,
        unit: None,
        label: None,
        observed_at: Utc::now() - Duration::seconds(age_secs),
        ttl_secs: ttl,
    }
}

#[test]
fn colours_round_trip_as_hex() {
    let c = Color::new(0x4A, 0xC8, 0xFF);
    let json = serde_json::to_string(&c).unwrap();
    assert_eq!(json, "\"#4ac8ff\"");
    assert_eq!(serde_json::from_str::<Color>(&json).unwrap(), c);
}

#[test]
fn short_hex_colours_expand() {
    assert_eq!(
        serde_json::from_str::<Color>("\"#f0a\"").unwrap(),
        Color::new(0xFF, 0x00, 0xAA)
    );
}

#[test]
fn bad_colours_are_rejected() {
    assert!(serde_json::from_str::<Color>("\"#12345\"").is_err());
    assert!(serde_json::from_str::<Color>("\"nope\"").is_err());
}

#[test]
fn rects_clip_to_the_panel() {
    let r = Rect::new(150, 90, 100, 100).clipped();
    assert_eq!(r, Rect::new(150, 90, 10, 6));
    assert!(Rect::new(0, 0, 0, 10).is_empty());
}

/// A missing reading must not silently render as zero -- on a glanceable display that
/// looks like a real measurement.
#[test]
fn a_missing_point_is_reported_missing() {
    let store = DataStore::new();
    let r = store.resolve(
        &Binding::DataPoint {
            source: "s".into(),
            key: "gone".into(),
            scale: None,
        },
        Utc::now(),
    );
    assert!(r.missing);
    assert!(r.degraded());
    assert_eq!(r.text_or_placeholder(), "--");
}

#[test]
fn a_point_past_its_ttl_is_stale() {
    let mut store = DataStore::new();
    store.insert(point("x", Value::Number(1.0), Some(60), 120));
    let r = store.resolve(
        &Binding::DataPoint {
            source: "s".into(),
            key: "x".into(),
            scale: None,
        },
        Utc::now(),
    );
    assert!(r.stale);
    assert!(r.degraded());
    // The value is still available -- widgets choose to dim it rather than hide it.
    assert_eq!(r.number_or(0.0), 1.0);
}

#[test]
fn a_fresh_point_is_not_degraded() {
    let mut store = DataStore::new();
    store.insert(point("x", Value::Number(0.5), Some(300), 5));
    let r = store.resolve(
        &Binding::DataPoint {
            source: "s".into(),
            key: "x".into(),
            scale: None,
        },
        Utc::now(),
    );
    assert!(!r.degraded());
}

#[test]
fn a_point_without_ttl_never_goes_stale() {
    let p = point("x", Value::Number(1.0), None, 10_000_000);
    assert!(!p.is_stale(Utc::now()));
}

#[test]
fn literals_are_never_degraded() {
    let store = DataStore::new();
    let r = store.resolve(&Binding::literal_number(0.4), Utc::now());
    assert!(!r.degraded());
    assert_eq!(r.number_or(0.0), 0.4);
}

#[test]
fn scale_is_applied_to_numeric_points() {
    let mut store = DataStore::new();
    store.insert(point("pct", Value::Number(0.42), None, 0));
    let r = store.resolve(
        &Binding::DataPoint {
            source: "s".into(),
            key: "pct".into(),
            scale: Some(100.0),
        },
        Utc::now(),
    );
    assert_eq!(r.number_or(0.0), 42.0);
}

#[test]
fn values_coerce_sensibly() {
    assert_eq!(Value::Text("3.5".into()).as_number(), Some(3.5));
    assert_eq!(Value::Bool(true).as_number(), Some(1.0));
    assert_eq!(Value::Number(4.0).as_display(), "4");
    assert_eq!(Value::Number(4.25).as_display(), "4.2");
    assert!(Value::Text("2026-08-18T20:00:00Z".into())
        .as_timestamp()
        .is_some());
}

#[test]
fn lint_flags_duplicate_slot_ids() {
    let l = Layout::new("t")
        .with_slot("a", Rect::new(0, 0, 10, 10), Widget::Blank)
        .with_slot("a", Rect::new(20, 20, 10, 10), Widget::Blank);
    assert!(l.lint().iter().any(|p| p.contains("reuses id")));
}

#[test]
fn lint_flags_overlaps_and_overflow() {
    let l = Layout::new("t")
        .with_slot("a", Rect::new(0, 0, 50, 50), Widget::Blank)
        .with_slot("b", Rect::new(40, 40, 50, 50), Widget::Blank)
        .with_slot("c", Rect::new(140, 90, 40, 40), Widget::Blank);
    let problems = l.lint();
    assert!(problems.iter().any(|p| p.contains("overlap")));
    assert!(problems.iter().any(|p| p.contains("past the")));
}

#[test]
fn a_clean_layout_lints_clean() {
    let l = Layout::new("t")
        .with_slot("a", Rect::new(0, 0, 50, 40), Widget::Blank)
        .with_slot("b", Rect::new(60, 0, 50, 40), Widget::Blank);
    assert!(l.lint().is_empty(), "{:?}", l.lint());
}

#[test]
fn layouts_round_trip_through_json() {
    let l = Layout::new("t").with_slot(
        "bar",
        Rect::new(1, 2, 100, 10),
        Widget::ProgressBar {
            value: Binding::literal_number(0.5),
            label: Some(Binding::literal_text("X")),
            style: BarStyle::Segmented,
            color: Color::WHITE,
            track: Color::BLACK,
            show_value: true,
        },
    );
    let json = serde_json::to_string(&l).unwrap();
    assert_eq!(serde_json::from_str::<Layout>(&json).unwrap(), l);
}

/// The MCP tools advertise these schemas, so they must actually generate.
#[test]
fn schemas_generate_for_layout_and_widget() {
    for schema in [layout_schema(), widget_schema()] {
        assert!(schema.is_object());
        assert!(!schema.to_string().is_empty());
    }
}

#[test]
fn dim_darkens_without_changing_hue_order() {
    let c = Color::new(200, 100, 50).dim(0.5);
    assert_eq!((c.r, c.g, c.b), (100, 50, 25));
}

// --- time quantisation: a flash-endurance control, not a formatting preference ---

#[test]
fn quantising_rounds_down_to_the_step() {
    use chrono::TimeZone;
    let t = |m, s| Utc.with_ymd_and_hms(2026, 8, 18, 20, m, s).unwrap();
    assert_eq!(quantize_time(t(17, 42), 15), t(15, 0));
    assert_eq!(quantize_time(t(0, 1), 15), t(0, 0));
    assert_eq!(quantize_time(t(59, 59), 15), t(45, 0));
    assert_eq!(quantize_time(t(7, 30), 5), t(5, 0));
}

#[test]
fn quantising_by_one_or_zero_is_a_noop_on_minutes() {
    use chrono::TimeZone;
    let t = Utc.with_ymd_and_hms(2026, 8, 18, 20, 17, 42).unwrap();
    assert_eq!(quantize_time(t, 1), t);
    assert_eq!(quantize_time(t, 0), t);
}

/// Everything inside one step must render identically, or the governor cannot skip the
/// upload and the endurance saving evaporates.
#[test]
fn all_times_within_a_step_quantise_identically() {
    use chrono::TimeZone;
    let base = Utc.with_ymd_and_hms(2026, 8, 18, 20, 15, 0).unwrap();
    for minute in 15..30 {
        for second in [0, 17, 59] {
            let t = Utc
                .with_ymd_and_hms(2026, 8, 18, 20, minute, second)
                .unwrap();
            assert_eq!(
                quantize_time(t, 15),
                base,
                "{minute}:{second} escaped its step"
            );
        }
    }
}

#[test]
fn write_rate_matches_the_documented_budget_table() {
    assert_eq!(writes_per_day(1), 1440.0);
    assert_eq!(writes_per_day(5), 288.0);
    assert_eq!(writes_per_day(15), 96.0);
    assert_eq!(writes_per_day(30), 48.0);
}

#[test]
fn time_widgets_default_to_the_safe_resolution() {
    // A clock that defaults to 1-minute resolution would exhaust the panel in ~69 days.
    let w: Widget = serde_json::from_str(r#"{"type":"clock"}"#).unwrap();
    match w {
        Widget::Clock {
            quantize_minutes, ..
        } => assert_eq!(quantize_minutes, 15),
        other => panic!("expected a clock, got {other:?}"),
    }
}
