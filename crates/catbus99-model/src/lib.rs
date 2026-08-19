//! The typed screen model: layouts, slots, widgets, and the data they bind to.
//!
//! Everything here derives both [`serde::Serialize`]/[`Deserialize`] and
//! [`schemars::JsonSchema`], so the same definitions serve three purposes: the on-disk
//! layout format, the daemon's control protocol, and the JSON Schemas advertised by the
//! MCP tools.
//!
//! # Why widgets bind rather than embed
//!
//! A widget holds a [`Binding`] — a *reference* to a data point — not a snapshot of its
//! value. That is what lets one `Layout` be rendered several ways: to the panel, to a PNG
//! preview, or to a terminal dump, all from the same declaration, with values resolved at
//! render time from whatever the data store currently holds.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const SCREEN_W: u32 = 160;
pub const SCREEN_H: u32 = 96;

/// An 8-bit RGB colour. Serialises as `"#rrggbb"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, JsonSchema)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
    };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Blend towards black; used to render stale data without changing its hue.
    pub fn dim(self, factor: f32) -> Self {
        let f = factor.clamp(0.0, 1.0);
        Self {
            r: (self.r as f32 * f) as u8,
            g: (self.g as f32 * f) as u8,
            b: (self.b as f32 * f) as u8,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::WHITE
    }
}

impl Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b))
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let s = String::deserialize(d)?;
        let h = s.trim_start_matches('#');
        let parse = |a: usize, b: usize| u8::from_str_radix(&h[a..b], 16);
        match h.len() {
            6 => Ok(Color {
                r: parse(0, 2).map_err(D::Error::custom)?,
                g: parse(2, 4).map_err(D::Error::custom)?,
                b: parse(4, 6).map_err(D::Error::custom)?,
            }),
            3 => {
                let d1 = |i: usize| {
                    u8::from_str_radix(&h[i..i + 1], 16)
                        .map(|v| v * 17)
                        .map_err(D::Error::custom)
                };
                Ok(Color {
                    r: d1(0)?,
                    g: d1(1)?,
                    b: d1(2)?,
                })
            }
            _ => Err(D::Error::custom(format!(
                "bad colour {s:?}, want #rgb or #rrggbb"
            ))),
        }
    }
}

/// A rectangle in panel pixels, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }
    pub const FULL: Rect = Rect {
        x: 0,
        y: 0,
        w: SCREEN_W,
        h: SCREEN_H,
    };

    /// Clip to the panel, so a bad layout cannot draw out of bounds.
    pub fn clipped(self) -> Rect {
        let x = self.x.min(SCREEN_W);
        let y = self.y.min(SCREEN_H);
        Rect {
            x,
            y,
            w: self.w.min(SCREEN_W - x),
            h: self.h.min(SCREEN_H - y),
        }
    }

    pub fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    #[default]
    Contain,
    Cover,
    Stretch,
}

/// Text size as an integer multiple of the bundled 8px pixel font.
///
/// Integer scaling only: fractional scaling of a pixel font produces uneven stem widths,
/// which is far more visible at this size than the coarser size steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum TextSize {
    #[default]
    Small,
    Medium,
    Large,
}

impl TextSize {
    pub fn scale(self) -> u32 {
        match self {
            TextSize::Small => 1,
            TextSize::Medium => 2,
            TextSize::Large => 3,
        }
    }
}

/// A scalar value a widget can display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Value {
    Number(f64),
    Text(String),
    Bool(bool),
    Timestamp(DateTime<Utc>),
}

impl Value {
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::Text(s) => s.parse().ok(),
            Value::Timestamp(_) => None,
        }
    }

    pub fn as_timestamp(&self) -> Option<DateTime<Utc>> {
        match self {
            Value::Timestamp(t) => Some(*t),
            Value::Text(s) => DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|t| t.with_timezone(&Utc)),
            _ => None,
        }
    }

    pub fn as_display(&self) -> String {
        match self {
            Value::Number(n) if n.fract() == 0.0 => format!("{n:.0}"),
            Value::Number(n) => format!("{n:.1}"),
            Value::Text(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Timestamp(t) => t.to_rfc3339(),
        }
    }
}

/// Where a widget gets its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Binding {
    /// A fixed value baked into the layout.
    Literal { value: Value },
    /// A live value from a registered data source.
    DataPoint {
        source: String,
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scale: Option<f64>,
    },
}

impl Binding {
    pub fn literal_text(s: impl Into<String>) -> Self {
        Binding::Literal {
            value: Value::Text(s.into()),
        }
    }
    pub fn literal_number(n: f64) -> Self {
        Binding::Literal {
            value: Value::Number(n),
        }
    }
}

