//! Text and layout (ADR 0107): how big and how dense Silt's UI is, over any
//! theme. Each setting left out is Silt's default, so a default can change
//! later without rewriting anyone's file.
//!
//! Read leniently: a value out of range is clamped, and one that can't be
//! read is left out, so a hand-edited `appearance.json` never loses the rest.
//! The CSS is built from the checked values, never passed through.

use std::fmt::{self, Write as _};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::FontName;

macro_rules! clamped {
    ($(#[$doc:meta])* $name:ident, $min:expr, $max:expr, $default:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        pub struct $name(u16);

        impl $name {
            pub const MIN: u16 = $min;
            pub const MAX: u16 = $max;
            pub const DEFAULT: Self = Self($default);

            pub fn new(value: u16) -> Self {
                Self(value.clamp(Self::MIN, Self::MAX))
            }

            pub fn get(self) -> u16 {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::DEFAULT
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_u16(self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                f64::deserialize(deserializer).map(|value| Self::new(rounded(value)))
            }
        }
    };
}

/// A number from JSON as a whole number in `u16`'s range, for clamping.
fn rounded(value: f64) -> u16 {
    if value.is_nan() {
        0
    } else {
        // Clamped first, so the cast can neither wrap nor lose a sign.
        value.round().clamp(0.0, f64::from(u16::MAX)) as u16
    }
}

clamped!(
    /// The interface's text in pixels: sidebar, headers, buttons and lists.
    UiSize, 11, 18, 13
);
clamped!(
    /// The conversation's text in pixels: replies, the person's messages, docs.
    ChatSize, 11, 22, 13
);
clamped!(
    /// Code's text in pixels: code blocks, diffs, tool output.
    CodeSize, 10, 20, 12
);
clamped!(
    /// How many columns a tab takes in code and diffs.
    TabWidth, 1, 8, 4
);

/// The desktop app's zoom in percent, in steps of ten. A browser has its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "number"))]
pub struct Zoom(u16);

impl Zoom {
    pub const MIN: u16 = 50;
    pub const MAX: u16 = 200;
    pub const STEP: u16 = 10;
    pub const DEFAULT: Self = Self(100);

    pub fn new(percent: u16) -> Self {
        let stepped = (percent + Self::STEP / 2) / Self::STEP * Self::STEP;
        Self(stepped.clamp(Self::MIN, Self::MAX))
    }

    pub fn percent(self) -> u16 {
        self.0
    }
}

impl Default for Zoom {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Serialize for Zoom {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u16(self.0)
    }
}

impl<'de> Deserialize<'de> for Zoom {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        f64::deserialize(deserializer).map(|value| Self::new(rounded(value)))
    }
}

/// A font chosen over the theme's: the system's, or one by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "string"))]
pub enum FontChoice {
    System,
    Named(FontName),
}

impl FontChoice {
    const SYSTEM: &str = "system";

    pub fn parse(text: &str) -> Option<Self> {
        if text.trim() == Self::SYSTEM { Some(Self::System) } else { FontName::parse(text).map(Self::Named) }
    }

    /// The family list it stands for, ending in `generic`'s stack.
    fn stack(&self, generic: Generic) -> String {
        match self {
            Self::System => generic.system().to_owned(),
            Self::Named(name) if name.generic() => name.as_str().to_owned(),
            Self::Named(name) => format!("\"{}\", {}", name.as_str(), generic.system()),
        }
    }
}

impl fmt::Display for FontChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::System => f.write_str(Self::SYSTEM),
            Self::Named(name) => f.write_str(name.as_str()),
        }
    }
}

impl Serialize for FontChoice {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for FontChoice {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("{text:?} isn't a font name")))
    }
}

#[derive(Clone, Copy)]
enum Generic {
    Sans,
    Mono,
}

impl Generic {
    fn system(self) -> &'static str {
        match self {
            Self::Sans => "system-ui, -apple-system, 'Segoe UI', sans-serif",
            Self::Mono => "ui-monospace, 'SF Mono', Menlo, Consolas, monospace",
        }
    }
}

macro_rules! choice {
    ($(#[$doc:meta])* $name:ident { $($(#[$vdoc:meta])* $variant:ident),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($(#[$vdoc])* $variant),+
        }
    };
}

