//! A theme file read leniently: whatever is wrong with part of it is a
//! warning naming where, and the rest still loads.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::color::Rgba;
use crate::font::{FontFile, MOST_FACES};
use crate::token::{ColorToken, SyntaxTag};

/// Where editors find the format's schema.
pub const SCHEMA_URL: &str = "https://raw.githubusercontent.com/Urcra/silt/main/docs/reference/theme.schema.json";

/// A theme's id: lowercase letters, digits and hyphens, at most 64.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ThemeId(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} isn't a theme id: lowercase letters, digits and hyphens, at most 64")]
pub struct BadThemeId(pub String);

impl ThemeId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// An id made from a name: "Rosé Pine Dawn" is `rose-pine-dawn`.
    pub fn from_name(name: &str) -> Option<Self> {
        let mut id = String::new();
        for c in name.chars().flat_map(fold_accent) {
            if c.is_ascii_alphanumeric() {
                id.push(c.to_ascii_lowercase());
            } else if !id.ends_with('-') && !id.is_empty() {
                id.push('-');
            }
        }
        let id = id.trim_end_matches('-');
        Self::try_from(id.chars().take(64).collect::<String>().trim_end_matches('-').to_owned()).ok()
    }
}

fn fold_accent(c: char) -> Option<char> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        c => c,
    })
}

impl TryFrom<String> for ThemeId {
    type Error = BadThemeId;
    fn try_from(id: String) -> Result<Self, Self::Error> {
        let ok = (1..=64).contains(&id.len())
            && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !id.starts_with('-');
        if ok { Ok(Self(id)) } else { Err(BadThemeId(id)) }
    }
}

impl std::str::FromStr for ThemeId {
    type Err = BadThemeId;
    fn from_str(id: &str) -> Result<Self, Self::Err> {
        Self::try_from(id.to_owned())
    }
}

impl From<ThemeId> for String {
    fn from(id: ThemeId) -> Self {
        id.0
    }
}

impl fmt::Display for ThemeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

impl fmt::Display for Appearance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A font family's name, quoted by us when it reaches the CSS: letters,
/// digits, spaces, `_` and `-`, at most 64.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(into = "String")]
pub struct FontName(String);

/// The families CSS names without quotes.
pub const GENERIC_FAMILIES: &[&str] =
    &["serif", "sans-serif", "monospace", "system-ui", "ui-sans-serif", "ui-serif", "ui-monospace", "ui-rounded"];

impl FontName {
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim();
        let ok = (1..=64).contains(&name.len())
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'_' || b == b'-');
        ok.then(|| Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn generic(&self) -> bool {
        GENERIC_FAMILIES.contains(&self.0.as_str())
    }
}

impl From<FontName> for String {
    fn from(name: FontName) -> Self {
        name.0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Fonts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<Vec<FontName>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mono: Option<Vec<FontName>>,
    /// Faces the theme ships in its folder's `fonts/` (ADR 0099).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<FontFace>,
}

impl Fonts {
    fn is_empty(&self) -> bool {
        self.ui.is_none() && self.mono.is_none() && self.faces.is_empty()
    }
}

/// One face of a family the theme ships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FontFace {
    pub family: FontName,
    pub file: FontFile,
    pub weight: u16,
    pub style: FontStyle,
}

/// The largest radius a theme may give, in pixels.
pub const MAX_RADIUS: u8 = 24;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Radii {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sm: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub md: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lg: Option<u8>,
}

impl Radii {
    fn is_empty(&self) -> bool {
        self.sm.is_none() && self.md.is_none() && self.lg.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontStyle {
    Normal,
    Italic,
}

impl FontStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Italic => "italic",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SyntaxStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Rgba>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_style: Option<FontStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<u16>,
}

impl SyntaxStyle {
    pub fn color(color: Rgba) -> Self {
        Self { color: Some(color), ..Self::default() }
    }

