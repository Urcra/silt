//! Themes for Silt's UI on this machine (ADR 0098): Silt's own with the
//! person's from `themes/` laid over them, the light and dark theme picked in
//! `appearance.json`, and the CSS a page gets.

use std::fs;
use std::path::{Path, PathBuf};

use silt_proto::theme::{self as views, ThemeEntry};
use silt_proto::view::{AppearanceMode, AppearanceSettings, ThemeCatalog, ThemeDetail, ThemeProblem, ThemeSaved, ThemeSource};
use silt_theme::{Family, ThemeId};

use crate::core::CoreError;
use crate::design::{read_capped, write_atomically};

/// The person's themes, one family a file.
pub const THEMES_DIR: &str = "themes";
const APPEARANCE: &str = "appearance.json";
/// A theme file's most, and how many files are read.
const FILE_BYTES: u64 = 256 << 10;
const MOST_FILES: usize = 256;

pub fn themes_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(THEMES_DIR)
}

/// The person's theme files, in name order.
pub fn files(data_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(themes_dir(data_dir)) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json") && path.is_file())
        .collect();
    files.sort();
    files.truncate(MOST_FILES);
    files
}

/// Every theme, the person's over Silt's, and the files that couldn't be read.
pub fn entries(data_dir: &Path) -> (Vec<ThemeEntry>, Vec<ThemeProblem>) {
    let mut yours = Vec::new();
    let mut problems = Vec::new();
    for path in files(data_dir) {
        let file = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(json) = read_capped(&path, FILE_BYTES) else {
            problems.push(ThemeProblem { file, message: format!("unreadable, or over {} KiB", FILE_BYTES >> 10) });
            continue;
        };
        match silt_theme::parse(&json) {
            Ok((family, warnings)) => {
                let warnings: Vec<String> = warnings.iter().map(ToString::to_string).collect();
                yours.extend(ThemeEntry::of(&family, ThemeSource::Yours, Some(file), &warnings));
            }
            Err(error) => problems.push(ThemeProblem { file, message: error.to_string() }),
        }
    }
    (views::merge(views::bundled_entries(), yours), problems)
}

