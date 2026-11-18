//! Every token and tag a theme leaves out, worked out from what it sets, so a
//! theme of three colours is whole. Derived text is solved to 4.5:1; what
//! the theme sets is kept as written.

use std::collections::{BTreeMap, BTreeSet};

use crate::color::{Rgba, solve};
use crate::model::{Appearance, FontFace, FontName, FontStyle, Radii, SyntaxStyle, Theme, ThemeId};
use crate::token::{ColorToken, SyntaxTag};

/// What text needs on its ground (WCAG 2, AA).
pub const TEXT_RATIO: f64 = 4.5;
/// What icons, dots and other marks need.
pub const MARK_RATIO: f64 = 3.0;

/// A theme with every token and tag filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub id: ThemeId,
    pub appearance: Appearance,
    pub colors: BTreeMap<ColorToken, Rgba>,
    pub syntax: BTreeMap<SyntaxTag, SyntaxStyle>,
    pub ui_font: Option<Vec<FontName>>,
    pub mono_font: Option<Vec<FontName>>,
    pub radii: Radii,
    /// The faces it ships, loaded from `theme-fonts/<id>/`.
    pub faces: Vec<FontFace>,
    /// The tokens the theme left out.
    pub derived: BTreeSet<ColorToken>,
}

impl Palette {
    pub fn get(&self, token: ColorToken) -> Rgba {
        self.colors[&token]
    }
}

/// Nous's status colours, which stand in when a theme gives none.
fn nous_status(appearance: Appearance, token: ColorToken) -> Rgba {
    let hex = match (appearance, token) {
        (Appearance::Light, ColorToken::Ok) => (0x5f, 0x6e, 0x2d),
        (Appearance::Dark, ColorToken::Ok) => (0x8a, 0x9a, 0x4c),
        (_, ColorToken::Warn) => (0xc9, 0x98, 0x2e),
        (Appearance::Light, ColorToken::Danger) => (0x9b, 0x43, 0x26),
        (Appearance::Dark, ColorToken::Danger) => (0xd9, 0x7a, 0x59),
        (Appearance::Light, _) => (0x3f, 0x5c, 0x6e),
        (Appearance::Dark, _) => (0x7b, 0x98, 0xa8),
    };
    Rgba::rgb(hex.0, hex.1, hex.2)
}

pub fn derive(theme: &Theme) -> Palette {
    use ColorToken as T;
    let light = theme.appearance == Appearance::Light;
    let given = &theme.colors;
    let mut colors = given.clone();
    let pick = |light_amount: f64, dark_amount: f64| if light { light_amount } else { dark_amount };

    let bg = colors[&T::Bg];
    let fg = colors[&T::Fg1];
    let accent = colors[&T::Accent];
    let fill = |token: ColorToken, colors: &mut BTreeMap<ColorToken, Rgba>, value: Rgba| {
        colors.entry(token).or_insert(value);
    };

    let elevated = if light { bg.mix(Rgba::WHITE, 0.4) } else { bg.mix(fg, 0.04) };
    fill(T::BgElevated, &mut colors, elevated);
    fill(T::Surface1, &mut colors, bg.mix(fg, pick(0.05, 0.04)));
    fill(T::Surface2, &mut colors, bg.mix(fg, pick(0.10, 0.09)));
    fill(T::Surface3, &mut colors, bg.mix(fg, pick(0.16, 0.14)));
    fill(T::Border, &mut colors, bg.mix(fg, pick(0.17, 0.13)));
    fill(T::BorderStrong, &mut colors, bg.mix(fg, pick(0.6, 0.3)));
    fill(T::BorderSubtle, &mut colors, fg.with_alpha(pick(0.08, 0.06)));

    let grounds = [bg, colors[&T::BgElevated]];
    fill(T::Fg2, &mut colors, solve(fg.mix(bg, 0.25), &grounds, TEXT_RATIO));
    fill(T::Fg3, &mut colors, solve(fg.mix(bg, 0.4), &grounds, TEXT_RATIO));
    fill(T::AccentHover, &mut colors, accent.at_lightness(accent.lightness() + pick(-0.07, 0.07)));
    fill(T::AccentTint, &mut colors, accent.with_alpha(pick(0.10, 0.14)));
    fill(T::AccentFg, &mut colors, solve(accent, &grounds, TEXT_RATIO));
    let on_accent = [bg, fg, Rgba::BLACK, Rgba::WHITE]
        .into_iter()
        .max_by(|a, b| a.contrast_on(accent).total_cmp(&b.contrast_on(accent)))
        .expect("four candidates");
    fill(T::FgOnAccent, &mut colors, on_accent);

    for (status, tint) in [(T::Ok, T::OkTint), (T::Warn, T::WarnTint), (T::Danger, T::DangerTint), (T::Info, T::InfoTint)] {
        let ratio = if status == T::Warn { MARK_RATIO } else { TEXT_RATIO };
        let color = *colors.entry(status).or_insert_with(|| solve(nous_status(theme.appearance, status), &grounds, ratio));
        fill(tint, &mut colors, color.with_alpha(pick(0.12, 0.14)));
    }
    let warn_fg = solve(colors[&T::Warn], &grounds, TEXT_RATIO);
    fill(T::WarnFg, &mut colors, warn_fg);
    fill(T::Backdrop, &mut colors, Rgba { r: 20, g: 14, b: 8, a: 102 });

    let derived = ColorToken::ALL.iter().copied().filter(|token| !given.contains_key(token)).collect();
    let syntax = syntax(theme, &colors);
    Palette {
        id: theme.id.clone(),
        appearance: theme.appearance,
        colors,
        syntax,
        ui_font: theme.fonts.ui.clone(),
        mono_font: theme.fonts.mono.clone(),
        radii: theme.radii,
        faces: theme.fonts.faces.clone(),
        derived,
    }
}