    fn is_empty(&self) -> bool {
        self.color.is_none() && self.font_style.is_none() && self.font_weight.is_none()
    }
}

/// One theme: one appearance, with what it sets. What it leaves out is
/// derived (`crate::derive`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Theme {
    pub id: ThemeId,
    pub name: String,
    pub appearance: Appearance,
    pub colors: BTreeMap<ColorToken, Rgba>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub syntax: BTreeMap<SyntaxTag, SyntaxStyle>,
    #[serde(skip_serializing_if = "Fonts::is_empty")]
    pub fonts: Fonts,
    #[serde(skip_serializing_if = "Radii::is_empty")]
    pub radii: Radii,
}

/// A theme file: a family of themes, with who made them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Family {
    #[serde(rename = "$schema")]
    pub schema: &'static str,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
    pub themes: Vec<Theme>,
}

impl Family {
    pub fn new(name: String, themes: Vec<Theme>) -> Self {
        Self { schema: SCHEMA_URL, name, author: None, license: None, source: None, copyright: None, themes }
    }

    /// The file, as `parse` reads it back.
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("a family serializes");
        text.push('\n');
        text
    }
}

/// Something in a file that was left out, and where (a JSON pointer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Warning {
    pub path: String,
    pub message: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", if self.path.is_empty() { "/" } else { &self.path }, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThemeError {
    #[error("not JSON: {0}")]
    Json(String),
    #[error("{path}: {message}")]
    Invalid { path: String, message: String },
    #[error("no theme in the file can be used: {0}")]
    NoThemes(String),
}

/// The longest free text a theme keeps: names, authors, licences, sources.
const TEXT_CHARS: usize = 200;

/// Reads a theme file in Silt's format.
pub fn parse(json: &str) -> Result<(Family, Vec<Warning>), ThemeError> {
    let value: Value = serde_json::from_str(json).map_err(|error| ThemeError::Json(error.to_string()))?;
    let mut warnings = Vec::new();
    let family = family(&value, &mut warnings)?;
    Ok((family, warnings))
}

fn invalid(path: &str, message: impl Into<String>) -> ThemeError {
    ThemeError::Invalid { path: if path.is_empty() { "/".to_owned() } else { path.to_owned() }, message: message.into() }
}

fn warn(warnings: &mut Vec<Warning>, path: String, message: impl Into<String>) {
    warnings.push(Warning { path, message: message.into() });
}

pub(crate) fn pointer(parent: &str, key: &str) -> String {
    format!("{parent}/{}", key.replace('~', "~0").replace('/', "~1"))
}

fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, ThemeError> {
    value.as_object().ok_or_else(|| invalid(path, "should be an object"))
}

pub(crate) fn text(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(TEXT_CHARS).collect::<String>().trim().to_owned()
}

fn optional_text(map: &Map<String, Value>, key: &str, path: &str, warnings: &mut Vec<Warning>) -> Option<String> {
    match map.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(text(value)).filter(|value| !value.is_empty()),
        Some(_) => {
            warn(warnings, pointer(path, key), "should be a string; left out");
            None
        }
    }
}

fn family(value: &Value, warnings: &mut Vec<Warning>) -> Result<Family, ThemeError> {
    let map = object(value, "")?;
    for key in map.keys() {
        if !["$schema", "name", "author", "license", "source", "copyright", "themes"].contains(&key.as_str()) {
            warn(warnings, pointer("", key), "isn't part of a theme file; ignored");
        }
    }
    let name = match map.get("name") {
        Some(Value::String(name)) if !text(name).is_empty() => text(name),
        _ => return Err(invalid("/name", "a theme file needs a name")),
    };
    let entries = map.get("themes").and_then(Value::as_array).ok_or_else(|| invalid("/themes", "should be a list of themes"))?;
    let mut themes = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let path = format!("/themes/{index}");
        if let Some(theme) = theme(entry, &path, warnings) {
            if seen.insert(theme.id.clone()) {
                themes.push(theme);
            } else {
                warn(warnings, pointer(&path, "id"), format!("{} is already used in this file; theme left out", theme.id));
            }
        }
    }
    if themes.is_empty() {
        let why = warnings.last().map_or_else(|| "it has none".to_owned(), ToString::to_string);
        return Err(ThemeError::NoThemes(why));
    }
    Ok(Family {
        schema: SCHEMA_URL,
        name,
        author: optional_text(map, "author", "", warnings),
        license: optional_text(map, "license", "", warnings),
        source: optional_text(map, "source", "", warnings),
        copyright: optional_text(map, "copyright", "", warnings),
        themes,
    })
}

