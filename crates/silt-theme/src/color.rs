//! Colours as a theme writes them: hex only, so what reaches the CSS is
//! always printed by us.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use silt_design::palette::{Srgb, contrast};

/// An sRGB colour with alpha, 8 bits a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} isn't a hex colour (#rgb, #rgba, #rrggbb or #rrggbbaa)")]
pub struct NotHex(pub String);

impl Rgba {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn parse(text: &str) -> Result<Self, NotHex> {
        let bad = || NotHex(text.to_owned());
        let digits = text.trim().strip_prefix('#').ok_or_else(bad)?;
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let nibble = |at: usize| u8::from_str_radix(&digits[at..=at], 16).map(|n| n * 17).map_err(|_| bad());
        let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).map_err(|_| bad());
        match digits.len() {
            3 => Ok(Self::rgb(nibble(0)?, nibble(1)?, nibble(2)?)),
            4 => Ok(Self { r: nibble(0)?, g: nibble(1)?, b: nibble(2)?, a: nibble(3)? }),
            6 => Ok(Self::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Ok(Self { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: byte(6)? }),
            _ => Err(bad()),
        }
    }

    pub fn opaque(self) -> bool {
        self.a == 255
    }

    pub fn with_alpha(self, alpha: f64) -> Self {
        Self { a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8, ..self }
    }

    /// This colour laid over `ground`, which is taken as opaque.
    pub fn over(self, ground: Self) -> Self {
        let alpha = f64::from(self.a) / 255.0;
        let mix = |top: u8, under: u8| (f64::from(top) * alpha + f64::from(under) * (1.0 - alpha)).round() as u8;
        Self::rgb(mix(self.r, ground.r), mix(self.g, ground.g), mix(self.b, ground.b))
    }

    pub(crate) fn srgb(self) -> Srgb {
        Srgb::from_bytes(self.r, self.g, self.b)
    }

    pub(crate) fn from_srgb(color: Srgb) -> Self {
        let [r, g, b] = color.bytes();
        Self::rgb(r, g, b)
    }

    pub(crate) fn oklab(self) -> [f64; 3] {
        self.srgb().oklab()
    }

    pub(crate) fn from_oklab(lab: [f64; 3]) -> Self {
        Self::from_srgb(Srgb::from_oklab(lab))
    }

    /// `amount` of the way from this colour to `other`, in OKLab; opaque.
    pub fn mix(self, other: Self, amount: f64) -> Self {
        let (a, b) = (self.oklab(), other.oklab());
        Self::from_oklab([0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * amount))
    }

    /// OKLab lightness, 0 to 1.
    pub fn lightness(self) -> f64 {
        self.oklab()[0]
    }

    /// The same hue and chroma at another OKLab lightness.
    pub fn at_lightness(self, lightness: f64) -> Self {
        let [_, a, b] = self.oklab();
        Self { a: self.a, ..Self::from_oklab([lightness.clamp(0.0, 1.0), a, b]) }
    }

    /// WCAG 2's ratio of this colour on `ground`, a translucent one first
    /// laid over it.
    pub fn contrast_on(self, ground: Self) -> f64 {
        contrast(self.over(ground).srgb(), ground.srgb())
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if !self.opaque() {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// `color` moved in OKLab lightness, away from the grounds, until it reaches
/// `ratio` on each of them, or as far as it can go.
pub(crate) fn solve(color: Rgba, grounds: &[Rgba], ratio: f64) -> Rgba {
    let worst = |candidate: Rgba| grounds.iter().map(|ground| candidate.contrast_on(*ground)).fold(f64::INFINITY, f64::min);
    if worst(color) >= ratio {
        return color;
    }
    let ground_lightness = grounds.iter().map(|ground| ground.lightness()).sum::<f64>() / grounds.len().max(1) as f64;
    let step = if ground_lightness > 0.5 { -0.005 } else { 0.005 };
    let mut lightness = color.lightness();
    let mut best = color;
    while (0.0..=1.0).contains(&lightness) {
        lightness += step;
        best = color.at_lightness(lightness);
        if worst(best) >= ratio {
            break;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hex_form_parses_and_prints_back_in_full() {
        assert_eq!(Rgba::parse("#abc").unwrap().to_string(), "#aabbcc");
        assert_eq!(Rgba::parse("#abc8").unwrap().to_string(), "#aabbcc88");
        assert_eq!(Rgba::parse(" #D9883A ").unwrap().to_string(), "#d9883a");
        assert_eq!(Rgba::parse("#d9883a1a").unwrap().a, 0x1a);
        assert_eq!(Rgba::parse("#d9883aff").unwrap().to_string(), "#d9883a");
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in ["d9883a", "#d9883", "#gggggg", "red", "rgb(1,2,3)", "#12;}", "", "#ééé"] {
            assert!(Rgba::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn solving_reaches_the_ratio_away_from_the_ground() {
        let paper = Rgba::parse("#f6f1e8").unwrap();
        let faint = Rgba::parse("#b8a98a").unwrap();
        let solved = solve(faint, &[paper], 4.5);
        assert!(solved.contrast_on(paper) >= 4.5, "{solved}");
        assert!(solved.lightness() < faint.lightness());
        let ink = Rgba::parse("#1a1714").unwrap();
        let dim = Rgba::parse("#3a3127").unwrap();
        assert!(solve(dim, &[ink], 4.5).lightness() > dim.lightness());
        assert_eq!(solve(ink, &[paper], 4.5), ink, "a colour that passes is kept");
    }
}
