//! A light and a dark theme as the CSS the page gets, after `nous.css`.
//!
//! The light block's selector is `:root:not([data-theme=dark])` and the dark
//! ones `:root:not([data-theme=light])` (under `prefers-color-scheme: dark`)
//! and `:root[data-theme=dark]`. All three outrank `nous.css`'s `:root`
//! wherever the style lands, and with no `data-theme` on a dark system the
//! dark block, coming later, wins. Each block sets every property, so
//! nothing of the light theme shows through the dark one.

use std::fmt::Write as _;

use crate::derive::Palette;
use crate::model::{Appearance, FontName};
use crate::token::{ColorToken, SyntaxTag};

const DEFAULT_UI: &str = "'IBM Plex Sans', ui-sans-serif, system-ui, -apple-system, 'Segoe UI', sans-serif";
const DEFAULT_MONO: &str = "'IBM Plex Mono', ui-monospace, 'SF Mono', Menlo, Consolas, monospace";
const DEFAULT_RADII: [u8; 3] = [2, 4, 8];

fn shadows(appearance: Appearance) -> [&'static str; 2] {
    match appearance {
        Appearance::Light => [
            "0 1px 2px rgb(20 14 8 / 0.04), 0 1px 1px rgb(20 14 8 / 0.06)",
            "0 8px 24px rgb(20 14 8 / 0.1), 0 2px 4px rgb(20 14 8 / 0.06)",
        ],
        Appearance::Dark => {
            ["0 1px 2px rgb(0 0 0 / 0.3), 0 1px 1px rgb(0 0 0 / 0.2)", "0 8px 24px rgb(0 0 0 / 0.4), 0 2px 4px rgb(0 0 0 / 0.25)"]
        }
    }
}

/// The CSS for `light` and `dark`, which the caller has checked are a light
/// and a dark theme.
pub fn compile(light: &Palette, dark: &Palette) -> String {
    format!(
        "{}{}:root:not([data-theme=dark]){{{}}}\n@media (prefers-color-scheme: dark){{:root:not([data-theme=light]){{{}}}}}\n:root[data-theme=dark]{{{}}}\n",
        font_faces(light),
        font_faces(dark),
        declarations(light),
        declarations(dark),
        declarations(dark),
    )
}

/// Each face the theme ships, from `theme-fonts/<id>/<file>`, relative to
/// the page so it resolves under the daemon's `/` and the hosted app's `/app/`.
fn font_faces(palette: &Palette) -> String {
    let mut out = String::new();
    for face in &palette.faces {
        let _ = writeln!(
            out,
            "@font-face{{font-family:\"{}\";src:url(theme-fonts/{}/{}) format(\"{}\");font-weight:{};font-style:{};font-display:swap}}",
            face.family.as_str(),
            palette.id,
            face.file.name(),
            face.file.format().css(),
            face.weight,
            face.style.as_str(),
        );
    }
    out
}

fn declarations(palette: &Palette) -> String {
    let mut out = String::new();
    for token in ColorToken::ALL {
        let _ = write!(out, "--{token}:{};", palette.get(*token));
    }
    let [raised, overlay] = shadows(palette.appearance);
    let _ = write!(out, "--shadow-1:{raised};--shadow-2:{overlay};");
    for tag in SyntaxTag::ALL {
        let style = palette.syntax[tag];
        let color = style.color.map_or_else(|| "initial".to_owned(), |color| color.to_string());
        let font_style = style.font_style.map_or("initial", |font_style| font_style.as_str());
        let weight = style.font_weight.map_or_else(|| "initial".to_owned(), |weight| weight.to_string());
        let _ = write!(out, "--syntax-{tag}:{color};--syntax-{tag}-style:{font_style};--syntax-{tag}-weight:{weight};");
    }
    let _ = write!(out, "--font-sans:{};", stack(palette.ui_font.as_deref(), "sans-serif", DEFAULT_UI));
    let _ = write!(out, "--font-mono:{};", stack(palette.mono_font.as_deref(), "monospace", DEFAULT_MONO));
    let radii = [palette.radii.sm, palette.radii.md, palette.radii.lg];
    for ((name, given), default) in ["sm", "md", "lg"].iter().zip(radii).zip(DEFAULT_RADII) {
        let _ = write!(out, "--radius-{name}:{}px;", given.unwrap_or(default));
    }
    out
}