fn theme(value: &Value, path: &str, warnings: &mut Vec<Warning>) -> Option<Theme> {
    let Some(map) = value.as_object() else {
        warn(warnings, path.to_owned(), "should be an object; theme left out");
        return None;
    };
    for key in map.keys() {
        if !["id", "name", "appearance", "colors", "syntax", "fonts", "radii"].contains(&key.as_str()) {
            warn(warnings, pointer(path, key), "isn't part of a theme; ignored");
        }
    }
    let id = match map.get("id").and_then(Value::as_str).map(|id| ThemeId::try_from(id.to_owned())) {
        Some(Ok(id)) => id,
        Some(Err(error)) => {
            warn(warnings, pointer(path, "id"), format!("{error}; theme left out"));
            return None;
        }
        None => {
            warn(warnings, pointer(path, "id"), "a theme needs an id; theme left out");
            return None;
        }
    };
    let appearance = match map.get("appearance").and_then(Value::as_str) {
        Some("light") => Appearance::Light,
        Some("dark") => Appearance::Dark,
        _ => {
            warn(warnings, pointer(path, "appearance"), "should be \"light\" or \"dark\"; theme left out");
            return None;
        }
    };
    let name = map.get("name").and_then(Value::as_str).map(text).filter(|name| !name.is_empty()).unwrap_or_else(|| {
        warn(warnings, pointer(path, "name"), "a theme needs a name; its id stands in");
        id.to_string()
    });
    let colors = colors(map.get("colors"), &pointer(path, "colors"), warnings);
    let missing: Vec<_> = ColorToken::ALL.iter().filter(|token| token.required() && !colors.contains_key(token)).collect();
    if !missing.is_empty() {
        let names: Vec<_> = missing.iter().map(|token| token.as_str()).collect();
        warn(warnings, pointer(path, "colors"), format!("needs {}; theme left out", names.join(", ")));
        return None;
    }
    Some(Theme {
        id,
        name,
        appearance,
        colors,
        syntax: syntax(map.get("syntax"), &pointer(path, "syntax"), warnings),
        fonts: fonts(map.get("fonts"), &pointer(path, "fonts"), warnings),
        radii: radii(map.get("radii"), &pointer(path, "radii"), warnings),
    })
}

fn colors(value: Option<&Value>, path: &str, warnings: &mut Vec<Warning>) -> BTreeMap<ColorToken, Rgba> {
    let mut colors = BTreeMap::new();
    let Some(value) = value else { return colors };
    let Some(map) = value.as_object() else {
        warn(warnings, path.to_owned(), "should be an object of colours; left out");
        return colors;
    };
    for (key, value) in map {
        let Some(token) = ColorToken::from_name(key) else {
            warn(warnings, pointer(path, key), "isn't a colour Silt has; ignored");
            continue;
        };
        match value.as_str().map(Rgba::parse) {
            Some(Ok(color)) => {
                colors.insert(token, color);
            }
            Some(Err(error)) => warn(warnings, pointer(path, key), format!("{error}; derived instead")),
            None => warn(warnings, pointer(path, key), "should be a hex colour; derived instead"),
        }
    }
    colors
}

