//! The window's zoom (ADR 0102): the page sends the level text and layout
//! set as a `silt://zoom/` link the window catches, and the app keeps it for
//! its next start, so the window opens at it before the page has loaded.

use std::fs;
use std::path::Path;

use tauri::Url;

/// The file in the app's data dir that keeps the last level.
pub const LAST_ZOOM: &str = "zoom";

/// A zoom level in percent, as silt-theme's `Zoom` allows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level(u16);

impl Level {
    const MIN: u16 = 50;
    const MAX: u16 = 200;

    pub fn parse(text: &str) -> Option<Self> {
        let percent: u16 = text.trim().parse().ok()?;
        (Self::MIN..=Self::MAX).contains(&percent).then_some(Self(percent))
    }

    /// As the webview takes it, 1.0 for 100%.
    pub fn factor(self) -> f64 {
        f64::from(self.0) / 100.0
    }
}

/// The level a `silt://zoom/?percent=120` link names.
pub fn of_link(link: &Url) -> Option<Level> {
    if link.scheme() != "silt" || link.host_str() != Some("zoom") || !matches!(link.path(), "" | "/") {
        return None;
    }
    let mut level = None;
    for (key, value) in link.query_pairs() {
        match key.as_ref() {
            "percent" => level = Some(Level::parse(&value)?),
            _ => return None,
        }
    }
    level
}

pub fn keep(file: &Path, level: Level) {
    let _ = fs::write(file, level.0.to_string());
}

pub fn last(file: &Path) -> Option<Level> {
    Level::parse(&fs::read_to_string(file).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(text: &str) -> Option<Level> {
        of_link(&Url::parse(text).expect("a link"))
    }

    #[test]
    fn a_zoom_link_names_a_level_in_range() {
        assert_eq!(link("silt://zoom/?percent=120"), Some(Level(120)));
        assert_eq!(link("silt://zoom?percent=50").map(Level::factor), Some(0.5));
        for refused in [
            "silt://zoom/?percent=20",
            "silt://zoom/?percent=900",
            "silt://zoom/?percent=big",
            "silt://zoom/?percent=120&css=x",
            "silt://zoom/x?percent=120",
            "silt://theme/?percent=120",
            "https://zoom/?percent=120",
            "silt://zoom/",
        ] {
            assert_eq!(link(refused), None, "{refused}");
        }
    }

    #[test]
    fn the_level_is_kept_for_the_next_start() {
        let dir = std::env::temp_dir().join(format!("silt-desktop-zoom-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("dir");
        let file = dir.join(LAST_ZOOM);
        assert_eq!(last(&file), None);
        keep(&file, Level(130));
        assert_eq!(last(&file), Some(Level(130)));
        fs::write(&file, "nonsense").expect("written");
        assert_eq!(last(&file), None);
        let _ = fs::remove_dir_all(dir);
    }
}
