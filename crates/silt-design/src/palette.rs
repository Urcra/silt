//! Checks a chart palette as `artifact-dataviz` asks (ADR 0073): a
//! categorical palette's lightness band, chroma, separation under normal
//! vision and colour-vision deficiencies, and contrast; an ordinal ramp's
//! order and steps; and text contrast, by WCAG 2's ratio and by APCA.

use std::fmt;

use crate::frame::Theme;

/// OKLCH lightness a categorical colour keeps to, by theme.
const LIGHT_BAND: (f64, f64) = (0.43, 0.77);
const DARK_BAND: (f64, f64) = (0.48, 0.67);
/// Below it a hue reads as grey.
const CHROMA_FLOOR: f64 = 0.10;
/// OKLab distance ×100 between neighbours under protanopia and
/// deuteranopia: the target, and the floor allowed with a second encoding.
const SEPARATION_TARGET: f64 = 8.0;
const SEPARATION_FLOOR: f64 = 6.0;
/// The same distance under normal vision, a hard floor.
const NORMAL_FLOOR: f64 = 15.0;
/// A thin mark against its surface.
const MARK_CONTRAST: f64 = 3.0;
/// An ordinal ramp's smallest lightness step, its pale end's contrast, and
/// the hue spread it may have.
const ORDINAL_STEP: f64 = 0.06;
const ORDINAL_LIGHT_END: f64 = 2.0;
const ORDINAL_HUE_SPREAD: f64 = 40.0;

/// Machado, Oliveira and Fernandes (2009), severity 1, in linear sRGB. The
/// thresholds above are calibrated to this simulation.
const PROTAN: [[f64; 3]; 3] = [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]];
const DEUTAN: [[f64; 3]; 3] = [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.011820, 0.042940, 0.968881]];
const TRITAN: [[f64; 3]; 3] = [[1.255528, -0.076749, -0.178779], [-0.078411, 0.930809, 0.147602], [0.004733, 0.691367, 0.303900]];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PaletteError {
    #[error("{0} isn't a #rrggbb colour")]
    NotHex(String),
    #[error("a palette needs at least one colour")]
    Empty,
}

/// An sRGB colour, each channel from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Srgb([f64; 3]);

impl Srgb {
    /// `#rrggbb` or `rrggbb`, with surrounding whitespace.
    pub fn parse(hex: &str) -> Result<Self, PaletteError> {
        let trimmed = hex.trim();
        let digits = trimmed.strip_prefix('#').unwrap_or(trimmed);
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(PaletteError::NotHex(trimmed.to_owned()));
        }
        let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).map(|value| f64::from(value) / 255.0);
        match (channel(0), channel(2), channel(4)) {
            (Ok(r), Ok(g), Ok(b)) => Ok(Self([r, g, b])),
            _ => Err(PaletteError::NotHex(trimmed.to_owned())),
        }
    }

    /// From 8-bit channels.
    pub fn from_bytes(r: u8, g: u8, b: u8) -> Self {
        Self([f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0])
    }

    /// A browser's computed colour, `rgb(r, g, b)` or `rgba(r, g, b, a)`,
    /// laid over `ground` when it's translucent; `None` when it's fully
    /// transparent or not one of those.
    pub fn computed(css: &str, ground: Srgb) -> Option<Self> {
        let inner = css.trim().strip_prefix("rgba(").or_else(|| css.trim().strip_prefix("rgb("))?.strip_suffix(')')?;
        let parts: Vec<f64> =
            inner.split([',', ' ', '/']).filter(|part| !part.is_empty()).map(|part| part.parse().ok()).collect::<Option<_>>()?;
        let (rgb, alpha) = match parts.as_slice() {
            [r, g, b] => ([*r, *g, *b], 1.0),
            [r, g, b, a] => ([*r, *g, *b], *a),
            _ => return None,
        };
        if alpha <= 0.0 {
            return None;
        }
        let over = |channel: f64, under: f64| (channel / 255.0).clamp(0.0, 1.0) * alpha + under * (1.0 - alpha);
        Some(Self([over(rgb[0], ground.0[0]), over(rgb[1], ground.0[1]), over(rgb[2], ground.0[2])]))
    }

    fn linear(self) -> [f64; 3] {
        self.0.map(|c| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) })
    }

    /// OKLab (L, a, b).
    pub fn oklab(self) -> [f64; 3] {
        oklab_of(self.linear())
    }

    /// OKLCH lightness, chroma and hue in degrees.
    pub fn oklch(self) -> (f64, f64, f64) {
        let [l, a, b] = self.oklab();
        (l, a.hypot(b), b.atan2(a).to_degrees().rem_euclid(360.0))
    }

    fn luminance(self) -> f64 {
        let [r, g, b] = self.linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }
}

