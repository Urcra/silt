//! Themes for Silt's UI (ADR 0098): the format, what a theme leaves out
//! derived, a light and a dark theme compiled to the CSS the page gets after
//! `nous.css`, contrast checks, the JSON Schema, and imports from VS Code and
//! Zed.

pub mod check;
pub mod color;
pub mod css;
pub mod derive;
pub mod font;
pub mod import;
pub mod model;
pub mod schema;
pub mod text;
pub mod token;

pub use check::{Finding, check, passes};
pub use color::Rgba;
pub use css::compile;
pub use derive::{Palette, derive};
pub use model::{Appearance, Family, FontName, Theme, ThemeError, ThemeId, Warning, parse};
pub use text::TextSettings;
pub use token::{ColorToken, SyntaxTag};

/// The themes Silt ships, by file name.
pub const BUNDLED: &[(&str, &str)] = &[
    ("nous.json", include_str!("../themes/nous.json")),
    ("fjord.json", include_str!("../themes/fjord.json")),
    ("ink.json", include_str!("../themes/ink.json")),
    ("catppuccin.json", include_str!("../themes/catppuccin.json")),
    ("gruvbox.json", include_str!("../themes/gruvbox.json")),
    ("rose-pine.json", include_str!("../themes/rose-pine.json")),
    ("tokyo-night.json", include_str!("../themes/tokyo-night.json")),
    ("solarized.json", include_str!("../themes/solarized.json")),
    ("nord.json", include_str!("../themes/nord.json")),
];

/// The theme shown when the one picked is gone.
pub const DEFAULT_LIGHT: &str = "nous-light";
pub const DEFAULT_DARK: &str = "nous-dark";

/// Silt's own families; each parses without a warning, as a test checks.
pub fn bundled() -> Vec<Family> {
    BUNDLED.iter().filter_map(|(_, json)| parse(json).ok()).map(|(family, _)| family).collect()
}

/// The CSS for a light and a dark theme.
pub fn theme_css(light: &Theme, dark: &Theme) -> String {
    compile(&derive(light), &derive(dark))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    #[test]
    fn every_bundled_theme_reads_clean_and_passes_its_checks() {
        let mut ids = BTreeSet::new();
        for (file, json) in BUNDLED {
            let (family, warnings) = parse(json).unwrap_or_else(|error| panic!("{file}: {error}"));
            assert!(warnings.is_empty(), "{file}: {warnings:?}");
            assert_eq!(&family.to_json(), json, "{file} isn't as Silt writes it; write it with Family::to_json");
            for theme in &family.themes {
                assert!(ids.insert(theme.id.clone()), "{file}: {} is used twice", theme.id);
                let findings = check(&derive(theme));
                let failing: Vec<_> = findings
                    .iter()
                    .filter(|f| f.kind == check::Kind::Text && !f.passes())
                    .map(|f| format!("{} {:.2}", f.pair, f.ratio))
                    .collect();
                assert!(failing.is_empty(), "{}: {failing:?}", theme.id);
            }
            if family.license.as_deref() != Some("AGPL-3.0-only") {
                assert!(
                    family.author.is_some() && family.source.is_some() && family.copyright.is_some(),
                    "{file} is a port without credits"
                );
            }
        }
        let ids: BTreeSet<_> = ids.iter().map(ThemeId::as_str).collect();
        assert!(ids.contains(DEFAULT_LIGHT) && ids.contains(DEFAULT_DARK));
    }

    /// `nous.css` is the web UI's and the server's; `nous.json` is the same
    /// look as a theme.
    #[test]
    fn nous_matches_nous_css() {
        let css = include_str!("../../../web/src/nous.css");
        let mut palette = BTreeMap::new();
        let mut pairs = BTreeMap::new();
        for line in css.lines().map(str::trim) {
            let Some((name, value)) = line.strip_prefix("--").and_then(|line| line.split_once(": ")) else { continue };
            let value = value.trim_end_matches(';');
            if let Some(pair) = value.strip_prefix("light-dark(").and_then(|value| value.strip_suffix(')')) {
                pairs.insert(name.to_owned(), split_pair(pair));
            } else if value.starts_with('#') || value.starts_with("rgb(") {
                palette.insert(format!("var(--{name})"), value.to_owned());
                pairs.insert(name.to_owned(), (value.to_owned(), value.to_owned()));
            } else if value.starts_with("var(") {
                pairs.insert(name.to_owned(), (value.to_owned(), value.to_owned()));
            }
        }
        let resolve = |value: &str| css_color(palette.get(value.trim()).map_or(value.trim(), String::as_str));
        let nous = parse(BUNDLED[0].1).expect("nous").0;
        let mut compared = 0;
        for theme in &nous.themes {
            let derived = derive(theme);
            for token in ColorToken::ALL {
                let Some((light, dark)) = pairs.get(token.as_str()) else { continue };
                let expected = if theme.appearance == Appearance::Light { light } else { dark };
                assert_eq!(Some(derived.get(*token)), resolve(expected), "{token} in {}", theme.id);
                compared += 1;
            }
        }
        assert_eq!(compared, 2 * ColorToken::ALL.len(), "every token is in nous.css");
    }

    fn split_pair(pair: &str) -> (String, String) {
        let mut depth = 0;
        for (at, c) in pair.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => return (pair[..at].trim().to_owned(), pair[at + 1..].trim().to_owned()),
                _ => {}
            }
        }
        panic!("{pair} isn't a pair")
    }

    /// `#rrggbb` or `rgb(r g b / a)`.
    fn css_color(value: &str) -> Option<Rgba> {
        if value.starts_with('#') {
            return Rgba::parse(value).ok();
        }
        let inner = value.strip_prefix("rgb(")?.strip_suffix(')')?;
        let (rgb, alpha) = inner.split_once('/').map_or((inner, 1.0), |(rgb, alpha)| (rgb, alpha.trim().parse().unwrap_or(1.0)));
        let channels: Vec<u8> = rgb.split_whitespace().map(|channel| channel.parse().ok()).collect::<Option<_>>()?;
        let [r, g, b] = channels[..] else { return None };
        Some(Rgba::rgb(r, g, b).with_alpha(alpha))
    }

    #[test]
    fn theme_schema() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/reference/theme.schema.json");
        // Sorted here, as another crate's `preserve_order` can change how a map keeps its keys.
        fn sorted(value: serde_json::Value) -> serde_json::Value {
            match value {
                serde_json::Value::Object(map) => {
                    let mut entries: Vec<_> = map.into_iter().collect();
                    entries.sort_by(|a, b| a.0.cmp(&b.0));
                    serde_json::Value::Object(entries.into_iter().map(|(key, value)| (key, sorted(value))).collect())
                }
                serde_json::Value::Array(items) => serde_json::Value::Array(items.into_iter().map(sorted).collect()),
                other => other,
            }
        }
        let mut expected = serde_json::to_string_pretty(&sorted(schema::schema())).expect("schema");
        expected.push('\n');
        if std::env::var_os("UPDATE_THEME_SCHEMA").is_some() {
            std::fs::write(path, &expected).expect("write the schema");
        }
        let on_disk = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            on_disk == expected,
            "docs/reference/theme.schema.json is out of date: UPDATE_THEME_SCHEMA=1 cargo test -p silt-theme theme_schema"
        );
    }
}
