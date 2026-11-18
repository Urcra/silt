//! A VS Code colour theme: `colors` by workbench key, `tokenColors` by
//! TextMate scope.

use std::collections::BTreeMap;

use serde_json::Value;

use super::textmate::{self, Rule};
use super::{Draft, ImportError, Imported, Options, id_for, map_colors};
use crate::color::Rgba;
use crate::model::{Appearance, Family, FontStyle, Fonts, Radii, SyntaxStyle, Theme, text};
use crate::token::{ColorToken as T, SyntaxTag as S};

/// Each token by the workbench keys that come closest, best first.
const COLORS: &[(T, &[&str])] = &[
    (T::Bg, &["editor.background"]),
    (T::BgElevated, &["editorWidget.background", "input.background", "dropdown.background"]),
    (T::Surface1, &["sideBar.background", "panel.background", "activityBar.background"]),
    (T::Surface2, &["list.hoverBackground", "editor.lineHighlightBackground"]),
    (T::Surface3, &["list.activeSelectionBackground", "list.inactiveSelectionBackground"]),
    (T::Fg1, &["editor.foreground", "foreground"]),
    (T::Fg2, &["descriptionForeground", "sideBar.foreground", "tab.inactiveForeground"]),
    (T::FgOnAccent, &["button.foreground"]),
    (T::Border, &["panel.border", "sideBar.border", "editorGroup.border", "widget.border"]),
    (T::BorderStrong, &["input.border", "contrastBorder", "editorWidget.border"]),
    (
        T::Accent,
        &["button.background", "focusBorder", "textLink.foreground", "activityBarBadge.background", "progressBar.background"],
    ),
    (T::AccentHover, &["button.hoverBackground"]),
    (T::AccentFg, &["textLink.foreground"]),
    (T::Ok, &["gitDecoration.addedResourceForeground", "terminal.ansiGreen"]),
    (T::WarnFg, &["editorWarning.foreground", "list.warningForeground"]),
    (T::Warn, &["editorWarning.foreground", "list.warningForeground", "terminal.ansiYellow"]),
    (T::Danger, &["editorError.foreground", "errorForeground", "gitDecoration.deletedResourceForeground", "terminal.ansiRed"]),
    (T::Info, &["editorInfo.foreground", "terminal.ansiBlue"]),
    (T::OkTint, &["diffEditor.insertedTextBackground", "diffEditor.insertedLineBackground"]),
    (T::DangerTint, &["diffEditor.removedTextBackground", "diffEditor.removedLineBackground"]),
];

/// Each tag by the scopes whose rule it takes, best first.
const SCOPES: &[(S, &[&str])] = &[
    (S::Keyword, &["keyword.control", "keyword", "storage.type", "storage.modifier"]),
    (S::String, &["string.quoted", "string"]),
    (S::StringSpecial, &["string.regexp", "constant.character.escape"]),
    (S::Number, &["constant.numeric"]),
    (S::Constant, &["constant.language", "constant"]),
    (S::Comment, &["comment.line", "comment"]),
    (S::Type, &["entity.name.type", "entity.name.class", "support.type", "support.class"]),
    (S::Macro, &["entity.name.function.macro", "support.function.macro"]),
    (S::Variable, &["variable.other", "variable"]),
    (S::Definition, &["entity.name.function", "meta.definition.variable"]),
    (S::Property, &["variable.other.property", "variable.other.member", "support.type.property-name", "meta.property-name"]),
    (S::Operator, &["keyword.operator"]),
    (S::Punctuation, &["punctuation.separator", "punctuation"]),
    (S::Meta, &["meta.attribute", "meta.annotation", "meta.preprocessor"]),
    (S::Heading, &["markup.heading", "entity.name.section"]),
    (S::Link, &["markup.underline.link", "string.other.link"]),
    (S::Emphasis, &["markup.italic"]),
    (S::Strong, &["markup.bold"]),
];

