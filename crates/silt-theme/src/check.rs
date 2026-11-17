//! Whether a theme's text reads on its grounds. Text that fails is a
//! problem; marks that fail (`warn`, used for dots and icons) are advice.

use serde::Serialize;

use crate::color::Rgba;
use crate::derive::{MARK_RATIO, Palette, TEXT_RATIO};
use crate::token::{ColorToken, SyntaxTag};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Text,
    Mark,
}

/// One pair measured.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// What's on what: `fg-3 on bg`, `syntax comment on bg-elevated`.
    pub pair: String,
    pub fore: Rgba,
    pub back: Rgba,
    pub ratio: f64,
    pub needs: f64,
    pub kind: Kind,
    /// The foreground was derived rather than set by the theme.
    pub derived: bool,
}

impl Finding {
    pub fn passes(&self) -> bool {
        self.ratio >= self.needs
    }
}

/// Every pair, passing or not.
pub fn check(palette: &Palette) -> Vec<Finding> {
    use ColorToken as T;
    let mut findings = Vec::new();
    let mut measure = |fore_name: String, fore: Rgba, back_token: ColorToken, kind: Kind, derived: bool| {
        let back = palette.get(back_token);
        let needs = if kind == Kind::Text { TEXT_RATIO } else { MARK_RATIO };
        findings.push(Finding {
            pair: format!("{fore_name} on {back_token}"),
            fore,
            back,
            ratio: fore.contrast_on(back),
            needs,
            kind,
            derived,
        });
    };
    let grounds = [T::Bg, T::BgElevated];
    for back in [T::Bg, T::BgElevated, T::Surface1, T::Surface2] {
        measure(T::Fg1.to_string(), palette.get(T::Fg1), back, Kind::Text, false);
    }
    for token in [T::Fg2, T::Fg3, T::AccentFg, T::Ok, T::Danger, T::Info] {
        for back in grounds {
            measure(token.to_string(), palette.get(token), back, Kind::Text, palette.derived.contains(&token));
        }
    }
    measure(T::Warn.to_string(), palette.get(T::Warn), T::Bg, Kind::Mark, palette.derived.contains(&T::Warn));
    measure(
        T::FgOnAccent.to_string(),
        palette.get(T::FgOnAccent),
        T::Accent,
        Kind::Text,
        palette.derived.contains(&T::FgOnAccent),
    );
    for tag in SyntaxTag::ALL {
        let Some(color) = palette.syntax[tag].color else { continue };
        for back in grounds {
            measure(format!("syntax {tag}"), color, back, Kind::Text, false);
        }
    }
    findings
}

/// Whether every text pair passes; marks are advice.
pub fn passes(findings: &[Finding]) -> bool {
    findings.iter().all(|finding| finding.kind == Kind::Mark || finding.passes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::derive;
    use crate::model::parse;

    #[test]
    fn a_faint_comment_fails_and_its_pair_is_named() {
        let json = r##"{ "name": "T", "themes": [{ "id": "t", "name": "T", "appearance": "dark",
            "colors": { "bg": "#1a1b26", "fg-1": "#c0caf5", "accent": "#7aa2f7" },
            "syntax": { "comment": "#565f89" } }] }"##;
        let palette = derive(&parse(json).expect("parses").0.themes[0]);
        let findings = check(&palette);
        assert!(!passes(&findings));
        let failing: Vec<_> = findings.iter().filter(|finding| !finding.passes()).map(|finding| finding.pair.as_str()).collect();
        assert_eq!(failing, ["syntax comment on bg", "syntax comment on bg-elevated"]);
    }
}