fn syntax(value: Option<&Value>, path: &str, warnings: &mut Vec<Warning>) -> BTreeMap<SyntaxTag, SyntaxStyle> {
    let mut styles = BTreeMap::new();
    let Some(value) = value else { return styles };
    let Some(map) = value.as_object() else {
        warn(warnings, path.to_owned(), "should be an object of highlight tags; left out");
        return styles;
    };
    for (key, value) in map {
        let here = pointer(path, key);
        let Some(tag) = SyntaxTag::from_name(key) else {
            warn(warnings, here, "isn't a highlight tag Silt has; ignored");
            continue;
        };
        let style = match value {
            Value::String(color) => match Rgba::parse(color) {
                Ok(color) => SyntaxStyle::color(color),
                Err(error) => {
                    warn(warnings, here, format!("{error}; derived instead"));
                    continue;
                }
            },
            Value::Object(fields) => syntax_style(fields, &here, warnings),
            _ => {
                warn(warnings, here, "should be a colour or an object; derived instead");
                continue;
            }
        };
        if !style.is_empty() {
            styles.insert(tag, style);
        }
    }
    styles
}

fn syntax_style(fields: &Map<String, Value>, path: &str, warnings: &mut Vec<Warning>) -> SyntaxStyle {
    let mut style = SyntaxStyle::default();
    for (key, value) in fields {
        let here = pointer(path, key);
        match key.as_str() {
            "color" => match value.as_str().map(Rgba::parse) {
                Some(Ok(color)) => style.color = Some(color),
                Some(Err(error)) => warn(warnings, here, format!("{error}; derived instead")),
                None => warn(warnings, here, "should be a hex colour; derived instead"),
            },
            "font_style" => match value.as_str() {
                Some("normal") => style.font_style = Some(FontStyle::Normal),
                Some("italic") => style.font_style = Some(FontStyle::Italic),
                _ => warn(warnings, here, "should be \"normal\" or \"italic\"; ignored"),
            },
            "font_weight" => match value.as_f64() {
                Some(weight) if (100.0..=900.0).contains(&weight) => {
                    style.font_weight = Some(((weight / 100.0).round() * 100.0) as u16);
                }
                _ => warn(warnings, here, "should be a weight from 100 to 900; ignored"),
            },
            _ => warn(warnings, here, "isn't part of a highlight style; ignored"),
        }
    }
    style
}

fn fonts(value: Option<&Value>, path: &str, warnings: &mut Vec<Warning>) -> Fonts {
    let mut fonts = Fonts::default();
    let Some(value) = value else { return fonts };
    let Some(map) = value.as_object() else {
        warn(warnings, path.to_owned(), "should be an object with ui and mono; left out");
        return fonts;
    };
    for (key, value) in map {
        let here = pointer(path, key);
        let slot = match key.as_str() {
            "ui" => &mut fonts.ui,
            "mono" => &mut fonts.mono,
            "faces" => {
                fonts.faces = faces(value, &here, warnings);
                continue;
            }
            _ => {
                warn(warnings, here, "isn't one of ui, mono and faces; ignored");
                continue;
            }
        };
        let Some(list) = value.as_array() else {
            warn(warnings, here, "should be a list of font names; Silt's own fonts stand in");
            continue;
        };
        let mut names = Vec::new();
        for (index, name) in list.iter().enumerate() {
            match name.as_str().and_then(FontName::parse) {
                Some(name) => names.push(name),
                None => warn(
                    warnings,
                    format!("{here}/{index}"),
                    "a font's name has letters, digits, spaces, _ and - only, at most 64; left out",
                ),
            }
        }
        if !names.is_empty() {
            *slot = Some(names);
        }
    }
    fonts
}

