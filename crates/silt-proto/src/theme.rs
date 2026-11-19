//! Themes for Silt's UI as the web UI reads them (ADR 0098), built alike by
//! the daemon, from the bundled themes and the person's, and by the server,
//! from the bundled ones.

use serde::{Deserialize, Serialize};
use silt_theme::check::Kind;
use silt_theme::{Appearance, ColorToken, Family, SyntaxTag, TextSettings, Theme, ThemeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ThemeSource {
    Bundled,
    Yours,
    /// Yours, with the id of a bundled theme it replaces.
    ReplacesBundled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AppearanceMode {
    /// The light or dark theme, as the system prefers.
    #[default]
    System,
    Light,
    Dark,
}

/// Which theme shows: the light one, the dark one, or as the system says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AppearanceSettings {
    #[serde(default)]
    pub mode: AppearanceMode,
    #[serde(default = "default_light")]
    pub light: String,
    #[serde(default = "default_dark")]
    pub dark: String,
    /// Sizes, fonts and spacing over the theme (ADR 0107).
    #[serde(default, deserialize_with = "silt_theme::text::lenient_settings")]
    pub text: TextSettings,
}

fn default_light() -> String {
    silt_theme::DEFAULT_LIGHT.to_owned()
}

fn default_dark() -> String {
    silt_theme::DEFAULT_DARK.to_owned()
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self { mode: AppearanceMode::System, light: default_light(), dark: default_dark(), text: TextSettings::default() }
    }
}

/// One theme in Settings' list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThemeSummary {
    pub id: String,
    pub name: String,
    pub family: String,
    /// `light` or `dark`.
    pub appearance: String,
    pub source: ThemeSource,
    /// bg, surface-1, fg-1, accent, ok and danger.
    pub swatch: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// The file it's in, for the person's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// What was wrong in its file, which the rest of it loaded despite.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Text pairs below 4.5:1.
    pub failing: usize,
}

/// A file of the person's that couldn't be read at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThemeProblem {
    pub file: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThemeCatalog {
    pub themes: Vec<ThemeSummary>,
    pub problems: Vec<ThemeProblem>,
    /// Where the person's themes go; none on the hosted app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    pub appearance: AppearanceSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThemeColorView {
    pub token: String,
    pub value: String,
    pub derived: bool,
    pub about: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThemeSyntaxView {
    pub tag: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub font_style: Option<String>,
    #[serde(default)]
    pub font_weight: Option<u16>,
    pub about: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThemeCheckView {
    pub pair: String,
    pub fore: String,
    pub back: String,
    pub ratio: f64,
    pub needs: f64,
    pub passes: bool,
    /// An icon or dot rather than text: advice, not a failure.
    pub mark: bool,
    pub derived: bool,
}

/// A theme's page: every token as shown, its checks, fonts and radii.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThemeDetail {
    pub summary: ThemeSummary,
    pub colors: Vec<ThemeColorView>,
    pub syntax: Vec<ThemeSyntaxView>,
    pub checks: Vec<ThemeCheckView>,
    pub ui_font: String,
    pub mono_font: String,
    /// sm, md and lg, in pixels.
    pub radii: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
}

/// The CSS a page gets after `nous.css`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThemeCss {
    pub css: String,
}

/// What importing a theme or starting one from another made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThemeSaved {
    /// The file written, in the person's themes folder.
    pub file: String,
    /// The ids of its themes.
    pub themes: Vec<String>,
    /// What Silt left out, guessed or derived, for an import.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// A theme with where it's from.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeEntry {
    pub theme: Theme,
    pub family: String,
    pub author: Option<String>,
    pub license: Option<String>,
    pub source_url: Option<String>,
    pub copyright: Option<String>,
    pub source: ThemeSource,
    pub file: Option<String>,
    pub warnings: Vec<String>,
}

impl ThemeEntry {
    /// A family's themes, from `source`.
    pub fn of(family: &Family, source: ThemeSource, file: Option<String>, warnings: &[String]) -> Vec<Self> {
        family
            .themes
            .iter()
            .map(|theme| Self {
                theme: theme.clone(),
                family: family.name.clone(),
                author: family.author.clone(),
                license: family.license.clone(),
                source_url: family.source.clone(),
                copyright: family.copyright.clone(),
                source,
                file: file.clone(),
                warnings: warnings.to_vec(),
            })
            .collect()
    }