choice!(
    /// How tightly lists, the sidebar and a thread's turns are packed.
    Density {
        Compact,
        #[default]
        Regular,
        Roomy,
    }
);
choice!(
    /// The conversation's line height.
    LineHeight {
        Tight,
        #[default]
        Regular,
        Loose,
    }
);
choice!(
    /// How wide a thread's column grows.
    ReadingWidth {
        Narrow,
        #[default]
        Regular,
        Wide,
        Full,
    }
);
choice!(
    /// What a diff opens in where its page hasn't kept a choice of its own.
    DiffLayout {
        #[default]
        Unified,
        Split,
    }
);
choice!(
    /// How much of the agent's work a thread shows (ADR 0049).
    Detail {
        #[default]
        Normal,
        Verbose,
    }
);
choice!(
    /// How the agent's work is grouped (ADR 0049).
    Grouping {
        #[default]
        Turn,
        Run,
    }
);
choice!(
    /// Whether to cut animations short: as the system asks, always, or never.
    Motion {
        #[default]
        System,
        Reduced,
        Full,
    }
);

/// Text and layout on this machine. `None` is Silt's default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TextSettings {
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub zoom: Option<Zoom>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub ui_size: Option<UiSize>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub chat_size: Option<ChatSize>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub code_size: Option<CodeSize>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub ui_font: Option<FontChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub code_font: Option<FontChoice>,
    /// Ligatures in code, where the font has them.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub ligatures: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub density: Option<Density>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub line_height: Option<LineHeight>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub width: Option<ReadingWidth>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub diff_layout: Option<DiffLayout>,
    /// Long lines in a unified diff wrap at the panel's edge rather than
    /// scroll; the web UI draws it, so it adds no CSS.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub wrap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub tab_width: Option<TabWidth>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub detail: Option<Detail>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub grouping: Option<Grouping>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub motion: Option<Motion>,
}

/// A value that can't be read is left out, rather than failing the whole file.
fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned>(deserializer: D) -> Result<Option<T>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// `TextSettings`, or the defaults where it can't be read at all.
pub fn lenient_settings<'de, D: Deserializer<'de>>(deserializer: D) -> Result<TextSettings, D::Error> {
    lenient(deserializer).map(Option::unwrap_or_default)
}

impl TextSettings {
    /// What the page's root says of motion, as `data-motion`, or none to
    /// follow the system.
    pub fn motion_attribute(&self) -> Option<&'static str> {
        match self.motion? {
            Motion::System => None,
            Motion::Reduced => Some("reduced"),
            Motion::Full => Some("full"),
        }
    }
}