fn faces(value: &Value, path: &str, warnings: &mut Vec<Warning>) -> Vec<FontFace> {
    let Some(list) = value.as_array() else {
        warn(warnings, path.to_owned(), "should be a list of faces; left out");
        return Vec::new();
    };
    let mut faces = Vec::new();
    for (index, entry) in list.iter().enumerate() {
        let here = format!("{path}/{index}");
        if faces.len() == MOST_FACES {
            warn(warnings, here, format!("a theme has at most {MOST_FACES} faces; the rest left out"));
            break;
        }
        let Some(fields) = entry.as_object() else {
            warn(warnings, here, "should be an object with family and file; left out");
            continue;
        };
        let Some(family) = fields.get("family").and_then(Value::as_str).and_then(FontName::parse) else {
            warn(
                warnings,
                pointer(&here, "family"),
                "a face needs a family named with letters, digits, spaces, _ and -; left out",
            );
            continue;
        };
        let Some(file) = fields.get("file").and_then(Value::as_str).and_then(FontFile::parse) else {
            warn(
                warnings,
                pointer(&here, "file"),
                "a face needs a file such as fonts/name.woff2 (woff2, woff, ttf or otf); left out",
            );
            continue;
        };
        let weight = match fields.get("weight") {
            None => 400,
            Some(weight) => match weight.as_f64().filter(|weight| (100.0..=900.0).contains(weight)) {
                Some(weight) => ((weight / 100.0).round() * 100.0) as u16,
                None => {
                    warn(warnings, pointer(&here, "weight"), "should be a weight from 100 to 900; 400 used");
                    400
                }
            },
        };
        let style = match fields.get("style").and_then(Value::as_str) {
            None | Some("normal") => FontStyle::Normal,
            Some("italic") => FontStyle::Italic,
            Some(_) => {
                warn(warnings, pointer(&here, "style"), "should be \"normal\" or \"italic\"; normal used");
                FontStyle::Normal
            }
        };
        for key in fields.keys().filter(|key| !["family", "file", "weight", "style"].contains(&key.as_str())) {
            warn(warnings, pointer(&here, key), "isn't part of a face; ignored");
        }
        faces.push(FontFace { family, file, weight, style });
    }
    faces
}