    pub fn summary(&self) -> ThemeSummary {
        let palette = silt_theme::derive(&self.theme);
        let failing =
            silt_theme::check(&palette).iter().filter(|finding| finding.kind == Kind::Text && !finding.passes()).count();
        let swatch =
            [ColorToken::Bg, ColorToken::Surface1, ColorToken::Fg1, ColorToken::Accent, ColorToken::Ok, ColorToken::Danger]
                .iter()
                .map(|token| palette.get(*token).to_string())
                .collect();
        ThemeSummary {
            id: self.theme.id.to_string(),
            name: self.theme.name.clone(),
            family: self.family.clone(),
            appearance: self.theme.appearance.to_string(),
            source: self.source,
            swatch,
            author: self.author.clone(),
            license: self.license.clone(),
            file: self.file.clone(),
            warnings: self.warnings.clone(),
            failing,
        }
    }

    pub fn detail(&self) -> ThemeDetail {
        let palette = silt_theme::derive(&self.theme);
        let colors = ColorToken::ALL
            .iter()
            .map(|token| ThemeColorView {
                token: token.to_string(),
                value: palette.get(*token).to_string(),
                derived: palette.derived.contains(token),
                about: token.about().to_owned(),
            })
            .collect();
        let syntax = SyntaxTag::ALL
            .iter()
            .map(|tag| {
                let style = palette.syntax[tag];
                ThemeSyntaxView {
                    tag: tag.to_string(),
                    color: style.color.map(|color| color.to_string()),
                    font_style: style.font_style.map(|style| style.as_str().to_owned()),
                    font_weight: style.font_weight,
                    about: tag.about().to_owned(),
                }
            })
            .collect();
        let checks = silt_theme::check(&palette)
            .into_iter()
            .map(|finding| ThemeCheckView {
                passes: finding.passes(),
                pair: finding.pair,
                fore: finding.fore.to_string(),
                back: finding.back.to_string(),
                ratio: (finding.ratio * 100.0).floor() / 100.0,
                needs: finding.needs,
                mark: finding.kind == Kind::Mark,
                derived: finding.derived,
            })
            .collect();
        let names = |names: &Option<Vec<silt_theme::FontName>>, default: &str| {
            names.as_ref().map_or_else(
                || default.to_owned(),
                |names| names.iter().map(silt_theme::FontName::as_str).collect::<Vec<_>>().join(", "),
            )
        };
        let radii = &self.theme.radii;
        ThemeDetail {
            summary: self.summary(),
            colors,
            syntax,
            checks,
            ui_font: names(&self.theme.fonts.ui, "IBM Plex Sans"),
            mono_font: names(&self.theme.fonts.mono, "IBM Plex Mono"),
            radii: vec![radii.sm.unwrap_or(2), radii.md.unwrap_or(4), radii.lg.unwrap_or(8)],
            source_url: self.source_url.clone(),
            copyright: self.copyright.clone(),
        }
    }
}

/// A synced copy's font, its bytes when they are the font its name says.
fn copy_font(copy: &crate::ThemeCopy, file: &silt_theme::font::FontFile) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(copy.fonts.get(file.name())?).ok()?;
    file.check(&bytes).ok()?;
    Some(bytes)
}

/// The person's themes as a server keeps their copies (ADR 0099), each
/// face kept only when its copy carries it as a font.
pub fn copy_entries(copies: &[crate::ThemeCopy]) -> Vec<ThemeEntry> {
    let mut entries = Vec::new();
    for copy in copies.iter().filter(|copy| !copy.deleted && copy.valid()) {
        let Ok((mut family, warnings)) = silt_theme::parse(&copy.json) else { continue };
        let mut warnings: Vec<String> = warnings.iter().map(ToString::to_string).collect();
        for theme in &mut family.themes {
            theme.fonts.faces.retain(|face| {
                let kept = copy_font(copy, &face.file).is_some();
                if !kept {
                    warnings.push(format!("{}: {} isn't among the fonts it ships; left out", theme.id, face.file));
                }
                kept
            });
        }
        let label = if copy.fonts.is_empty() { format!("{}.json", copy.name) } else { format!("{}/theme.json", copy.name) };
        entries.extend(ThemeEntry::of(&family, ThemeSource::Yours, Some(label), &warnings));
    }
    entries
}