/// The CSS for `settings`, after the theme's; empty when everything is Silt's
/// default. `:root` thrice outranks the theme's `:root:not([data-theme=…])`,
/// so a font chosen here wins over the theme's.
pub fn css(settings: &TextSettings) -> String {
    let mut out = String::new();
    let mut put = |name: &str, value: &dyn fmt::Display| {
        let _ = write!(out, "--{name}:{value};");
    };
    if let Some(size) = settings.ui_size {
        put("ui-size", &format_args!("{}px", size.get()));
    }
    if let Some(size) = settings.chat_size {
        put("chat-size", &format_args!("{}px", size.get()));
    }
    if let Some(size) = settings.code_size {
        put("code-size", &format_args!("{}px", size.get()));
    }
    if let Some(font) = &settings.ui_font {
        put("font-sans", &font.stack(Generic::Sans));
    }
    if let Some(font) = &settings.code_font {
        put("font-mono", &font.stack(Generic::Mono));
    }
    if settings.ligatures == Some(false) {
        put("code-ligatures", &"none");
    }
    match settings.density {
        Some(Density::Compact) => {
            put("row", &"26px");
            put("density", &"0.6");
        }
        Some(Density::Roomy) => {
            put("row", &"36px");
            put("density", &"1.4");
        }
        Some(Density::Regular) | None => {}
    }
    match settings.line_height {
        Some(LineHeight::Tight) => put("chat-leading", &"1.45"),
        Some(LineHeight::Loose) => put("chat-leading", &"1.8"),
        Some(LineHeight::Regular) | None => {}
    }
    match settings.width {
        Some(ReadingWidth::Narrow) => put("read-width", &"640px"),
        Some(ReadingWidth::Wide) => put("read-width", &"880px"),
        Some(ReadingWidth::Full) => put("read-width", &"100%"),
        Some(ReadingWidth::Regular) | None => {}
    }
    if let Some(width) = settings.tab_width {
        put("tab-size", &width.get());
    }
    if out.is_empty() { out } else { format!(":root:root:root{{{out}}}\n") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(json: &str) -> TextSettings {
        serde_json::from_str(json).expect("reads")
    }

    #[test]
    fn nothing_set_is_no_css() {
        assert_eq!(css(&TextSettings::default()), "");
        assert_eq!(read("{}"), TextSettings::default());
        assert_eq!(serde_json::to_string(&TextSettings::default()).expect("writes"), "{}");
    }

    #[test]
    fn sizes_are_clamped_and_zoom_kept_to_its_steps() {
        let settings = read(r#"{ "ui_size": 40, "chat_size": 3, "code_size": 13.6, "zoom": 117, "tab_width": 0 }"#);
        assert_eq!(settings.ui_size, Some(UiSize::new(18)));
        assert_eq!(settings.chat_size, Some(ChatSize::new(11)));
        assert_eq!(settings.code_size.map(CodeSize::get), Some(14));
        assert_eq!(settings.zoom.map(Zoom::percent), Some(120));
        assert_eq!(settings.tab_width.map(TabWidth::get), Some(1));
        assert_eq!(Zoom::new(10).percent(), 50);
        assert_eq!(Zoom::new(999).percent(), 200);
        assert_eq!(read(r#"{ "zoom": -5 }"#).zoom.map(Zoom::percent), Some(50));
    }

    #[test]
    fn a_value_that_cant_be_read_is_left_out_and_the_rest_kept() {
        let settings = read(r#"{ "density": "huge", "ui_font": "x\"}body{", "chat_size": "big", "code_size": 14, "wrap": 3 }"#);
        assert_eq!(settings, TextSettings { code_size: Some(CodeSize::new(14)), ..TextSettings::default() });
    }

    #[test]
    fn each_setting_becomes_its_variable() {
        let settings = read(
            r#"{ "ui_size": 14, "chat_size": 16, "code_size": 13, "ui_font": "Inter", "code_font": "system",
                 "ligatures": false, "density": "compact", "line_height": "loose", "width": "full",
                 "wrap": false, "tab_width": 2, "zoom": 120, "detail": "verbose", "motion": "reduced" }"#,
        );
        assert_eq!(
            css(&settings),
            ":root:root:root{--ui-size:14px;--chat-size:16px;--code-size:13px;\
             --font-sans:\"Inter\", system-ui, -apple-system, 'Segoe UI', sans-serif;\
             --font-mono:ui-monospace, 'SF Mono', Menlo, Consolas, monospace;--code-ligatures:none;\
             --row:26px;--density:0.6;--chat-leading:1.8;--read-width:100%;--tab-size:2;}\n"
        );
        assert_eq!(settings.motion_attribute(), Some("reduced"));
    }

    #[test]
    fn a_default_written_out_adds_nothing() {
        let settings =
            read(r#"{ "density": "regular", "line_height": "regular", "width": "regular", "ligatures": true, "wrap": true }"#);
        assert_eq!(css(&settings), "");
        assert_eq!(read(r#"{ "motion": "system" }"#).motion_attribute(), None);
    }

    #[test]
    fn a_generic_family_stands_alone() {
        let settings = read(r#"{ "code_font": "monospace" }"#);
        assert_eq!(css(&settings), ":root:root:root{--font-mono:monospace;}\n");
    }

    #[test]
    fn it_round_trips() {
        let json = r#"{"zoom":110,"chat_size":15,"code_font":"JetBrains Mono","density":"roomy","diff_layout":"split"}"#;
        assert_eq!(serde_json::to_string(&read(json)).expect("writes"), json);
    }

    #[test]
    fn the_css_holds_nothing_a_file_could_smuggle_in() {
        let settings = read(r#"{ "ui_font": "A_b-9 Sans", "code_font": "Mono 2" }"#);
        let out = css(&settings);
        let allowed = |c: char| c.is_ascii_alphanumeric() || " #%(),.:;{}=\"'_-\n".contains(c);
        assert!(out.chars().all(allowed), "{out}");
        assert!(FontChoice::parse("x;}body{").is_none());
        assert!(FontChoice::parse("url(x)").is_none());
    }
}
