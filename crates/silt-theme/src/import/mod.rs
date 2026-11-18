//! Themes from VS Code and Zed, read once into Silt's format: what maps is
//! kept, what Silt has no place for is listed, and the rest is derived.

pub mod jsonc;
pub mod textmate;
mod vscode;
mod zed;

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

use crate::check::{Kind, check};
use crate::color::{Rgba, solve};
use crate::derive::{TEXT_RATIO, derive};
use crate::model::{Appearance, Family, Theme, ThemeId};
use crate::token::{ColorToken, SyntaxTag};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    VsCode,
    Zed,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Self::VsCode => "VS Code",
            Self::Zed => "Zed",
        }
    }
}

/// What the person can choose about an import.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The id of the theme, or the stem of each id in a family.
    pub id: Option<ThemeId>,
    /// For a VS Code theme that doesn't say.
    pub appearance: Option<Appearance>,
}

/// What became of one theme in the import.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mapped {
    pub id: ThemeId,
    pub colors: usize,
    pub syntax: usize,
    pub derived: Vec<ColorToken>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub format: Format,
    pub family: Family,
    pub mapped: Vec<Mapped>,
    /// What the file has that Silt doesn't use.
    pub ignored: Vec<String>,
    /// What Silt had to guess.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("not JSON, even allowing comments: {0}")]
    Json(String),
    #[error("already in Silt's format; put it in the themes folder as it is")]
    AlreadySilt,
    #[error("neither a VS Code theme (colors, tokenColors) nor a Zed theme family (themes with a style)")]
    Unknown,
    #[error("no theme in the file could be read: {0}")]
    NoThemes(String),
}

/// Reads a VS Code theme or a Zed theme family.
pub fn import(text: &str, options: &Options) -> Result<Imported, ImportError> {
    let value: Value = serde_json::from_str(&jsonc::strip(text)).map_err(|error| ImportError::Json(error.to_string()))?;
    let themes = value.get("themes").and_then(Value::as_array);
    let (format, mut imported) = if themes.is_some_and(|themes| themes.iter().any(|theme| theme.get("style").is_some())) {
        (Format::Zed, zed::import(&value, options)?)
    } else if themes.is_some() {
        return Err(ImportError::AlreadySilt);
    } else if value.get("colors").is_some() || value.get("tokenColors").is_some() {
        (Format::VsCode, vscode::import(&value, options)?)
    } else {
        return Err(ImportError::Unknown);
    };
    imported.format = format;
    imported.mapped = imported
        .family
        .themes
        .iter()
        .map(|theme| Mapped {
            id: theme.id.clone(),
            colors: theme.colors.len(),
            syntax: theme.syntax.len(),
            derived: derive(theme).derived.into_iter().collect(),
        })
        .collect();
    Ok(imported)
}

/// What a source file's themes become before the format is known.
pub(crate) struct Draft {
    pub family: Family,
    pub ignored: Vec<String>,
    pub notes: Vec<String>,
}

impl From<Draft> for Imported {
    fn from(draft: Draft) -> Self {
        Self { format: Format::VsCode, family: draft.family, mapped: Vec::new(), ignored: draft.ignored, notes: draft.notes }
    }
}

/// Tokens whose alpha means something; the rest are laid over `bg`.
fn keeps_alpha(token: ColorToken) -> bool {
    matches!(
        token,
        ColorToken::BorderSubtle
            | ColorToken::AccentTint
            | ColorToken::OkTint
            | ColorToken::WarnTint
            | ColorToken::DangerTint
            | ColorToken::InfoTint
            | ColorToken::Backdrop
    )
}

/// A source's colours for each token, by the first key in its list that the
/// source has, translucent ones laid over the background where Silt needs a
/// solid colour.
pub(crate) fn map_colors(
    lookup: impl Fn(&str) -> Option<Rgba>,
    table: &[(ColorToken, &[&str])],
) -> (BTreeMap<ColorToken, Rgba>, BTreeSet<String>) {
    let mut colors = BTreeMap::new();
    let mut used = BTreeSet::new();
    for (token, keys) in table {
        if let Some((key, color)) = keys.iter().find_map(|key| lookup(key).map(|color| (*key, color))) {
            colors.insert(*token, color);
            used.insert(key.to_owned());
        }
    }
    if let Some(bg) = colors.get(&ColorToken::Bg).copied() {
        let bg = Rgba { a: 255, ..bg };
        colors.insert(ColorToken::Bg, bg);
        for (token, color) in colors.iter_mut() {
            if !keeps_alpha(*token) && !color.opaque() {
                *color = color.over(bg);
            }
        }
    }
    (colors, used)
}

