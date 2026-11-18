//! The window follows the theme the page shows (ADR 0099): the page sends
//! it as a `silt://theme/` link the window catches, and the app keeps it for
//! its next start, so the window and its starting page open in it.

use std::fs;
use std::path::Path;

use tauri::Url;

/// What the window shows of the page's theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowTheme {
    pub dark: bool,
    pub bg: [u8; 3],
    pub fg: [u8; 3],
}

/// The file in the app's data dir that keeps the theme last shown.
pub const LAST_THEME: &str = "theme";

fn hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn to_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The theme a `silt://theme/?appearance=dark&bg=%23…&fg=%23…` link names.
pub fn of_link(link: &Url) -> Option<WindowTheme> {
    if link.scheme() != "silt" || link.host_str() != Some("theme") || !matches!(link.path(), "" | "/") {
        return None;
    }
    let (mut dark, mut bg, mut fg) = (None, None, None);
    for (key, value) in link.query_pairs() {
        match key.as_ref() {
            "appearance" => dark = matches!(value.as_ref(), "light" | "dark").then(|| value == "dark"),
            "bg" => bg = hex(&value),
            "fg" => fg = hex(&value),
            _ => return None,
        }
    }
    Some(WindowTheme { dark: dark?, bg: bg?, fg: fg? })
}

impl WindowTheme {
    fn line(&self) -> String {
        format!("{} {} {}", if self.dark { "dark" } else { "light" }, to_hex(self.bg), to_hex(self.fg))
    }

    fn of_line(line: &str) -> Option<Self> {
        let mut parts = line.split_whitespace();
        let dark = match parts.next()? {
            "dark" => true,
            "light" => false,
            _ => return None,
        };
        let theme = Self { dark, bg: hex(parts.next()?)?, fg: hex(parts.next()?)? };
        parts.next().is_none().then_some(theme)
    }

    /// The starting page's colours, before the daemon's page loads.
    pub fn script(theme: Option<&Self>) -> String {
        let value = theme.map_or(serde_json::Value::Null, |theme| {
            serde_json::json!({ "appearance": if theme.dark { "dark" } else { "light" }, "bg": to_hex(theme.bg), "fg": to_hex(theme.fg) })
        });
        format!("window.__siltTheme = {value};")
    }
}

pub fn keep(file: &Path, theme: &WindowTheme) {
    let _ = fs::write(file, theme.line());
}

pub fn last(file: &Path) -> Option<WindowTheme> {
    WindowTheme::of_line(fs::read_to_string(file).ok()?.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(text: &str) -> Option<WindowTheme> {
        of_link(&Url::parse(text).expect("a link"))
    }

    #[test]
    fn a_theme_link_names_the_appearance_and_two_colours() {
        let theme = link("silt://theme/?appearance=dark&bg=%231e1e2e&fg=%23CDD6F4").expect("a theme");
        assert_eq!(theme, WindowTheme { dark: true, bg: [0x1e, 0x1e, 0x2e], fg: [0xcd, 0xd6, 0xf4] });
        assert_eq!(link("silt://theme?appearance=light&bg=%23ffffff&fg=%23000000").map(|theme| theme.dark), Some(false));
    }

    #[test]
    fn anything_else_is_no_theme() {
        for refused in [
            "silt://open/?backend=native",
            "https://theme/?appearance=dark&bg=%23000000&fg=%23ffffff",
            "silt://theme/x?appearance=dark&bg=%23000000&fg=%23ffffff",
            "silt://theme/?appearance=dusk&bg=%23000000&fg=%23ffffff",
            "silt://theme/?appearance=dark&bg=red&fg=%23ffffff",
            "silt://theme/?appearance=dark&bg=%23000000",
            "silt://theme/?appearance=dark&bg=%23000000&fg=%23ffffff&css=x",
        ] {
            assert_eq!(link(refused), None, "{refused}");
        }
    }

    #[test]
    fn the_theme_is_kept_for_the_next_start() {
        let dir = std::env::temp_dir().join(format!("silt-desktop-theme-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("dir");
        let file = dir.join(LAST_THEME);
        assert_eq!(last(&file), None);
        let theme = WindowTheme { dark: true, bg: [26, 23, 20], fg: [239, 230, 214] };
        keep(&file, &theme);
        assert_eq!(last(&file), Some(theme));
        fs::write(&file, "dark #1a1714 nope").expect("write");
        assert_eq!(last(&file), None);
        let _ = fs::remove_dir_all(dir);
        assert!(WindowTheme::script(Some(&theme)).contains(r##""bg":"#1a1714""##));
        assert_eq!(WindowTheme::script(None), "window.__siltTheme = null;");
    }
}
