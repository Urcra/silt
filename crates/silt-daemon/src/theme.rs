//! Themes for Silt's UI on this machine (ADR 0098): Silt's own with the
//! person's from `themes/` laid over them, the light and dark theme picked in
//! `appearance.json`, and the CSS a page gets.

use std::fs;
use std::path::{Path, PathBuf};

use silt_proto::theme::{self as views, ThemeEntry};
use silt_proto::view::{AppearanceMode, AppearanceSettings, ThemeCatalog, ThemeDetail, ThemeProblem, ThemeSaved, ThemeSource};
use silt_theme::font::{FACE_BYTES, FontFile, THEME_FONT_BYTES};
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

/// A theme that ships fonts is a folder holding this and `fonts/` (ADR 0099).
pub const FOLDER_FILE: &str = "theme.json";

/// One of the person's theme files: `<name>.json`, or a folder
/// `<name>/theme.json` with its `fonts/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub json: PathBuf,
    /// The folder, for a theme that ships fonts.
    pub root: Option<PathBuf>,
}

impl Source {
    /// How Settings names it.
    pub fn label(&self) -> String {
        if self.root.is_some() { format!("{}/{FOLDER_FILE}", self.name) } else { format!("{}.json", self.name) }
    }

    /// A file or folder given by hand, as `silt theme check` takes it.
    pub fn at(path: &Path) -> Self {
        let folder = if path.is_dir() {
            Some(path.to_owned())
        } else if path.file_name().is_some_and(|name| name == FOLDER_FILE) {
            path.parent().map(Path::to_owned)
        } else {
            None
        };
        let name = |path: &Path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        match folder {
            Some(root) => Self { name: name(&root), json: root.join(FOLDER_FILE), root: Some(root) },
            None => Self { name: name(path), json: path.to_owned(), root: None },
        }
    }
}

/// The person's themes, in name order; a folder hides a file of its name.
pub fn sources(data_dir: &Path) -> Vec<Source> {
    let Ok(entries) = fs::read_dir(themes_dir(data_dir)) else { return Vec::new() };
    let mut found = std::collections::BTreeMap::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        let Some(name) = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else { continue };
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() && path.join(FOLDER_FILE).is_file() {
            found.insert(name.clone(), Source { name, json: path.join(FOLDER_FILE), root: Some(path) });
        } else if path.extension().is_some_and(|extension| extension == "json") && path.is_file() {
            found.entry(name.clone()).or_insert(Source { name, json: path, root: None });
        }
    }
    found.into_values().take(MOST_FILES).collect()
}

/// A face's file under `root/fonts/`, never outside it, however it's named.
fn font_path(root: &Path, file: &FontFile) -> Option<PathBuf> {
    let fonts = root.join("fonts").canonicalize().ok()?;
    let path = fonts.join(file.name()).canonicalize().ok()?;
    (path.starts_with(&fonts) && path.is_file()).then_some(path)
}

/// Its first bytes, enough to tell a font's format.
fn head(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let mut head = vec![0; 4];
    let read = fs::File::open(path).ok()?.read(&mut head).ok()?;
    head.truncate(read);
    Some(head)
}

/// Leaves out the faces whose files are missing, aren't fonts or are too
/// large, saying why; a theme that isn't a folder ships none.
fn check_faces(family: &mut Family, root: Option<&Path>, warnings: &mut Vec<String>) {
    for theme in &mut family.themes {
        let mut total = 0;
        theme.fonts.faces.retain(|face| {
            let Some(root) = root else {
                warnings.push(format!(
                    "{}: a theme ships fonts as a folder, <name>/theme.json beside fonts/; {} left out",
                    theme.id, face.file
                ));
                return false;
            };
            let Some(path) = font_path(root, &face.file) else {
                warnings.push(format!("{}: {} isn't in the theme's fonts/; left out", theme.id, face.file));
                return false;
            };
            let size = fs::metadata(&path).map_or(u64::MAX, |meta| meta.len());
            let checked = if size > FACE_BYTES {
                Err(format!("{} is over {} KiB", face.file, FACE_BYTES >> 10))
            } else if total + size > THEME_FONT_BYTES {
                Err(format!("{} would take the theme's fonts over {} MiB", face.file, THEME_FONT_BYTES >> 20))
            } else {
                head(&path)
                    .ok_or_else(|| format!("{} can't be read", face.file))
                    .and_then(|head| face.file.check(&head).map(|_| ()))
            };
            match checked {
                Ok(()) => {
                    total += size;
                    true
                }
                Err(why) => {
                    warnings.push(format!("{}: {why}; left out", theme.id));
                    false
                }
            }
        });
    }
}