/// An id for a theme from its name, or from the person's choice; `suffix`
/// tells a family's themes apart.
pub(crate) fn id_for(name: &str, options: &Options, suffix: Option<&str>) -> Option<ThemeId> {
    let stem = options.id.clone().or_else(|| ThemeId::from_name(name))?;
    match suffix {
        None => Some(stem),
        Some(suffix) => ThemeId::from_name(&format!("{stem} {suffix}")),
    }
}

/// A change `fix_contrast` made.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fix {
    pub theme: ThemeId,
    pub what: String,
    pub from: Rgba,
    pub to: Rgba,
}

/// Moves the colours a theme sets that fail as text until they read, in
/// OKLab lightness, keeping their hue.
pub fn fix_contrast(family: &mut Family) -> Vec<Fix> {
    let mut fixes = Vec::new();
    for theme in &mut family.themes {
        fix_theme(theme, &mut fixes);
    }
    fixes
}

fn fix_theme(theme: &mut Theme, fixes: &mut Vec<Fix>) {
    let palette = derive(theme);
    let grounds = [palette.get(ColorToken::Bg), palette.get(ColorToken::BgElevated)];
    let failing: BTreeSet<String> = check(&palette)
        .into_iter()
        .filter(|finding| finding.kind == Kind::Text && !finding.passes() && !finding.derived)
        .map(|finding| finding.pair.split(" on ").next().unwrap_or_default().to_owned())
        .collect();
    let id = theme.id.clone();
    for token in ColorToken::ALL.iter().filter(|token| failing.contains(token.as_str())) {
        let Some(color) = theme.colors.get(token).copied() else { continue };
        let fixed = if *token == ColorToken::FgOnAccent {
            solve(color, &[palette.get(ColorToken::Accent)], TEXT_RATIO)
        } else if *token == ColorToken::Fg1 {
            solve(
                color,
                &[grounds[0], grounds[1], palette.get(ColorToken::Surface1), palette.get(ColorToken::Surface2)],
                TEXT_RATIO,
            )
        } else {
            solve(color, &grounds, TEXT_RATIO)
        };
        if fixed != color {
            theme.colors.insert(*token, fixed);
            fixes.push(Fix { theme: id.clone(), what: token.to_string(), from: color, to: fixed });
        }
    }
    for tag in SyntaxTag::ALL.iter().filter(|tag| failing.contains(&format!("syntax {tag}"))) {
        let Some(style) = theme.syntax.get_mut(tag) else { continue };
        let Some(color) = style.color else { continue };
        let fixed = solve(color, &grounds, TEXT_RATIO);
        if fixed != color {
            style.color = Some(fixed);
            fixes.push(Fix { theme: id.clone(), what: format!("syntax {tag}"), from: color, to: fixed });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::passes;

    #[test]
    fn fixing_contrast_moves_only_what_fails() {
        let mut imported = import(include_str!("../../tests/fixtures/vscode-night.jsonc"), &Options::default()).expect("imports");
        let before = imported.family.themes[0].clone();
        assert!(!passes(&check(&derive(&before))));
        let fixes = fix_contrast(&mut imported.family);
        let after = &imported.family.themes[0];
        assert!(
            passes(&check(&derive(after))),
            "{:?}",
            check(&derive(after)).into_iter().filter(|f| !f.passes()).collect::<Vec<_>>()
        );
        let fixed: Vec<_> = fixes.iter().map(|fix| fix.what.as_str()).collect();
        assert!(fixed.contains(&"syntax comment"), "{fixed:?}");
        assert_eq!(after.colors[&ColorToken::Bg], before.colors[&ColorToken::Bg]);
        assert_eq!(after.syntax[&SyntaxTag::Keyword], before.syntax[&SyntaxTag::Keyword]);
    }

    #[test]
    fn a_silt_theme_or_anything_else_is_refused() {
        let silt = crate::BUNDLED[0].1;
        assert_eq!(import(silt, &Options::default()), Err(ImportError::AlreadySilt));
        assert_eq!(import("{\"a\": 1}", &Options::default()), Err(ImportError::Unknown));
        assert!(matches!(import("{ nope", &Options::default()), Err(ImportError::Json(_))));
    }
}