/// A live reading from a data source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DataPoint {
    pub source: String,
    pub key: String,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub observed_at: DateTime<Utc>,
    /// After this many seconds the reading is stale and must be rendered as such.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u64>,
}

impl DataPoint {
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        match self.ttl_secs {
            Some(ttl) => (now - self.observed_at).num_seconds() > ttl as i64,
            None => false,
        }
    }
}

/// The data available at render time.
#[derive(Debug, Clone, Default)]
pub struct DataStore {
    points: HashMap<(String, String), DataPoint>,
}

impl DataStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, point: DataPoint) {
        self.points
            .insert((point.source.clone(), point.key.clone()), point);
    }

    pub fn get(&self, source: &str, key: &str) -> Option<&DataPoint> {
        self.points.get(&(source.to_string(), key.to_string()))
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Every stored reading, in no particular order.
    pub fn all(&self) -> impl Iterator<Item = &DataPoint> {
        self.points.values()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Resolve a binding to a value, reporting whether the reading is stale.
    ///
    /// A missing or stale point returns `None` for the value rather than a default, so
    /// widgets can show a placeholder instead of silently displaying a wrong number.
    pub fn resolve(&self, binding: &Binding, now: DateTime<Utc>) -> Resolved {
        match binding {
            Binding::Literal { value } => Resolved {
                value: Some(value.clone()),
                stale: false,
                missing: false,
            },
            Binding::DataPoint { source, key, scale } => match self.get(source, key) {
                None => Resolved {
                    value: None,
                    stale: false,
                    missing: true,
                },
                Some(p) => {
                    let stale = p.is_stale(now);
                    let value = match (scale, p.value.as_number()) {
                        (Some(s), Some(n)) => Value::Number(n * s),
                        _ => p.value.clone(),
                    };
                    Resolved {
                        value: Some(value),
                        stale,
                        missing: false,
                    }
                }
            },
        }
    }
}

/// The outcome of resolving a [`Binding`].
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub value: Option<Value>,
    pub stale: bool,
    pub missing: bool,
}

impl Resolved {
    /// True when the value should not be trusted as a current reading.
    pub fn degraded(&self) -> bool {
        self.stale || self.missing
    }

    pub fn number_or(&self, fallback: f64) -> f64 {
        self.value
            .as_ref()
            .and_then(|v| v.as_number())
            .unwrap_or(fallback)
    }

    /// Display text, or a placeholder when there is nothing trustworthy to show.
    pub fn text_or_placeholder(&self) -> String {
        match (&self.value, self.missing) {
            (Some(v), false) => v.as_display(),
            _ => "--".to_string(),
        }
    }
}

/// How a countdown is formatted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimerFormat {
    /// `2h 14m`
    #[default]
    Compact,
    /// `02:14:33`
    Clock,
    /// `134m`
    Minutes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum BarStyle {
    #[default]
    Solid,
    Segmented,
}

/// Where an image comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageSource {
    Path {
        path: String,
    },
    /// Base64-encoded image bytes, for pushing an image over MCP.
    Inline {
        base64: String,
    },
}

/// The drawable elements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Widget {
    Label {
        text: Binding,
        #[serde(default)]
        size: TextSize,
        #[serde(default)]
        align: Align,
        #[serde(default)]
        color: Color,
    },
    ProgressBar {
        /// Fraction in 0.0..=1.0.
        value: Binding,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<Binding>,
        #[serde(default)]
        style: BarStyle,
        #[serde(default = "default_accent")]
        color: Color,
        #[serde(default = "default_track")]
        track: Color,
        /// Draw the percentage inside the bar.
        #[serde(default)]
        show_value: bool,
    },
    Gauge {
        value: Binding,
        #[serde(default)]
        min: f64,
        #[serde(default = "default_one")]
        max: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default = "default_accent")]
        color: Color,
        #[serde(default = "default_track")]
        track: Color,
    },
    ResetTimer {
        deadline: Binding,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default)]
        format: TimerFormat,
        /// Round the remaining time to a multiple of this many minutes, for the same
        /// flash-endurance reason as [`Widget::Clock`].
        #[serde(default = "default_quantize")]
        quantize_minutes: u32,
        #[serde(default)]
        color: Color,
    },
    Clock {
        /// strftime format, e.g. `%H:%M`.
        #[serde(default = "default_clock_format")]
        format: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
        /// Round the displayed time down to a multiple of this many minutes.
        ///
        /// This is a **flash-endurance control**, not a formatting preference. The write
        /// governor skips uploads whose pixels are unchanged, so the displayed resolution
        /// sets the write rate: a 1-minute clock changes 1,440 times a day and would
        /// exhaust the panel in about 69 days, where a 15-minute clock changes 96 times
        /// and lasts years. See `docs/FLASH_BUDGET.md`.
        #[serde(default = "default_quantize")]
        quantize_minutes: u32,
        #[serde(default)]
        size: TextSize,
        #[serde(default)]
        align: Align,
        #[serde(default)]
        color: Color,
    },
    Image {
        source: ImageSource,
        #[serde(default)]
        fit: Fit,
    },
    Sparkline {
        /// Recent values, oldest first.
        points: Vec<f64>,
        #[serde(default = "default_accent")]
        color: Color,
    },
    /// Fill the slot with a colour.
    Fill {
        #[serde(default)]
        color: Color,
    },
    Blank,
}