pub(super) fn import(value: &Value, options: &Options) -> Result<Imported, ImportError> {
    let mut notes = Vec::new();
    let mut ignored = Vec::new();
    let name = value.get("name").and_then(Value::as_str).map(text).filter(|name| !name.is_empty()).unwrap_or_else(|| {
        notes.push("the theme has no name; called it Imported".to_owned());
        "Imported".to_owned()
    });
    let raw = value.get("colors").and_then(Value::as_object);
    let lookup = |key: &str| raw.and_then(|colors| colors.get(key)).and_then(Value::as_str).and_then(|hex| Rgba::parse(hex).ok());
    let (mut colors, used) = map_colors(lookup, COLORS);
    if let Some(raw) = raw {
        let unused = raw.keys().filter(|key| !used.contains(*key)).count();
        if unused > 0 {
            ignored.push(format!("{unused} colours Silt has no place for"));
        }
    }

    let appearance = options.appearance.or_else(|| match value.get("type").and_then(Value::as_str) {
        Some("light" | "hcLight") => Some(Appearance::Light),
        Some("dark" | "hc" | "hcDark") => Some(Appearance::Dark),
        _ => None,
    });
    let appearance = appearance.unwrap_or_else(|| {
        let guessed = colors
            .get(&T::Bg)
            .map_or(Appearance::Dark, |bg| if bg.lightness() > 0.6 { Appearance::Light } else { Appearance::Dark });
        notes.push(format!(
            "the theme doesn't say whether it's light or dark; {guessed} from its background (--appearance to choose)"
        ));
        guessed
    });
    for (token, default) in
        [(T::Bg, ("#1e1e1e", "#ffffff")), (T::Fg1, ("#d4d4d4", "#3b3b3b")), (T::Accent, ("#0078d4", "#005fb8"))]
    {
        if let std::collections::btree_map::Entry::Vacant(slot) = colors.entry(token) {
            let hex = if appearance == Appearance::Dark { default.0 } else { default.1 };
            slot.insert(Rgba::parse(hex).expect("a default colour"));
            notes.push(format!("no colour for {token}; VS Code's default {hex}"));
        }
    }

    let rules = match value.get("tokenColors") {
        Some(Value::String(path)) => {
            ignored.push(format!("tokenColors from another file ({path}); import that file's rules by pasting them in"));
            Vec::new()
        }
        Some(token_colors) => textmate::rules(token_colors),
        None => Vec::new(),
    };
    for key in ["semanticTokenColors", "include"] {
        if value.get(key).is_some() {
            ignored.push(key.to_owned());
        }
    }
    let syntax = syntax(&rules);

    let id =
        id_for(&name, options, None).ok_or_else(|| ImportError::NoThemes(format!("{name:?} makes no id; give one with --to")))?;
    let theme = Theme { id, name: name.clone(), appearance, colors, syntax, fonts: Fonts::default(), radii: Radii::default() };
    Ok(Draft { family: Family::new(name, vec![theme]), ignored, notes }.into())
}

fn syntax(rules: &[Rule]) -> BTreeMap<S, SyntaxStyle> {
    let mut styles = BTreeMap::new();
    for (tag, scopes) in SCOPES {
        let color = scopes.iter().find_map(|scope| {
            textmate::best(rules, scope, |rule| rule.foreground.is_some())
                .and_then(|rule| rule.foreground.as_deref())
                .and_then(|hex| Rgba::parse(hex).ok())
        });
        let style = scopes.iter().find_map(|scope| textmate::best(rules, scope, |rule| rule.font_style.is_some()));
        let font_style = style.and_then(|rule| rule.font_style.as_deref());
        let style = SyntaxStyle {
            color,
            font_style: font_style.map(|style| if style.contains("italic") { FontStyle::Italic } else { FontStyle::Normal }),
            font_weight: font_style.and_then(|style| style.contains("bold").then_some(700)),
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
    use crate::token::{ColorToken, SyntaxTag};

    const NIGHT: &str = include_str!("../../tests/fixtures/vscode-night.jsonc");

    #[test]
    fn a_vscode_theme_maps_its_colours_and_scopes() {
        let imported = import(NIGHT, &Options::default()).expect("imports");
        assert_eq!(imported.format, Format::VsCode);
        let theme = &imported.family.themes[0];
        assert_eq!(theme.id.as_str(), "fixture-night");
        assert_eq!(theme.colors[&ColorToken::Bg].to_string(), "#1a1b26");
        assert_eq!(theme.colors[&ColorToken::Accent].to_string(), "#3d59a1");
        assert_eq!(theme.colors[&ColorToken::Surface2].to_string(), "#222538", "a translucent hover is laid over bg");
        assert_eq!(theme.syntax[&SyntaxTag::Keyword].color.map(|c| c.to_string()).as_deref(), Some("#bb9af7"));
        assert_eq!(theme.syntax[&SyntaxTag::Comment].color.map(|c| c.to_string()).as_deref(), Some("#565f89"));
        assert_eq!(theme.syntax[&SyntaxTag::Comment].font_style, Some(crate::model::FontStyle::Italic));
        assert_eq!(theme.syntax[&SyntaxTag::Definition].color.map(|c| c.to_string()).as_deref(), Some("#7aa2f7"));
        assert!(imported.ignored.iter().any(|note| note == "semanticTokenColors"));
        assert!(imported.ignored.iter().any(|note| note.contains("no place for")));
        assert!(!imported.mapped[0].derived.is_empty());
    }

    #[test]
    fn a_theme_that_doesnt_say_takes_its_appearance_from_its_background() {
        let imported =
            import(r##"{ "colors": { "editor.background": "#fdf6e3", "editor.foreground": "#657b83" } }"##, &Options::default())
                .expect("imports");
        assert_eq!(imported.family.themes[0].appearance, crate::model::Appearance::Light);
        assert!(imported.notes.iter().any(|note| note.contains("light or dark")));
        assert!(imported.notes.iter().any(|note| note.contains("no colour for accent")));
    }
}
