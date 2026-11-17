//! A Zed theme family: each theme's `style`, with `syntax` by Zed's
//! highlight names.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{Draft, ImportError, Imported, Options, id_for, map_colors};
use crate::color::Rgba;
use crate::model::{Appearance, Family, FontStyle, Fonts, Radii, SyntaxStyle, Theme, text};
use crate::token::{ColorToken as T, SyntaxTag as S};

const COLORS: &[(T, &[&str])] = &[
    (T::Bg, &["editor.background", "background"]),
    (T::BgElevated, &["elevated_surface.background", "editor.background"]),
    (T::Surface1, &["surface.background", "panel.background"]),
    (T::Surface2, &["element.hover", "ghost_element.hover"]),
    (T::Surface3, &["element.selected", "element.active"]),
    (T::Fg1, &["text", "editor.foreground"]),
    (T::Fg2, &["text.muted"]),
    (T::Fg3, &["text.placeholder", "text.disabled"]),
    (T::Border, &["border.variant", "border"]),
    (T::BorderStrong, &["border"]),
    (T::Accent, &["text.accent", "icon.accent", "border.focused"]),
    (T::AccentFg, &["text.accent"]),
    (T::Ok, &["created", "success"]),
    (T::Warn, &["warning", "modified"]),
    (T::Danger, &["error", "deleted"]),
    (T::Info, &["info", "hint"]),
    (T::OkTint, &["created.background", "success.background"]),
    (T::WarnTint, &["warning.background"]),
    (T::DangerTint, &["deleted.background", "error.background"]),
    (T::InfoTint, &["info.background"]),
];

/// Each tag by Zed's highlight names, best first.
const NAMES: &[(S, &[&str])] = &[
    (S::Keyword, &["keyword"]),
    (S::String, &["string"]),
    (S::StringSpecial, &["string.regex", "string.escape", "string.special"]),
    (S::Number, &["number"]),
    (S::Constant, &["boolean", "constant"]),
    (S::Comment, &["comment"]),
    (S::Type, &["type", "enum"]),
    (S::Macro, &["preproc"]),
    (S::Variable, &["variable"]),
    (S::Definition, &["function"]),
    (S::Property, &["property"]),
    (S::Operator, &["operator"]),
    (S::Punctuation, &["punctuation", "punctuation.delimiter"]),
    (S::Meta, &["attribute", "preproc"]),
    (S::Heading, &["title"]),
    (S::Link, &["link_uri", "link_text"]),
    (S::Emphasis, &["emphasis"]),
    (S::Strong, &["emphasis.strong"]),
];