/// A face one of the person's synced themes ships, with its media type.
pub fn synced_font(copies: &[crate::ThemeCopy], id: &str, file: &str) -> Option<(Vec<u8>, &'static str)> {
    let file = silt_theme::font::FontFile::parse(file)?;
    copies.iter().filter(|copy| !copy.deleted && copy.valid()).find_map(|copy| {
        let (family, _) = silt_theme::parse(&copy.json).ok()?;
        let theme = family.themes.iter().find(|theme| theme.id.as_str() == id)?;
        theme.fonts.faces.iter().find(|face| face.file == file)?;
        copy_font(copy, &file).map(|bytes| (bytes, file.format().media_type()))
    })
}

/// Silt's own themes.
pub fn bundled_entries() -> Vec<ThemeEntry> {
    silt_theme::bundled().iter().flat_map(|family| ThemeEntry::of(family, ThemeSource::Bundled, None, &[])).collect()
}

/// The bundled themes with the person's laid over them: one with a bundled
/// theme's id replaces it, in its place.
pub fn merge(bundled: Vec<ThemeEntry>, yours: Vec<ThemeEntry>) -> Vec<ThemeEntry> {
    let mut all = bundled;
    for mut entry in yours {
        match all.iter_mut().find(|existing| existing.theme.id == entry.theme.id) {
            Some(existing) if existing.source == ThemeSource::Bundled => {
                entry.source = ThemeSource::ReplacesBundled;
                *existing = entry;
            }
            Some(_) => {}
            None => all.push(entry),
        }
    }
    all
}

/// The theme for a slot: the one picked, if it's there and of the slot's
/// appearance, else Nous.
pub fn pick<'a>(entries: &'a [ThemeEntry], id: &str, appearance: Appearance) -> &'a Theme {
    let fallback = if appearance == Appearance::Light { silt_theme::DEFAULT_LIGHT } else { silt_theme::DEFAULT_DARK };
    let find = |id: &str| entries.iter().find(|entry| entry.theme.id.as_str() == id && entry.theme.appearance == appearance);
    find(id).or_else(|| find(fallback)).map(|entry| &entry.theme).unwrap_or_else(|| nous(appearance))
}

fn nous(appearance: Appearance) -> &'static Theme {
    use std::sync::OnceLock;
    static NOUS: OnceLock<Vec<Theme>> = OnceLock::new();
    let themes =
        NOUS.get_or_init(|| silt_theme::parse(silt_theme::BUNDLED[0].1).map(|(family, _)| family.themes).unwrap_or_default());
    themes.iter().find(|theme| theme.appearance == appearance).expect("Nous has a light and a dark theme")
}

/// The CSS for the light and dark themes `settings` pick.
/// The picked themes' CSS, then the text and layout over them (ADR 0107).
pub fn css(entries: &[ThemeEntry], settings: &AppearanceSettings) -> String {
    let themes =
        silt_theme::theme_css(pick(entries, &settings.light, Appearance::Light), pick(entries, &settings.dark, Appearance::Dark));
    themes + &silt_theme::text::css(&settings.text)
}