fn radii(value: Option<&Value>, path: &str, warnings: &mut Vec<Warning>) -> Radii {
    let mut radii = Radii::default();
    let Some(value) = value else { return radii };
    let Some(map) = value.as_object() else {
        warn(warnings, path.to_owned(), "should be an object with sm, md and lg; left out");
        return radii;
    };
    for (key, value) in map {
        let here = pointer(path, key);
        let slot = match key.as_str() {
            "sm" => &mut radii.sm,
            "md" => &mut radii.md,
            "lg" => &mut radii.lg,
            _ => {
                warn(warnings, here, "isn't one of sm, md and lg; ignored");
                continue;
            }
        };
        match value.as_f64() {
            Some(px) if px.is_finite() => {
                let clamped = px.round().clamp(0.0, f64::from(MAX_RADIUS));
                if clamped != px.round() {
                    warn(warnings, here, format!("radii are 0 to {MAX_RADIUS} px; {clamped} used"));
                }
                *slot = Some(clamped as u8);
            }
            _ => warn(warnings, here, "should be a number of pixels; Silt's own stands in"),
        }
    }
    radii
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: &str = r##"{
        "name": "Ember",
        "themes": [{
            "id": "ember-dark", "name": "Ember Dark", "appearance": "dark",
            "colors": { "bg": "#1b1612", "fg-1": "#efe6d6", "accent": "#e0803a" }
        }]
    }"##;

    #[test]
    fn a_small_theme_reads_without_a_warning() {
        let (family, warnings) = parse(SMALL).expect("parses");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(family.themes[0].id.as_str(), "ember-dark");
        assert_eq!(family.themes[0].appearance, Appearance::Dark);
        assert_eq!(family.themes[0].colors.len(), 3);
    }

    #[test]
    fn written_back_it_reads_the_same() {
        let (family, _) = parse(SMALL).expect("parses");
        let (again, warnings) = parse(&family.to_json()).expect("parses again");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again, family);
        assert!(family.to_json().contains(SCHEMA_URL));
    }

    #[test]
    fn what_is_wrong_is_a_warning_with_its_place() {
        let json = r##"{
            "name": "Mixed", "extra": 1,
            "themes": [{
                "id": "mixed", "name": "Mixed", "appearance": "light",
                "colors": { "bg": "#fff", "fg-1": "#111", "accent": "#06c", "danger": "red", "sparkle": "#123" },
                "syntax": { "keyword": "#06c", "comment": { "color": "#777", "font_style": "oblique", "font_weight": 650 }, "tag": "#f00" },
                "fonts": { "ui": ["Inter", "x;}body{", "system-ui"], "mono": "JetBrains Mono" },
                "radii": { "sm": 2, "md": 99, "lg": "big" }
            }]
        }"##;
        let (family, warnings) = parse(json).expect("parses");
        let paths: Vec<_> = warnings.iter().map(|warning| warning.path.as_str()).collect();
        for expected in [
            "/extra",
            "/themes/0/colors/danger",
            "/themes/0/colors/sparkle",
            "/themes/0/syntax/comment/font_style",
            "/themes/0/syntax/tag",
            "/themes/0/fonts/ui/1",
            "/themes/0/fonts/mono",
            "/themes/0/radii/md",
            "/themes/0/radii/lg",
        ] {
            assert!(paths.contains(&expected), "{expected} not in {paths:?}");
        }
        let theme = &family.themes[0];
        assert!(!theme.colors.contains_key(&ColorToken::Danger));
        assert_eq!(theme.syntax[&SyntaxTag::Comment].font_weight, Some(700));
        let ui: Vec<_> = theme.fonts.ui.as_ref().expect("ui fonts").iter().map(FontName::as_str).collect();
        assert_eq!(ui, ["Inter", "system-ui"]);
        assert_eq!(theme.radii, Radii { sm: Some(2), md: Some(MAX_RADIUS), lg: None });
    }

    #[test]
    fn a_theme_without_what_it_needs_is_left_out() {
        let json = r##"{ "name": "Half", "themes": [
            { "id": "half-light", "name": "Half", "appearance": "light", "colors": { "bg": "#fff" } },
            { "id": "half-dark", "name": "Half", "appearance": "dusk", "colors": { "bg": "#000", "fg-1": "#fff", "accent": "#0cf" } },
            { "id": "Bad Id", "appearance": "dark", "colors": { "bg": "#000", "fg-1": "#fff", "accent": "#0cf" } },
            { "id": "whole", "name": "Whole", "appearance": "dark", "colors": { "bg": "#000", "fg-1": "#fff", "accent": "#0cf" } },
            { "id": "whole", "name": "Again", "appearance": "dark", "colors": { "bg": "#000", "fg-1": "#fff", "accent": "#0cf" } }
        ] }"##;
        let (family, warnings) = parse(json).expect("parses");
        assert_eq!(family.themes.len(), 1);
        assert_eq!(family.themes[0].id.as_str(), "whole");
        assert_eq!(warnings.len(), 4, "{warnings:?}");
    }

    #[test]
    fn a_file_with_no_usable_theme_is_refused() {
        assert!(matches!(parse("[]"), Err(ThemeError::Invalid { .. })));
        assert!(matches!(parse("{ \"themes\": [] }"), Err(ThemeError::Invalid { .. })));
        assert!(matches!(parse("{ \"name\": \"x\", \"themes\": [] }"), Err(ThemeError::NoThemes(_))));
        assert!(matches!(parse("{ nope"), Err(ThemeError::Json(_))));
    }

    #[test]
    fn ids_come_from_names() {
        assert_eq!(ThemeId::from_name("Rosé Pine Dawn").expect("id").as_str(), "rose-pine-dawn");
        assert_eq!(ThemeId::from_name("  Tokyo Night (Storm) ").expect("id").as_str(), "tokyo-night-storm");
        assert!(ThemeId::from_name("!!!").is_none());
    }
}