/// A theme file read, with its faces checked against its folder.
pub fn read_source(source: &Source) -> Result<(Family, Vec<String>), String> {
    let json = read_capped(&source.json, FILE_BYTES).ok_or_else(|| format!("unreadable, or over {} KiB", FILE_BYTES >> 10))?;
    let (mut family, warnings) = silt_theme::parse(&json).map_err(|error| error.to_string())?;
    let mut warnings: Vec<String> = warnings.iter().map(ToString::to_string).collect();
    check_faces(&mut family, source.root.as_deref(), &mut warnings);
    Ok((family, warnings))
}

/// Every theme, the person's over Silt's, and the files that couldn't be read.
pub fn entries(data_dir: &Path) -> (Vec<ThemeEntry>, Vec<ThemeProblem>) {
    let mut yours = Vec::new();
    let mut problems = Vec::new();
    for source in sources(data_dir) {
        match read_source(&source) {
            Ok((family, warnings)) => yours.extend(ThemeEntry::of(&family, ThemeSource::Yours, Some(source.label()), &warnings)),
            Err(message) => problems.push(ThemeProblem { file: source.label(), message }),
        }
    }
    (views::merge(views::bundled_entries(), yours), problems)
}

/// A face a current theme of the person's ships, read and checked whole,
/// with its media type; for `/theme-fonts/<id>/<file>`.
pub fn font(data_dir: &Path, id: &str, file: &str) -> Option<(Vec<u8>, &'static str)> {
    let file = FontFile::parse(file)?;
    let (entries, _) = entries(data_dir);
    let entry = entries.iter().find(|entry| entry.theme.id.as_str() == id && entry.source != ThemeSource::Bundled)?;
    entry.theme.fonts.faces.iter().find(|face| face.file == file)?;
    let source = sources(data_dir).into_iter().find(|source| source.label() == entry.file.as_deref().unwrap_or_default())?;
    let path = font_path(source.root.as_deref()?, &file)?;
    let bytes = fs::read(path).ok()?;
    let format = file.check(&bytes).ok()?;
    Some((bytes, format.media_type()))
}

/// The file or folder the person's theme `id` is in.
pub fn source_of(data_dir: &Path, id: &str) -> Option<Source> {
    sources(data_dir)
        .into_iter()
        .find(|source| read_source(source).is_ok_and(|(family, _)| family.themes.iter().any(|theme| theme.id.as_str() == id)))
}

/// Copies the faces `theme` ships from `from`'s fonts to `to`'s.
pub fn copy_fonts(from: &Path, to: &Path, faces: &[silt_theme::model::FontFace]) -> Result<(), CoreError> {
    fs::create_dir_all(to.join("fonts"))?;
    for face in faces {
        if let Some(path) = font_path(from, &face.file) {
            fs::copy(path, to.join("fonts").join(face.file.name()))?;
        }
    }
    Ok(())
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

/// The CSS for what's picked, or for `light`, `dark` and `text` in its place,
/// as a preview or a page changing them asks.
pub fn css(data_dir: &Path, light: Option<String>, dark: Option<String>, text: Option<silt_theme::TextSettings>) -> String {
    let (entries, _) = entries(data_dir);
    let mut settings = appearance(data_dir);
    settings.light = light.unwrap_or(settings.light);
    settings.dark = dark.unwrap_or(settings.dark);
    settings.text = text.unwrap_or(settings.text);
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
    let faces = theme.fonts.faces.clone();
    let family = Family::new(theme.name.clone(), vec![theme]);
    let from_root = sources(data_dir)
        .into_iter()
        .find(|found| Some(found.label()) == source.file)
        .and_then(|found| found.root)
        .filter(|_| !faces.is_empty());
    let path = match from_root {
        Some(from) => {
            let to = themes_dir(data_dir).join(&id);
            copy_fonts(&from, &to, &faces)?;
            let path = to.join(FOLDER_FILE);
            write_atomically(&path, family.to_json().as_bytes())?;
            path
        }
        None => save(data_dir, &id, &family, false)?,
    };
    Ok(saved(&path, &family, Vec::new()))
}

/// What `<html>` says of motion, as text and layout set it (ADR 0107).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionAttribute {
    Reduced,
    Full,
}

impl MotionAttribute {
    pub fn of(text: &silt_theme::TextSettings) -> Option<Self> {
        match text.motion_attribute()? {
            "reduced" => Some(Self::Reduced),
            _ => Some(Self::Full),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Reduced => "reduced",
            Self::Full => "full",
        }
    }
}