/// `settings`, refused when a slot names a theme of the other appearance.
pub fn checked(entries: &[ThemeEntry], settings: &AppearanceSettings) -> Result<(), String> {
    for (id, appearance) in [(&settings.light, Appearance::Light), (&settings.dark, Appearance::Dark)] {
        let id: ThemeId = id.parse().map_err(|error: silt_theme::model::BadThemeId| error.to_string())?;
        if let Some(entry) = entries.iter().find(|entry| entry.theme.id == id)
            && entry.theme.appearance != appearance
        {
            return Err(format!("{id} is a {} theme, so it can't be the {appearance} one", entry.theme.appearance));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yours(json: &str) -> Vec<ThemeEntry> {
        let (family, _) = silt_theme::parse(json).expect("parses");
        ThemeEntry::of(&family, ThemeSource::Yours, Some("mine.json".into()), &[])
    }

    const MINE: &str = r##"{ "name": "Mine", "themes": [
        { "id": "nous-dark", "name": "My Nous Dark", "appearance": "dark", "colors": { "bg": "#000000", "fg-1": "#ffffff", "accent": "#ff8800" } },
        { "id": "mine-light", "name": "Mine", "appearance": "light", "colors": { "bg": "#ffffff", "fg-1": "#111111", "accent": "#0055cc" } }
    ] }"##;

    #[test]
    fn yours_replace_bundled_themes_by_id_and_join_the_rest() {
        let all = merge(bundled_entries(), yours(MINE));
        let dark = all.iter().find(|entry| entry.theme.id.as_str() == "nous-dark").expect("nous-dark");
        assert_eq!(dark.source, ThemeSource::ReplacesBundled);
        assert_eq!(dark.theme.name, "My Nous Dark");
        assert_eq!(all.iter().filter(|entry| entry.theme.id.as_str() == "nous-dark").count(), 1);
        assert!(all.iter().any(|entry| entry.theme.id.as_str() == "mine-light" && entry.source == ThemeSource::Yours));
    }

    #[test]
    fn a_missing_or_mismatched_pick_falls_back_to_nous() {
        let all = merge(bundled_entries(), yours(MINE));
        assert_eq!(pick(&all, "mine-light", Appearance::Light).id.as_str(), "mine-light");
        assert_eq!(pick(&all, "gone", Appearance::Light).id.as_str(), "nous-light");
        assert_eq!(pick(&all, "mine-light", Appearance::Dark).id.as_str(), "nous-dark");
        let settings = AppearanceSettings { light: "mine-light".into(), ..AppearanceSettings::default() };
        assert!(css(&all, &settings).starts_with(":root:not([data-theme=dark]){--bg:#ffffff;"));
    }

    #[test]
    fn a_theme_of_the_other_appearance_is_refused_for_a_slot() {
        let all = merge(bundled_entries(), yours(MINE));
        let wrong = AppearanceSettings { dark: "mine-light".into(), ..AppearanceSettings::default() };
        assert!(checked(&all, &wrong).is_err());
        assert!(checked(&all, &AppearanceSettings::default()).is_ok());
        assert!(checked(&all, &AppearanceSettings { light: "Not an id".into(), ..AppearanceSettings::default() }).is_err());
    }

    #[test]
    fn synced_copies_are_themes_and_only_their_shipped_fonts_are_served() {
        use base64::Engine as _;
        let json = r##"{ "name": "Harbour", "themes": [{ "id": "harbour", "name": "Harbour", "appearance": "light",
            "colors": { "bg": "#ffffff", "fg-1": "#111111", "accent": "#0055cc" },
            "fonts": { "ui": ["Harbour Sans"], "faces": [
              { "family": "Harbour Sans", "file": "fonts/sans.woff2" },
              { "family": "Harbour Sans", "file": "fonts/missing.woff2" }
            ] } }] }"##;
        let mut fonts = std::collections::BTreeMap::new();
        fonts.insert("sans.woff2".to_owned(), base64::engine::general_purpose::STANDARD.encode(b"wOF2 a font"));
        let copies = vec![
            crate::ThemeCopy { name: "harbour".into(), at_ms: 1, deleted: false, json: json.into(), fonts },
            crate::ThemeCopy { name: "gone".into(), at_ms: 2, deleted: true, json: String::new(), fonts: Default::default() },
        ];
        let entries = copy_entries(&copies);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file.as_deref(), Some("harbour/theme.json"));
        assert_eq!(entries[0].theme.fonts.faces.len(), 1);
        assert!(entries[0].warnings.iter().any(|warning| warning.contains("missing.woff2")));
        let all = merge(bundled_entries(), entries);
        assert!(all.iter().any(|entry| entry.theme.id.as_str() == "catppuccin-mocha"));
        assert_eq!(synced_font(&copies, "harbour", "sans.woff2"), Some((b"wOF2 a font".to_vec(), "font/woff2")));
        assert_eq!(synced_font(&copies, "harbour", "missing.woff2"), None);
        assert_eq!(synced_font(&copies, "harbour", "../sans.woff2"), None);
        let settings = AppearanceSettings { light: "harbour".into(), ..AppearanceSettings::default() };
        assert!(css(&all, &settings).contains("url(theme-fonts/harbour/sans.woff2)"));
    }

    #[test]
    fn a_summary_shows_its_swatch_and_failures() {
        let summary = bundled_entries()[0].summary();
        assert_eq!(summary.swatch.len(), 6);
        assert_eq!(summary.failing, 0);
        let detail = bundled_entries()[0].detail();
        assert_eq!(detail.colors.len(), ColorToken::ALL.len());
        assert_eq!(detail.radii, [2, 4, 8]);
    }
}