/// Names quoted unless generic, ending in a generic family.
fn stack(names: Option<&[FontName]>, fallback: &str, default: &str) -> String {
    let Some(names) = names.filter(|names| !names.is_empty()) else { return default.to_owned() };
    let mut parts: Vec<String> = names
        .iter()
        .map(|name| if name.generic() { name.as_str().to_owned() } else { format!("\"{}\"", name.as_str()) })
        .collect();
    if !names.iter().any(FontName::generic) {
        parts.push(fallback.to_owned());
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::derive;
    use crate::model::parse;

    fn pair(json: &str) -> (Palette, Palette) {
        let (family, _) = parse(json).expect("parses");
        let find = |appearance| derive(family.themes.iter().find(|theme| theme.appearance == appearance).expect("theme"));
        (find(Appearance::Light), find(Appearance::Dark))
    }

    const PAIR: &str = r##"{ "name": "P", "themes": [
        { "id": "p-light", "name": "P Light", "appearance": "light",
          "colors": { "bg": "#f7f8f6", "fg-1": "#22302c", "accent": "#2f7d6d" },
          "fonts": { "ui": ["Inter", "system-ui"], "mono": ["JetBrains Mono"] }, "radii": { "md": 6 } },
        { "id": "p-dark", "name": "P Dark", "appearance": "dark",
          "colors": { "bg": "#151b20", "fg-1": "#dfe8ee", "accent": "#4fb3a9" } }
    ] }"##;

    #[test]
    fn each_block_sets_everything() {
        let (light, dark) = pair(PAIR);
        let css = compile(&light, &dark);
        let blocks: Vec<&str> = css.split('{').filter(|part| part.contains("--bg:")).collect();
        assert_eq!(blocks.len(), 3, "{css}");
        let count = |block: &str| block.matches(';').count();
        assert!(blocks.iter().all(|block| count(block) == count(blocks[0])), "{css}");
        assert!(css.contains("--font-sans:\"Inter\", system-ui;"), "{css}");
        assert!(css.contains("--font-mono:\"JetBrains Mono\", monospace;"), "{css}");
        assert!(css.contains("--radius-md:6px;") && css.contains("--radius-md:4px;"), "{css}");
        assert!(css.contains("--syntax-variable:initial;"), "{css}");
        assert!(css.starts_with(":root:not([data-theme=dark]){--bg:#f7f8f6;"), "{css}");
    }

    #[test]
    fn the_css_holds_nothing_a_file_could_smuggle_in() {
        let (light, dark) = pair(PAIR);
        let css = compile(&light, &dark);
        let allowed = |c: char| c.is_ascii_alphanumeric() || " #%(),.:;{}[]=\"'_-@/\n".contains(c);
        assert!(css.chars().all(allowed), "{:?}", css.chars().filter(|c| !allowed(*c)).collect::<String>());
        assert!(!css.contains('<') && !css.contains("url(") && !css.contains("@import"));
        assert!(!css.contains("/*"), "a comment would swallow the rest");
    }

    #[test]
    fn a_theme_s_faces_load_from_its_own_fonts_route_and_nothing_else() {
        let json = r##"{ "name": "F", "themes": [
            { "id": "f-light", "name": "F", "appearance": "light",
              "colors": { "bg": "#ffffff", "fg-1": "#111111", "accent": "#0055cc" },
              "fonts": { "ui": ["Harbour Sans", "system-ui"], "faces": [
                { "family": "Harbour Sans", "file": "fonts/harbour-400.woff2" },
                { "family": "Harbour Sans", "file": "fonts/harbour-700.woff2", "weight": 700, "style": "italic" },
                { "family": "x\"}body{", "file": "fonts/x.woff2" },
                { "family": "Evil", "file": "fonts/../../etc/passwd.woff2" },
                { "family": "Evil", "file": "fonts/a.svg" }
              ] } },
            { "id": "f-dark", "name": "F", "appearance": "dark", "colors": { "bg": "#000000", "fg-1": "#eeeeee", "accent": "#66aaff" } }
        ] }"##;
        let (light, dark) = pair(json);
        let css = compile(&light, &dark);
        assert!(css.starts_with(
            "@font-face{font-family:\"Harbour Sans\";src:url(theme-fonts/f-light/harbour-400.woff2) format(\"woff2\");font-weight:400;font-style:normal;font-display:swap}\n"
        ), "{css}");
        assert!(css.contains("url(theme-fonts/f-light/harbour-700.woff2) format(\"woff2\");font-weight:700;font-style:italic"));
        assert_eq!(css.matches("@font-face").count(), 2, "{css}");
        assert_eq!(css.matches("url(").count(), 2);
        assert!(css.match_indices("url(").all(|(at, _)| css[at..].starts_with("url(theme-fonts/f-light/")));
        let allowed = |c: char| c.is_ascii_alphanumeric() || " #%(),.:;{}[]=\"'_-@/\n".contains(c);
        assert!(css.chars().all(allowed));
        assert!(!css.contains("..") && !css.contains("passwd") && !css.contains("body{"));
    }
}