/// Nous's mapping from tags to tokens, as `web/src/index.css` had it.
fn syntax(theme: &Theme, colors: &BTreeMap<ColorToken, Rgba>) -> BTreeMap<SyntaxTag, SyntaxStyle> {
    use SyntaxTag as S;
    let token = |token: ColorToken| Some(colors[&token]);
    SyntaxTag::ALL
        .iter()
        .map(|tag| {
            let default = match tag {
                S::Keyword => SyntaxStyle { color: token(ColorToken::AccentFg), ..SyntaxStyle::default() },
                S::String | S::StringSpecial => SyntaxStyle { color: token(ColorToken::Ok), ..SyntaxStyle::default() },
                S::Number | S::Constant => SyntaxStyle { color: token(ColorToken::Danger), ..SyntaxStyle::default() },
                S::Comment => {
                    SyntaxStyle { color: token(ColorToken::Fg3), font_style: Some(FontStyle::Italic), font_weight: None }
                }
                S::Type | S::Macro => SyntaxStyle { color: token(ColorToken::Info), ..SyntaxStyle::default() },
                S::Operator | S::Punctuation | S::Meta => SyntaxStyle { color: token(ColorToken::Fg2), ..SyntaxStyle::default() },
                S::Definition | S::Heading => SyntaxStyle { font_weight: Some(500), ..SyntaxStyle::default() },
                S::Emphasis => SyntaxStyle { font_style: Some(FontStyle::Italic), ..SyntaxStyle::default() },
                S::Strong => SyntaxStyle { font_weight: Some(600), ..SyntaxStyle::default() },
                S::Variable | S::Property | S::Link => SyntaxStyle::default(),
            };
            let style = match theme.syntax.get(tag) {
                Some(given) => SyntaxStyle {
                    color: given.color.or(default.color),
                    font_style: given.font_style.or(default.font_style),
                    font_weight: given.font_weight.or(default.font_weight),
                },
                None => default,
            };
            (*tag, style)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse;

    fn three(appearance: &str, bg: &str, fg: &str, accent: &str) -> Theme {
        let json = format!(
            r#"{{ "name": "T", "themes": [{{ "id": "t", "name": "T", "appearance": "{appearance}",
                "colors": {{ "bg": "{bg}", "fg-1": "{fg}", "accent": "{accent}" }} }}] }}"#
        );
        parse(&json).expect("parses").0.themes.remove(0)
    }

    #[test]
    fn three_colours_fill_every_token_and_tag() {
        for theme in [three("light", "#f7f8f6", "#22302c", "#2f7d6d"), three("dark", "#1b1612", "#efe6d6", "#e0803a")] {
            let palette = derive(&theme);
            assert_eq!(palette.colors.len(), ColorToken::ALL.len());
            assert_eq!(palette.syntax.len(), SyntaxTag::ALL.len());
            assert_eq!(palette.derived.len(), ColorToken::ALL.len() - 3);
            assert!(palette.get(ColorToken::Surface1) != palette.get(ColorToken::Bg));
            assert!(!palette.get(ColorToken::AccentTint).opaque());
        }
    }

    #[test]
    fn derived_text_reads_on_bg_even_from_a_faint_accent() {
        let theme = three("light", "#ffffff", "#222222", "#f6c177");
        let palette = derive(&theme);
        let bg = palette.get(ColorToken::Bg);
        for token in [
            ColorToken::Fg2,
            ColorToken::Fg3,
            ColorToken::AccentFg,
            ColorToken::WarnFg,
            ColorToken::Ok,
            ColorToken::Danger,
            ColorToken::Info,
        ] {
            assert!(palette.get(token).contrast_on(bg) >= TEXT_RATIO, "{token} {}", palette.get(token));
        }
        assert!(palette.get(ColorToken::FgOnAccent).contrast_on(palette.get(ColorToken::Accent)) >= TEXT_RATIO);
    }

    #[test]
    fn what_the_theme_sets_is_kept() {
        let mut theme = three("dark", "#1a1b26", "#c0caf5", "#7aa2f7");
        let faint = Rgba::parse("#565f89").expect("hex");
        theme.colors.insert(ColorToken::Fg3, faint);
        theme.syntax.insert(SyntaxTag::Comment, SyntaxStyle::color(faint));
        let palette = derive(&theme);
        assert_eq!(palette.get(ColorToken::Fg3), faint);
        assert!(!palette.derived.contains(&ColorToken::Fg3));
        let comment = palette.syntax[&SyntaxTag::Comment];
        assert_eq!(comment.color, Some(faint));
        assert_eq!(comment.font_style, Some(FontStyle::Italic), "the rest of the tag's default stays");
    }
}
