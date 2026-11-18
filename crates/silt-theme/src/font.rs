//! Font files a theme ships beside it (ADR 0099): named plainly, of a font
//! format by their first bytes as well as their name, and capped.

use std::fmt;

use serde::{Serialize, Serializer};

/// The most one face's file may hold.
pub const FACE_BYTES: u64 = 1 << 20;
/// The most a theme's faces may hold together.
pub const THEME_FONT_BYTES: u64 = 4 << 20;
/// The most faces a theme may have.
pub const MOST_FACES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFormat {
    Woff2,
    Woff,
    TrueType,
    OpenType,
}

impl FontFormat {
    /// CSS's name for it, in `format()`.
    pub fn css(self) -> &'static str {
        match self {
            Self::Woff2 => "woff2",
            Self::Woff => "woff",
            Self::TrueType => "truetype",
            Self::OpenType => "opentype",
        }
    }

    pub fn media_type(self) -> &'static str {
        match self {
            Self::Woff2 => "font/woff2",
            Self::Woff => "font/woff",
            Self::TrueType => "font/ttf",
            Self::OpenType => "font/otf",
        }
    }

    fn of_extension(name: &str) -> Option<Self> {
        match name.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
            "woff2" => Some(Self::Woff2),
            "woff" => Some(Self::Woff),
            "ttf" => Some(Self::TrueType),
            "otf" => Some(Self::OpenType),
            _ => None,
        }
    }

    fn matches(self, head: &[u8]) -> bool {
        match self {
            Self::Woff2 => head.starts_with(b"wOF2"),
            Self::Woff => head.starts_with(b"wOFF"),
            // An OpenType file holding TrueType outlines, or Apple's TrueType.
            Self::TrueType => head.starts_with(&[0, 1, 0, 0]) || head.starts_with(b"true"),
            Self::OpenType => head.starts_with(b"OTTO") || head.starts_with(&[0, 1, 0, 0]),
        }
    }
}

/// A font file's name under its theme's `fonts/`, as `fonts/<name>` in a
/// theme: letters, digits, `.`, `-` and `_`, of a font format by its name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontFile(String);

impl FontFile {
    /// `fonts/<name>` as a theme writes it, or the bare name.
    pub fn parse(text: &str) -> Option<Self> {
        let name = text.strip_prefix("fonts/").unwrap_or(text);
        let plain = (1..=64).contains(&name.len())
            && !name.starts_with('.')
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        (plain && FontFormat::of_extension(name).is_some()).then(|| Self(name.to_owned()))
    }

    /// Its name under `fonts/`.
    pub fn name(&self) -> &str {
        &self.0
    }

    pub fn format(&self) -> FontFormat {
        FontFormat::of_extension(&self.0).expect("checked when parsed")
    }

    /// Whether `bytes` are a font of this file's format, small enough.
    pub fn check(&self, bytes: &[u8]) -> Result<FontFormat, String> {
        let format = self.format();
        if bytes.len() as u64 > FACE_BYTES {
            return Err(format!("fonts/{} is over {} KiB", self.0, FACE_BYTES >> 10));
        }
        if !format.matches(bytes) {
            return Err(format!("fonts/{} isn't a {} font", self.0, format.css()));
        }
        Ok(format)
    }
}

impl fmt::Display for FontFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fonts/{}", self.0)
    }
}

impl Serialize for FontFile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_is_known_by_its_first_bytes_as_well_as_its_name() {
        let woff2 = FontFile::parse("fonts/Sans-Regular.woff2").expect("a font");
        assert_eq!(woff2.name(), "Sans-Regular.woff2");
        assert_eq!(woff2.to_string(), "fonts/Sans-Regular.woff2");
        assert_eq!(woff2.check(b"wOF2\0\0"), Ok(FontFormat::Woff2));
        assert!(woff2.check(b"<svg onload=").is_err());
        assert!(woff2.check(b"wOFF").is_err(), "a woff named woff2");
        assert_eq!(FontFile::parse("a.otf").expect("otf").check(b"OTTO"), Ok(FontFormat::OpenType));
        assert_eq!(FontFile::parse("a.ttf").expect("ttf").check(&[0, 1, 0, 0, 9]), Ok(FontFormat::TrueType));
        let big = vec![b'w'; FACE_BYTES as usize + 1];
        assert!(woff2.check(&big).is_err());
    }

    #[test]
    fn a_name_that_could_leave_fonts_or_isnt_a_font_is_refused() {
        for refused in
            ["fonts/../x.woff2", "fonts/a/b.woff2", "fonts/.hidden.woff2", "fonts/x.svg", "fonts/x", "x\".woff2", "x).woff2"]
        {
            assert!(FontFile::parse(refused).is_none(), "{refused}");
        }
    }
}