fn default_accent() -> Color {
    Color::new(0x4A, 0xC8, 0xFF)
}
fn default_track() -> Color {
    Color::new(0x22, 0x2A, 0x33)
}
fn default_one() -> f64 {
    1.0
}
fn default_clock_format() -> String {
    "%H:%M".to_string()
}
/// Default display resolution for time widgets, matching the default write interval.
fn default_quantize() -> u32 {
    15
}

/// Round a timestamp down to a multiple of `minutes`.
pub fn quantize_time<Tz: chrono::TimeZone>(t: DateTime<Tz>, minutes: u32) -> DateTime<Tz> {
    use chrono::Timelike;
    if minutes <= 1 {
        return t;
    }
    let m = t.minute();
    let floored = m - (m % minutes.max(1));
    t.with_minute(floored)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(t)
}

/// Estimated image changes per day at a given display resolution, and the projected
/// panel lifetime in years against the conservative 100,000-cycle budget.
pub fn writes_per_day(quantize_minutes: u32) -> f64 {
    if quantize_minutes == 0 {
        return 1440.0;
    }
    1440.0 / quantize_minutes as f64
}

/// A named region of the panel holding one widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Slot {
    /// Stable handle, used by MCP `set_widget` to address this region.
    pub id: String,
    pub rect: Rect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<Widget>,
}

/// A complete screen definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Layout {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub background: Color,
    #[serde(default)]
    pub slots: Vec<Slot>,
}

impl Layout {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: String::new(),
            background: Color::BLACK,
            slots: Vec::new(),
        }
    }

    pub fn with_slot(mut self, id: impl Into<String>, rect: Rect, widget: Widget) -> Self {
        self.slots.push(Slot {
            id: id.into(),
            rect,
            widget: Some(widget),
        });
        self
    }

    pub fn slot_mut(&mut self, id: &str) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.id == id)
    }

    /// Report layout problems: duplicate ids, empty or out-of-bounds rects, overlaps.
    ///
    /// Overlaps are reported rather than rejected — layering can be deliberate — but a
    /// silent overlap is a common cause of "my widget disappeared".
    pub fn lint(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut seen: HashMap<&str, usize> = HashMap::new();

        for (i, s) in self.slots.iter().enumerate() {
            if let Some(prev) = seen.insert(&s.id, i) {
                problems.push(format!("slot {i} reuses id {:?} from slot {prev}", s.id));
            }
            if s.rect.is_empty() {
                problems.push(format!("slot {:?} has zero width or height", s.id));
            }
            if s.rect.x + s.rect.w > SCREEN_W || s.rect.y + s.rect.h > SCREEN_H {
                problems.push(format!(
                    "slot {:?} extends past the {SCREEN_W}x{SCREEN_H} panel and will be clipped",
                    s.id
                ));
            }
        }

        for (i, a) in self.slots.iter().enumerate() {
            for b in self.slots.iter().skip(i + 1) {
                let (ra, rb) = (a.rect, b.rect);
                let overlap = ra.x < rb.x + rb.w
                    && rb.x < ra.x + ra.w
                    && ra.y < rb.y + rb.h
                    && rb.y < ra.y + ra.h;
                if overlap {
                    problems.push(format!("slots {:?} and {:?} overlap", a.id, b.id));
                }
            }
        }
        problems
    }
}

/// The JSON Schema for a [`Layout`], for MCP tool advertisement and editor tooling.
pub fn layout_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Layout)).expect("schema serialises")
}

/// The JSON Schema for a [`Widget`], used by the MCP `set_widget` tool.
pub fn widget_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Widget)).expect("schema serialises")
}