impl fmt::Display for Srgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.0.map(|c| (c * 255.0).round() as u8);
        write!(f, "#{r:02x}{g:02x}{b:02x}")
    }
}

/// A comma-separated list of `#rrggbb` colours.
pub fn parse_list(list: &str) -> Result<Vec<Srgb>, PaletteError> {
    let colors =
        list.split(',').map(str::trim).filter(|item| !item.is_empty()).map(Srgb::parse).collect::<Result<Vec<_>, _>>()?;
    if colors.is_empty() { Err(PaletteError::Empty) } else { Ok(colors) }
}

fn oklab_of([r, g, b]: [f64; 3]) -> [f64; 3] {
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

fn simulated(color: Srgb, matrix: &[[f64; 3]; 3]) -> [f64; 3] {
    let c = color.linear();
    matrix.map(|row| (row[0] * c[0] + row[1] * c[1] + row[2] * c[2]).clamp(0.0, 1.0))
}

/// The distance between two colours in OKLab ×100, as seen with `vision`.
pub fn distance(a: Srgb, b: Srgb, vision: Vision) -> f64 {
    let (x, y) = match vision.matrix() {
        Some(matrix) => (oklab_of(simulated(a, matrix)), oklab_of(simulated(b, matrix))),
        None => (a.oklab(), b.oklab()),
    };
    100.0 * ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vision {
    Normal,
    Protan,
    Deutan,
    Tritan,
}

impl Vision {
    fn matrix(self) -> Option<&'static [[f64; 3]; 3]> {
        match self {
            Self::Normal => None,
            Self::Protan => Some(&PROTAN),
            Self::Deutan => Some(&DEUTAN),
            Self::Tritan => Some(&TRITAN),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Protan => "protan",
            Self::Deutan => "deutan",
            Self::Tritan => "tritan",
        }
    }
}

/// WCAG 2's contrast ratio between two colours.
pub fn contrast(a: Srgb, b: Srgb) -> f64 {
    let (x, y) = (a.luminance(), b.luminance());
    let (hi, lo) = if x >= y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// APCA's lightness contrast (Lc) of `text` on `background`, by the
/// 0.0.98G constants; negative for light text on a dark ground.
pub fn apca(text: Srgb, background: Srgb) -> f64 {
    let y = |color: Srgb| {
        let [r, g, b] = color.0.map(|c| c.powf(2.4));
        let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
        if y >= 0.022 { y } else { y + (0.022 - y).powf(1.414) }
    };
    let (t, b) = (y(text), y(background));
    if (b - t).abs() < 0.0005 {
        return 0.0;
    }
    let c = 1.14 * if b > t { b.powf(0.56) - t.powf(0.57) } else { b.powf(0.65) - t.powf(0.62) };
    if c.abs() < 0.1 { 0.0 } else { (c - c.signum() * 0.027) * 100.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    /// Allowed only with what the check's detail says.
    Warn,
    Fail,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub verdict: Verdict,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    /// No check failed; warnings still oblige what they say.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.verdict != Verdict::Fail)
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for check in &self.checks {
            writeln!(f, "{:<5} {:<22} {}", check.verdict.as_str(), check.name, check.detail)?;
        }
        Ok(())
    }
}

/// Which pairs must be told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pairs {
    /// Neighbours, as in stacks, bars and lines.
    Adjacent,
    /// Every pair, as in scatter plots, maps and small multiples.
    All,
}

fn pairs_of(n: usize, pairs: Pairs) -> Vec<(usize, usize)> {
    match pairs {
        Pairs::Adjacent => (1..n).map(|j| (j - 1, j)).collect(),
        Pairs::All => (0..n).flat_map(|i| (i + 1..n).map(move |j| (i, j))).collect(),
    }
}

/// The categorical checks of `colors`, in the order they're assigned, on
/// `surface` in `theme`.
pub fn categorical(colors: &[Srgb], surface: Srgb, theme: Theme, pairs: Pairs) -> Report {
    let (lo, hi) = match theme {
        Theme::Light => LIGHT_BAND,
        Theme::Dark => DARK_BAND,
    };
    let mut checks = Vec::new();
    let off_band: Vec<String> =
        colors.iter().filter(|c| !(lo..=hi).contains(&c.oklch().0)).map(|c| format!("{c} L {:.3}", c.oklch().0)).collect();
    checks.push(Check {
        name: "lightness band",
        verdict: if off_band.is_empty() { Verdict::Pass } else { Verdict::Fail },
        detail: if off_band.is_empty() {
            format!("all {} within L {lo}–{hi}", colors.len())
        } else {
            format!("outside L {lo}–{hi}: {}", off_band.join(", "))
        },
    });
    let grey: Vec<String> =
        colors.iter().filter(|c| c.oklch().1 < CHROMA_FLOOR).map(|c| format!("{c} C {:.3}", c.oklch().1)).collect();
    checks.push(Check {
        name: "chroma floor",
        verdict: if grey.is_empty() { Verdict::Pass } else { Verdict::Fail },
        detail: if grey.is_empty() {
            format!("all {} at C {CHROMA_FLOOR} or more", colors.len())
        } else {
            format!("read as grey: {}", grey.join(", "))
        },
    });
    let list = pairs_of(colors.len(), pairs);
    let which = match pairs {
        Pairs::Adjacent => "neighbours",
        Pairs::All => "all pairs",
    };
    let worst = |vision: Vision| {
        list.iter().map(|&(i, j)| (distance(colors[i], colors[j], vision), i, j)).min_by(|a, b| a.0.total_cmp(&b.0))
    };
    let deficient = [Vision::Protan, Vision::Deutan]
        .into_iter()
        .filter_map(|vision| worst(vision).map(|found| (found, vision)))
        .min_by(|a, b| a.0.0.total_cmp(&b.0.0));
    let tritan = worst(Vision::Tritan).map_or(String::new(), |(d, _, _)| format!("; tritan {d:.1}"));
    checks.push(match deficient {
        Some(((d, i, j), vision)) => Check {
            name: "separation (CVD)",
            verdict: if d >= SEPARATION_TARGET {
                Verdict::Pass
            } else if d >= SEPARATION_FLOOR {
                Verdict::Warn
            } else {
                Verdict::Fail
            },
            detail: format!(
                "worst of {which}: {} / {} at {d:.1} ({}){tritan}{}",
                colors[i],
                colors[j],
                vision.name(),
                if (SEPARATION_FLOOR..SEPARATION_TARGET).contains(&d) {
                    "; allowed only with direct labels, gaps or texture"
                } else {
                    ""
                }
            ),
        },
        None => Check { name: "separation (CVD)", verdict: Verdict::Pass, detail: "one colour".into() },
    });
    checks.push(match worst(Vision::Normal) {
        Some((d, i, j)) => Check {
            name: "separation (normal)",
            verdict: if d >= NORMAL_FLOOR { Verdict::Pass } else { Verdict::Fail },
            detail: format!(
                "worst of {which}: {} / {} at {d:.1}{}",
                colors[i],
                colors[j],
                if d >= NORMAL_FLOOR { "" } else { "; hard to tell apart even with full colour vision" }
            ),
        },
        None => Check { name: "separation (normal)", verdict: Verdict::Pass, detail: "one colour".into() },
    });
    let faint: Vec<String> = colors
        .iter()
        .filter(|&&c| contrast(c, surface) < MARK_CONTRAST)
        .map(|&c| format!("{c} at {:.2}:1", contrast(c, surface)))
        .collect();
    checks.push(Check {
        name: "contrast",
        verdict: if faint.is_empty() { Verdict::Pass } else { Verdict::Warn },
        detail: if faint.is_empty() {
            format!("all {} at {MARK_CONTRAST}:1 or more on {surface}", colors.len())
        } else {
            format!("under {MARK_CONTRAST}:1 on {surface}, so label directly or add a table view: {}", faint.join(", "))
        },
    });
    Report { checks }
}

/// The checks of an ordinal ramp: one hue whose lightness steps steadily,
/// with its pale end still a mark on `surface`.
pub fn ordinal(colors: &[Srgb], surface: Srgb, theme: Theme) -> Report {
    let lightness: Vec<f64> = colors.iter().map(|c| c.oklch().0).collect();
    let rising = lightness.windows(2).all(|pair| pair[1] > pair[0]);
    let falling = lightness.windows(2).all(|pair| pair[1] < pair[0]);
    let mut checks = vec![Check {
        name: "lightness in order",
        verdict: if rising || falling { Verdict::Pass } else { Verdict::Fail },
        detail: format!("L {}", lightness.iter().map(|l| format!("{l:.3}")).collect::<Vec<_>>().join(", ")),
    }];
    let close: Vec<String> = colors
        .windows(2)
        .zip(lightness.windows(2))
        .filter(|(_, l)| (l[1] - l[0]).abs() < ORDINAL_STEP)
        .map(|(c, l)| format!("{} / {} {:.3} apart", c[0], c[1], (l[1] - l[0]).abs()))
        .collect();
    checks.push(Check {
        name: "steps apart",
        verdict: if close.is_empty() { Verdict::Pass } else { Verdict::Fail },
        detail: if close.is_empty() { format!("every step {ORDINAL_STEP} or more") } else { close.join(", ") },
    });
    let pale = match theme {
        Theme::Light => colors.iter().zip(&lightness).max_by(|a, b| a.1.total_cmp(b.1)),
        Theme::Dark => colors.iter().zip(&lightness).min_by(|a, b| a.1.total_cmp(b.1)),
    };
    if let Some((&pale, _)) = pale {
        let ratio = contrast(pale, surface);
        checks.push(Check {
            name: "pale end",
            verdict: if ratio >= ORDINAL_LIGHT_END { Verdict::Pass } else { Verdict::Fail },
            detail: format!("{pale} at {ratio:.2}:1 on {surface}"),
        });
    }
    let hues: Vec<f64> = colors.iter().map(|c| c.oklch().2).collect();
    let spread = hue_spread(&hues);
    checks.push(Check {
        name: "one hue",
        verdict: if spread <= ORDINAL_HUE_SPREAD { Verdict::Pass } else { Verdict::Fail },
        detail: format!("hues spread {spread:.0}°"),
    });
    Report { checks }
}

/// The smallest arc holding every hue.
fn hue_spread(hues: &[f64]) -> f64 {
    let mut sorted = hues.to_vec();
    sorted.sort_by(f64::total_cmp);
    let Some((&first, &last)) = sorted.first().zip(sorted.last()) else {
        return 0.0;
    };
    let widest_gap = sorted.windows(2).map(|pair| pair[1] - pair[0]).fold(360.0 - (last - first), f64::max);
    360.0 - widest_gap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Srgb {
        Srgb::parse(text).expect("a colour")
    }

    #[test]
    fn colours_parse_and_print_as_hex() {
        assert_eq!(hex(" #C25A3A ").to_string(), "#c25a3a");
        assert_eq!(hex("00ff00").to_string(), "#00ff00");
        for bad in ["#fff", "#gggggg", "", "#1234567"] {
            assert!(Srgb::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_list(" , "), Err(PaletteError::Empty));
        assert_eq!(parse_list("#000000, #ffffff").map(|list| list.len()), Ok(2));
    }

    #[test]
    fn computed_colours_parse_and_blend() {
        let white = hex("#ffffff");
        assert_eq!(Srgb::computed("rgb(0, 0, 0)", white), Some(hex("#000000")));
        assert_eq!(Srgb::computed("rgba(0, 0, 0, 0)", white), None);
        assert_eq!(Srgb::computed("rgba(0, 0, 0, 0.5)", white).map(|c| c.to_string()).as_deref(), Some("#808080"));
        assert_eq!(Srgb::computed("color(srgb 1 0 0)", white), None);
    }

    #[test]
    fn oklab_and_contrast_match_known_values() {
        let white = hex("#ffffff");
        let black = hex("#000000");
        let [l, a, b] = white.oklab();
        assert!((l - 1.0).abs() < 1e-3 && a.abs() < 1e-3 && b.abs() < 1e-3, "{l} {a} {b}");
        let (l, c, h) = hex("#ff0000").oklch();
        assert!((l - 0.628).abs() < 1e-3 && (c - 0.2577).abs() < 1e-3 && (h - 29.23).abs() < 0.1, "{l} {c} {h}");
        assert!((contrast(white, black) - 21.0).abs() < 1e-9);
        assert!((contrast(hex("#767676"), white) - 4.54).abs() < 0.01);
        assert!((apca(black, white) - 106.04).abs() < 0.1, "{}", apca(black, white));
        assert!((apca(white, black) + 107.88).abs() < 0.1, "{}", apca(white, black));
        assert_eq!(apca(white, white), 0.0);
    }

    #[test]
    fn a_good_palette_passes_and_a_bad_one_fails() {
        let surface = hex("#fcfcfb");
        let good = parse_list("#2a78d6,#eb6834,#1baf7a,#eda100").expect("colours");
        let report = categorical(&good, surface, Theme::Light, Pairs::Adjacent);
        assert!(report.checks.iter().all(|check| check.name == "contrast" || check.verdict == Verdict::Pass), "{report}");

        let muddy = parse_list("#808080,#7f8a80").expect("colours");
        let report = categorical(&muddy, surface, Theme::Light, Pairs::Adjacent);
        assert!(!report.passed(), "{report}");
        let verdict = |name: &str| report.checks.iter().find(|check| check.name == name).map(|check| check.verdict);
        assert_eq!(verdict("chroma floor"), Some(Verdict::Fail));
        assert_eq!(verdict("separation (normal)"), Some(Verdict::Fail));
    }

    #[test]
    fn all_pairs_is_stricter_than_neighbours() {
        let colors = parse_list("#2a78d6,#eb6834,#2c7fd0").expect("colours");
        let surface = hex("#fcfcfb");
        assert!(categorical(&colors, surface, Theme::Light, Pairs::Adjacent).passed());
        assert!(!categorical(&colors, surface, Theme::Light, Pairs::All).passed());
    }

    #[test]
    fn an_ordinal_ramp_steps_one_hue_in_order() {
        let surface = hex("#fcfcfb");
        let ramp = parse_list("#86b6ef,#5598e7,#256abf,#104281").expect("colours");
        assert!(ordinal(&ramp, surface, Theme::Light).passed(), "{}", ordinal(&ramp, surface, Theme::Light));
        let shuffled = parse_list("#5598e7,#86b6ef,#256abf").expect("colours");
        assert!(!ordinal(&shuffled, surface, Theme::Light).passed());
        let two_hues = parse_list("#86b6ef,#e8862a,#104281").expect("colours");
        assert!(!ordinal(&two_hues, surface, Theme::Light).passed());
        assert!((hue_spread(&[350.0, 10.0]) - 20.0).abs() < 1e-9);
    }
}