/// `index.html` with the picked theme's CSS, its mode and the motion asked
/// for in it, so the page never shows Nous first or at the wrong size.
pub fn inject(html: &str, css: &str, mode: AppearanceMode, motion: Option<MotionAttribute>) -> String {
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
    if let Some(motion) = motion {
        page = page.replacen("<html", &format!("<html data-motion=\"{}\"", motion.as_str()), 1);
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
        let picked = AppearanceSettings {
            mode: AppearanceMode::Dark,
            light: "mine".into(),
            dark: "nous-dark".into(),
            text: silt_theme::TextSettings::default(),
            seasonal: silt_proto::view::SeasonalSettings { show: silt_proto::view::SeasonalShow::Off, snow: false },
        };
        set_appearance(data.path(), &picked).expect("saved");
        assert_eq!(appearance(data.path()), picked);
        assert!(css(data.path(), None, None, None).starts_with(":root:not([data-theme=dark]){--bg:#ffffff;"));
        let wrong = AppearanceSettings { dark: "mine".into(), ..picked.clone() };
        assert!(set_appearance(data.path(), &wrong).is_err());
        fs::remove_file(themes_dir(data.path()).join("mine.json")).expect("removed");
        assert!(css(data.path(), None, None, None).starts_with(":root:not([data-theme=dark]){--bg:#f6f1e8;"), "Nous stands in");
    }

    #[test]
    fn text_and_layout_are_kept_beside_the_pick_and_follow_the_theme_s_css() {
        let data = tempfile::tempdir().expect("temp dir");
        let text: silt_theme::TextSettings =
            serde_json::from_str(r#"{ "chat_size": 16, "code_font": "JetBrains Mono", "motion": "full" }"#).expect("text");
        let picked = AppearanceSettings { text: text.clone(), ..AppearanceSettings::default() };
        set_appearance(data.path(), &picked).expect("saved");
        assert_eq!(appearance(data.path()).text, text);
        let page = css(data.path(), None, None, None);
        let (themes, ours) = page.split_once(":root:root:root{").expect("the text's block comes after the themes'");
        assert!(themes.contains("--font-mono:"), "{themes}");
        assert!(ours.starts_with("--chat-size:16px;--font-mono:\"JetBrains Mono\", ui-monospace"), "{ours}");
        assert_eq!(MotionAttribute::of(&appearance(data.path()).text), Some(MotionAttribute::Full));
    }

    #[test]
    fn a_text_setting_that_cant_be_read_keeps_the_theme_pick() {
        let data = tempfile::tempdir().expect("temp dir");
        let json =
            r#"{ "mode": "dark", "light": "nous-light", "dark": "fjord-dark", "text": { "ui_size": "huge", "chat_size": 15 } }"#;
        fs::write(data.path().join(APPEARANCE), json).expect("written by hand");
        let read = appearance(data.path());
        assert_eq!((read.mode, read.dark.as_str()), (AppearanceMode::Dark, "fjord-dark"));
        assert_eq!(read.text.ui_size, None);
        assert_eq!(read.text.chat_size.map(silt_theme::text::ChatSize::get), Some(15));
        fs::write(data.path().join(APPEARANCE), r#"{ "dark": "fjord-dark", "text": 7 }"#).expect("written by hand");
        assert_eq!(appearance(data.path()).dark, "fjord-dark");
    }

    #[test]
    fn the_december_decorations_default_to_auto_and_a_slip_keeps_the_pick() {
        use silt_proto::view::{SeasonalSettings, SeasonalShow};
        let data = tempfile::tempdir().expect("temp dir");
        assert_eq!(appearance(data.path()).seasonal, SeasonalSettings { show: SeasonalShow::Auto, snow: true });
        fs::write(data.path().join(APPEARANCE), r#"{ "dark": "yule-dark", "seasonal": { "show": "always", "snow": false } }"#)
            .expect("written by hand");
        assert_eq!(appearance(data.path()).seasonal, SeasonalSettings { show: SeasonalShow::Always, snow: false });
        fs::write(data.path().join(APPEARANCE), r#"{ "dark": "yule-dark", "seasonal": { "show": "sometimes" } }"#)
            .expect("written by hand");
        let read = appearance(data.path());
        assert_eq!((read.dark.as_str(), read.seasonal), ("yule-dark", SeasonalSettings::default()));
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

    const FACED: &str = r##"{ "name": "Harbour", "themes": [
        { "id": "harbour", "name": "Harbour", "appearance": "light",
          "colors": { "bg": "#ffffff", "fg-1": "#111111", "accent": "#0055cc" },
          "fonts": { "ui": ["Harbour Sans", "system-ui"], "faces": [
            { "family": "Harbour Sans", "file": "fonts/sans.woff2" },
            { "family": "Harbour Sans", "file": "fonts/fake.woff2", "weight": 700 },
            { "family": "Harbour Sans", "file": "fonts/gone.woff2", "style": "italic" }
          ] } }
    ] }"##;

    fn folder_theme(data: &Path) -> PathBuf {
        let root = themes_dir(data).join("harbour");
        fs::create_dir_all(root.join("fonts")).expect("fonts dir");
        fs::write(root.join(FOLDER_FILE), FACED).expect("theme");
        fs::write(root.join("fonts/sans.woff2"), b"wOF2 a font").expect("font");
        fs::write(root.join("fonts/fake.woff2"), b"<svg/>").expect("not a font");
        fs::write(themes_dir(data).join("secret.txt"), b"wOF2 not yours").expect("outside fonts");
        root
    }

    #[test]
    fn a_folder_theme_ships_the_faces_it_has_and_says_which_it_doesnt() {
        let data = tempfile::tempdir().expect("temp dir");
        folder_theme(data.path());
        let catalog = catalog(data.path());
        let harbour = catalog.themes.iter().find(|theme| theme.id == "harbour").expect("listed");
        assert_eq!(harbour.file.as_deref(), Some("harbour/theme.json"));
        assert_eq!(harbour.warnings.len(), 2, "{:?}", harbour.warnings);
        assert!(harbour.warnings.iter().any(|warning| warning.contains("fake.woff2 isn't a woff2 font")));
        assert!(harbour.warnings.iter().any(|warning| warning.contains("gone.woff2 isn't in the theme's fonts/")));
        let picked = AppearanceSettings { light: "harbour".into(), ..AppearanceSettings::default() };
        set_appearance(data.path(), &picked).expect("picked");
        let css = css(data.path(), None, None, None);
        assert!(css.contains("url(theme-fonts/harbour/sans.woff2)"), "{css}");
        assert!(!css.contains("fake.woff2") && !css.contains("gone.woff2"));
    }

    #[test]
    fn only_a_shipped_face_is_served() {
        let data = tempfile::tempdir().expect("temp dir");
        folder_theme(data.path());
        assert_eq!(font(data.path(), "harbour", "sans.woff2"), Some((b"wOF2 a font".to_vec(), "font/woff2")));
        for (id, file) in [
            ("harbour", "fake.woff2"),
            ("harbour", "gone.woff2"),
            ("harbour", "../secret.txt"),
            ("harbour", "..%2Fsecret.txt"),
            ("nous-light", "sans.woff2"),
            ("other", "sans.woff2"),
        ] {
            assert_eq!(font(data.path(), id, file), None, "{id}/{file}");
        }
    }

    #[test]
    fn a_theme_that_is_one_file_ships_no_fonts_and_a_copy_keeps_its_faces() {
        let data = tempfile::tempdir().expect("temp dir");
        fs::create_dir_all(themes_dir(data.path())).expect("themes dir");
        fs::write(themes_dir(data.path()).join("flat.json"), FACED.replace("\"harbour\"", "\"flat\"")).expect("theme");
        let flat = catalog(data.path()).themes.into_iter().find(|theme| theme.id == "flat").expect("listed");
        assert_eq!(flat.warnings.len(), 3);
        assert!(flat.warnings.iter().all(|warning| warning.contains("ships fonts as a folder")));

        folder_theme(data.path());
        let copy = duplicate(data.path(), "harbour").expect("copied");
        assert!(std::path::Path::new(&copy.file).ends_with("harbour-mine/theme.json"), "{}", copy.file);
        assert!(themes_dir(data.path()).join("harbour-mine/fonts/sans.woff2").is_file());
        assert!(font(data.path(), "harbour-mine", "sans.woff2").is_some());
    }

    #[test]
    fn the_page_gets_the_style_and_the_mode() {
        let html = "<!doctype html><html lang=\"en\"><head><title>Silt</title></head><body></body></html>";
        let page = inject(html, ":root{--bg:#000000;}", AppearanceMode::Dark, None);
        assert!(page.contains("<html data-theme=\"dark\" lang=\"en\">"), "{page}");
        assert!(page.contains("<style id=\"silt-theme\">:root{--bg:#000000;}</style></head>"), "{page}");
        assert!(!inject(html, "", AppearanceMode::System, None).contains("data-theme"));
        let calm = inject(html, "", AppearanceMode::System, Some(MotionAttribute::Reduced));
        assert!(calm.contains("<html data-motion=\"reduced\" lang=\"en\">"), "{calm}");
    }
}