pub fn appearance(data_dir: &Path) -> AppearanceSettings {
    read_capped(&data_dir.join(APPEARANCE), 16 << 10).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

pub fn set_appearance(data_dir: &Path, settings: &AppearanceSettings) -> Result<(), CoreError> {
    let (entries, _) = entries(data_dir);
    views::checked(&entries, settings).map_err(CoreError::Invalid)?;
    let json = serde_json::to_vec_pretty(settings).map_err(|error| CoreError::Invalid(error.to_string()))?;
    write_atomically(&data_dir.join(APPEARANCE), &json)
}

pub fn catalog(data_dir: &Path) -> ThemeCatalog {
    let (entries, problems) = entries(data_dir);
    ThemeCatalog {
        themes: entries.iter().map(ThemeEntry::summary).collect(),
        problems,
        folder: Some(themes_dir(data_dir).display().to_string()),
        appearance: appearance(data_dir),
    }
}

pub fn detail(data_dir: &Path, id: &str) -> Option<ThemeDetail> {
    entries(data_dir).0.iter().find(|entry| entry.theme.id.as_str() == id).map(ThemeEntry::detail)
}

/// The CSS for what's picked, or for `light` and `dark` in its place, as a
/// preview asks.
pub fn css(data_dir: &Path, light: Option<String>, dark: Option<String>) -> String {
    let (entries, _) = entries(data_dir);
    let mut settings = appearance(data_dir);
    settings.light = light.unwrap_or(settings.light);
    settings.dark = dark.unwrap_or(settings.dark);
    views::css(&entries, &settings)
}

/// Writes `family` as `<stem>.json` in the themes folder; with `replace`
/// false, a file that's there is kept and the next free `<stem>-2.json` and
/// so on is used.
pub fn save(data_dir: &Path, stem: &str, family: &Family, replace: bool) -> Result<PathBuf, CoreError> {
    let dir = themes_dir(data_dir);
    fs::create_dir_all(&dir)?;
    let mut path = dir.join(format!("{stem}.json"));
    let mut next = 2;
    while !replace && path.exists() {
        path = dir.join(format!("{stem}-{next}.json"));
        next += 1;
    }
    write_atomically(&path, family.to_json().as_bytes())?;
    Ok(path)
}

fn saved(path: &Path, family: &Family, notes: Vec<String>) -> ThemeSaved {
    ThemeSaved {
        file: path.display().to_string(),
        themes: family.themes.iter().map(|theme| theme.id.to_string()).collect(),
        notes,
    }
}

/// A VS Code theme or a Zed family, read into Silt's format and saved.
pub fn import(data_dir: &Path, text: &str) -> Result<ThemeSaved, CoreError> {
    let imported = silt_theme::import::import(text, &silt_theme::import::Options::default())
        .map_err(|error| CoreError::Invalid(error.to_string()))?;
    let stem = ThemeId::from_name(&imported.family.name)
        .ok_or_else(|| CoreError::Invalid("the theme's name makes no file name".into()))?;
    let path = save(data_dir, stem.as_str(), &imported.family, false)?;
    let mut notes: Vec<String> = imported.notes.clone();
    notes.extend(imported.ignored.iter().map(|ignored| format!("ignored {ignored}")));
    Ok(saved(&path, &imported.family, notes))
}

/// A copy of `from` as a theme of the person's, to edit in their editor.
pub fn duplicate(data_dir: &Path, from: &str) -> Result<ThemeSaved, CoreError> {
    let (entries, _) = entries(data_dir);
    let source = entries
        .iter()
        .find(|entry| entry.theme.id.as_str() == from)
        .ok_or_else(|| CoreError::Invalid(format!("no theme {from}")))?;
    let taken = |id: &str| entries.iter().any(|entry| entry.theme.id.as_str() == id);
    let mut id = format!("{from}-mine");
    let mut next = 2;
    while taken(&id) {
        id = format!("{from}-mine-{next}");
        next += 1;
    }
    let mut theme = source.theme.clone();
    theme.id = id.parse().map_err(|error: silt_theme::model::BadThemeId| CoreError::Invalid(error.to_string()))?;
    theme.name = format!("{} (mine)", source.theme.name);
    let family = Family::new(theme.name.clone(), vec![theme]);
    let path = save(data_dir, &id, &family, false)?;
    Ok(saved(&path, &family, Vec::new()))
}

/// `index.html` with the picked theme's CSS and mode in it, so the page never
/// shows Nous first.
pub fn inject(html: &str, css: &str, mode: AppearanceMode) -> String {
    let style = format!("<style id=\"silt-theme\">{css}</style></head>");
    let mut page = html.replacen("</head>", &style, 1);
    let theme = match mode {
        AppearanceMode::System => None,
        AppearanceMode::Light => Some("light"),
        AppearanceMode::Dark => Some("dark"),
    };
    if let Some(theme) = theme {
        page = page.replacen("<html", &format!("<html data-theme=\"{theme}\""), 1);
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINE: &str = r##"{ "name": "Mine", "themes": [
        { "id": "mine", "name": "Mine", "appearance": "light", "extra": 1,
          "colors": { "bg": "#ffffff", "fg-1": "#111111", "accent": "#0055cc" } }
    ] }"##;

    #[test]
    fn the_persons_themes_join_silts_and_a_broken_file_is_a_problem() {
        let data = tempfile::tempdir().expect("temp dir");
        fs::create_dir_all(themes_dir(data.path())).expect("themes dir");
        fs::write(themes_dir(data.path()).join("mine.json"), MINE).expect("theme");
        fs::write(themes_dir(data.path()).join("broken.json"), "{ nope").expect("broken");
        fs::write(themes_dir(data.path()).join("notes.txt"), "not a theme").expect("notes");
        let catalog = catalog(data.path());
        let mine = catalog.themes.iter().find(|theme| theme.id == "mine").expect("mine listed");
        assert_eq!(mine.source, ThemeSource::Yours);
        assert_eq!(mine.file.as_deref(), Some("mine.json"));
        assert_eq!(mine.warnings.len(), 1, "{:?}", mine.warnings);
        assert_eq!(catalog.problems.len(), 1);
        assert_eq!(catalog.problems[0].file, "broken.json");
        assert!(catalog.themes.iter().any(|theme| theme.id == "nous-dark"));
    }

    #[test]
    fn the_pick_is_kept_checked_and_falls_back() {
        let data = tempfile::tempdir().expect("temp dir");
        assert_eq!(appearance(data.path()), AppearanceSettings::default());
        fs::create_dir_all(themes_dir(data.path())).expect("themes dir");
        fs::write(themes_dir(data.path()).join("mine.json"), MINE).expect("theme");
        let picked = AppearanceSettings { mode: AppearanceMode::Dark, light: "mine".into(), dark: "nous-dark".into() };
        set_appearance(data.path(), &picked).expect("saved");
        assert_eq!(appearance(data.path()), picked);
        assert!(css(data.path(), None, None).starts_with(":root:not([data-theme=dark]){--bg:#ffffff;"));
        let wrong = AppearanceSettings { dark: "mine".into(), ..picked.clone() };
        assert!(set_appearance(data.path(), &wrong).is_err());
        fs::remove_file(themes_dir(data.path()).join("mine.json")).expect("removed");
        assert!(css(data.path(), None, None).starts_with(":root:not([data-theme=dark]){--bg:#f6f1e8;"), "Nous stands in");
    }

    #[test]
    fn an_import_and_a_copy_are_saved_without_replacing_a_file() {
        let data = tempfile::tempdir().expect("temp dir");
        let night = include_str!("../../silt-theme/tests/fixtures/vscode-night.jsonc");
        let first = import(data.path(), night).expect("imported");
        let second = import(data.path(), night).expect("imported again");
        assert!(first.file.ends_with("fixture-night.json") && second.file.ends_with("fixture-night-2.json"));
        assert!(first.notes.iter().any(|note| note.contains("semanticTokenColors")));
        let copy = duplicate(data.path(), "nous-dark").expect("copied");
        assert_eq!(copy.themes, ["nous-dark-mine"]);
        assert_eq!(duplicate(data.path(), "nous-dark").expect("copied again").themes, ["nous-dark-mine-2"]);
        assert!(catalog(data.path()).themes.iter().any(|theme| theme.id == "nous-dark-mine"));
        assert!(duplicate(data.path(), "nope").is_err());
    }

    #[test]
    fn the_page_gets_the_style_and_the_mode() {
        let html = "<!doctype html><html lang=\"en\"><head><title>Silt</title></head><body></body></html>";
        let page = inject(html, ":root{--bg:#000000;}", AppearanceMode::Dark);
        assert!(page.contains("<html data-theme=\"dark\" lang=\"en\">"), "{page}");
        assert!(page.contains("<style id=\"silt-theme\">:root{--bg:#000000;}</style></head>"), "{page}");
        assert!(!inject(html, "", AppearanceMode::System).contains("data-theme"));
    }
}