pub(super) fn import(value: &Value, options: &Options) -> Result<Imported, ImportError> {
    let mut notes = Vec::new();
    let mut ignored = Vec::new();
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(text)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Imported".to_owned());
    let entries = value.get("themes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut themes = Vec::new();
    for entry in entries {
        let theme_name = entry.get("name").and_then(Value::as_str).map(text).unwrap_or_else(|| name.clone());
        let appearance = match entry.get("appearance").and_then(Value::as_str) {
            Some("light") => Appearance::Light,
            Some("dark") => Appearance::Dark,
            other => {
                notes.push(format!("{theme_name}: appearance {other:?} isn't light or dark; left out"));
                continue;
            }
        };
        let style = entry.get("style").and_then(Value::as_object);
        let lookup =
            |key: &str| style.and_then(|style| style.get(key)).and_then(Value::as_str).and_then(|hex| Rgba::parse(hex).ok());
        let (colors, _) = map_colors(lookup, COLORS);
        let missing: Vec<_> = [T::Bg, T::Fg1, T::Accent]
            .into_iter()
            .filter(|token| !colors.contains_key(token))
            .map(|token| token.as_str())
            .collect();
        if !missing.is_empty() {
            notes.push(format!("{theme_name}: no colour for {}; left out", missing.join(", ")));
            continue;
        }
        let suffix = (entries.len() > 1 && options.id.is_some()).then(|| appearance.as_str());
        let id_source = if options.id.is_some() { &name } else { &theme_name };
        let Some(id) = id_for(id_source, options, suffix) else {
            notes.push(format!("{theme_name}: makes no id; left out"));
            continue;
        };
        if themes.iter().any(|theme: &Theme| theme.id == id) {
            notes.push(format!("{theme_name}: {id} is taken by another theme in the family; left out"));
            continue;
        }
        let syntax = syntax(style.and_then(|style| style.get("syntax")).and_then(Value::as_object));
        themes.push(Theme { id, name: theme_name, appearance, colors, syntax, fonts: Fonts::default(), radii: Radii::default() });
    }
    if entries.iter().any(|entry| entry.pointer("/style/players").is_some()) {
        ignored.push("players (Silt's collaborators take colours of their own)".to_owned());
    }
    if themes.is_empty() {
        return Err(ImportError::NoThemes(notes.join("; ")));
    }
    let mut family = Family::new(name, themes);
    family.author = value.get("author").and_then(Value::as_str).map(text);
    Ok(Draft { family, ignored, notes }.into())
}

fn syntax(raw: Option<&serde_json::Map<String, Value>>) -> BTreeMap<S, SyntaxStyle> {
    let mut styles = BTreeMap::new();
    let Some(raw) = raw else { return styles };
    for (tag, names) in NAMES {
        let Some(entry) = names.iter().find_map(|name| raw.get(*name)) else { continue };
        let style = SyntaxStyle {
            color: entry.get("color").and_then(Value::as_str).and_then(|hex| Rgba::parse(hex).ok()),
            font_style: match entry.get("font_style").and_then(Value::as_str) {
                Some("italic" | "oblique") => Some(FontStyle::Italic),
                Some("normal") => Some(FontStyle::Normal),
                _ => None,
            },
            font_weight: entry
                .get("font_weight")
                .and_then(Value::as_f64)
                .filter(|weight| (100.0..=900.0).contains(weight))
                .map(|weight| ((weight / 100.0).round() * 100.0) as u16),
        };
        if style != SyntaxStyle::default() {
            styles.insert(*tag, style);
        }
    }
    styles
}

#[cfg(test)]
mod tests {
    use crate::import::{Format, Options, import};
    use crate::model::{Appearance, FontStyle};
    use crate::token::{ColorToken, SyntaxTag};

    const HARBOUR: &str = include_str!("../../tests/fixtures/zed-family.json");

    #[test]
    fn a_zed_family_keeps_its_light_and_dark_themes() {
        let imported = import(HARBOUR, &Options::default()).expect("imports");
        assert_eq!(imported.format, Format::Zed);
        let ids: Vec<_> = imported.family.themes.iter().map(|theme| theme.id.as_str()).collect();
        assert_eq!(ids, ["fixture-harbour-light", "fixture-harbour-dark"]);
        assert_eq!(imported.family.author.as_deref(), Some("Silt's tests"));
        let light = &imported.family.themes[0];
        assert_eq!(light.appearance, Appearance::Light);
        assert_eq!(light.colors[&ColorToken::Bg].to_string(), "#f7f8f6");
        assert_eq!(light.colors[&ColorToken::Border].to_string(), "#cfd3cb");
        assert_eq!(light.colors[&ColorToken::BorderStrong].to_string(), "#9aa39d");
        assert!(!light.colors.contains_key(&ColorToken::Info), "a null is left to derivation");
        assert_eq!(light.syntax[&SyntaxTag::Keyword].font_weight, Some(600));
        assert_eq!(light.syntax[&SyntaxTag::Comment].font_style, Some(FontStyle::Italic));
        assert_eq!(light.syntax[&SyntaxTag::Constant].color.map(|c| c.to_string()).as_deref(), Some("#9a4f12"));
        assert!(!light.syntax.contains_key(&SyntaxTag::Variable));
        assert!(imported.notes.iter().any(|note| note.starts_with("Broken")));
        assert!(imported.ignored.iter().any(|note| note.starts_with("players")));
    }

    #[test]
    fn a_chosen_id_names_each_theme_by_its_appearance() {
        let options = Options { id: Some("harbour".parse().expect("id")), appearance: None };
        let imported = import(HARBOUR, &options).expect("imports");
        let ids: Vec<_> = imported.family.themes.iter().map(|theme| theme.id.as_str()).collect();
        assert_eq!(ids, ["harbour-light", "harbour-dark"]);
    }
}
