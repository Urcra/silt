//! The window follows the theme the page shows (ADR 0099): the page sends
//! it as a `silt://theme/` link the window catches, and the app keeps it for
//! its next start, so the window and its starting page open in it. With it
//! come the theme's tones and when the December decorations show, for the
//! starting page's tree (ADR 0133).

use std::fs;
use std::path::Path;

use tauri::Url;

/// What the window shows of the page's theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowTheme {
    pub dark: bool,
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    /// Left out by a page from before the December decorations.
    pub seasonal: Option<Seasonal>,
}

/// The December decorations as the page shows them: when, and in which of
/// the theme's tones (danger, ok, warn, info, accent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seasonal {
    pub show: Show,
    pub tones: [[u8; 3]; 5],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    Auto,
    Always,
    Off,
}

impl Show {
    fn of(word: &str) -> Option<Self> {
        match word {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Off => "off",
        }
    }
}

fn tones(text: &str) -> Option<[[u8; 3]; 5]> {
    let mut parts = text.split(',');
    let tones = [hex(parts.next()?)?, hex(parts.next()?)?, hex(parts.next()?)?, hex(parts.next()?)?, hex(parts.next()?)?];
    parts.next().is_none().then_some(tones)
}

fn tones_text(tones: &[[u8; 3]; 5]) -> String {
    tones.iter().map(|tone| to_hex(*tone)).collect::<Vec<_>>().join(",")
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
    let (mut dark, mut bg, mut fg, mut shades, mut show) = (None, None, None, None, None);
    for (key, value) in link.query_pairs() {
        match key.as_ref() {
            "appearance" => dark = matches!(value.as_ref(), "light" | "dark").then(|| value == "dark"),
            "bg" => bg = hex(&value),
            "fg" => fg = hex(&value),
            "tones" => shades = Some(tones(&value)?),
            "seasonal" => show = Some(Show::of(&value)?),
            _ => return None,
        }
    }
    let seasonal = match (show, shades) {
        (Some(show), Some(tones)) => Some(Seasonal { show, tones }),
        (None, None) => None,
        _ => return None,
    };
    Some(WindowTheme { dark: dark?, bg: bg?, fg: fg?, seasonal })
}

impl WindowTheme {
    fn line(&self) -> String {
        let mut line = format!("{} {} {}", if self.dark { "dark" } else { "light" }, to_hex(self.bg), to_hex(self.fg));
        if let Some(seasonal) = &self.seasonal {
            line += &format!(" {} {}", seasonal.show.word(), tones_text(&seasonal.tones));
        }
        line
    }

    fn of_line(line: &str) -> Option<Self> {
        let mut parts = line.split_whitespace();
        let dark = match parts.next()? {
            "dark" => true,
            "light" => false,
            _ => return None,
        };
        let (bg, fg) = (hex(parts.next()?)?, hex(parts.next()?)?);
        let seasonal = match parts.next() {
            None => None,
            Some(show) => Some(Seasonal { show: Show::of(show)?, tones: tones(parts.next()?)? }),
        };
        parts.next().is_none().then_some(Self { dark, bg, fg, seasonal })
    }

    /// The starting page's colours, before the daemon's page loads.
    pub fn script(theme: Option<&Self>) -> String {
        let value = theme.map_or(serde_json::Value::Null, |theme| {
            let mut value = serde_json::json!({ "appearance": if theme.dark { "dark" } else { "light" }, "bg": to_hex(theme.bg), "fg": to_hex(theme.fg) });
            if let Some(seasonal) = &theme.seasonal {
                value["seasonal"] = seasonal.show.word().into();
                value["tones"] = seasonal.tones.iter().map(|tone| to_hex(*tone)).collect();
            }
            value
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
        assert_eq!(theme, WindowTheme { dark: true, bg: [0x1e, 0x1e, 0x2e], fg: [0xcd, 0xd6, 0xf4], seasonal: None });
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
            "silt://theme/?appearance=dark&bg=%23000000&fg=%23ffffff&seasonal=auto",
            "silt://theme/?appearance=dark&bg=%23000000&fg=%23ffffff&seasonal=never&tones=%23000000,%23000000,%23000000,%23000000,%23000000",
            "silt://theme/?appearance=dark&bg=%23000000&fg=%23ffffff&seasonal=auto&tones=%23000000,%23000000",
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
        let theme = WindowTheme { dark: true, bg: [26, 23, 20], fg: [239, 230, 214], seasonal: None };
        keep(&file, &theme);
        assert_eq!(last(&file), Some(theme));
        fs::write(&file, "dark #1a1714 nope").expect("write");
        assert_eq!(last(&file), None);
        let _ = fs::remove_dir_all(dir);
        assert!(WindowTheme::script(Some(&theme)).contains(r##""bg":"#1a1714""##));
        assert_eq!(WindowTheme::script(None), "window.__siltTheme = null;");
    }

    #[test]
    fn the_december_decorations_come_with_the_theme_and_are_kept_with_it() {
        let theme = link(
            "silt://theme/?appearance=light&bg=%23f2f5f0&fg=%231d2a22&tones=%23a62a2f%2C%2347701b%2C%23a87a0a%2C%232b5f86%2C%231e6b48&seasonal=always",
        )
        .expect("a theme");
        let seasonal = theme.seasonal.expect("the decorations");
        assert_eq!(seasonal.show, Show::Always);
        assert_eq!(seasonal.tones[0], [0xa6, 0x2a, 0x2f]);
        assert_eq!(WindowTheme::of_line(&theme.line()), Some(theme));
        assert_eq!(WindowTheme::of_line("light #f2f5f0 #1d2a22 auto #a62a2f"), None);
        let script = WindowTheme::script(Some(&theme));
        assert!(script.contains(r#""seasonal":"always""#), "{script}");
        assert!(script.contains(r##""tones":["#a62a2f","#47701b","#a87a0a","#2b5f86","#1e6b48"]"##), "{script}");
    }
}
