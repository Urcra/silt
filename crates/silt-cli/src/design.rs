//! `silt design`: checking a chart palette's colours and text contrast,
//! as the `artifact-dataviz` skill asks (ADR 0073).

use clap::{Subcommand, ValueEnum};
use exn::Exn;
use silt_design::frame::Theme;
use silt_design::palette::{self, Pairs, Srgb};

use crate::CliError;
use crate::commands::fail;

#[derive(Debug, Subcommand)]
pub enum DesignCommand {
    /// Check a chart palette: lightness, chroma, separation under normal vision and colour-vision deficiencies, and contrast
    Palette {
        /// The colours in the order they're assigned, comma-separated, such as `#c25a3a,#5b7a8a`
        colors: String,
        /// The chart's surface, such as the design system's `bg`
        #[arg(long)]
        surface: String,
        /// The theme the surface belongs to, which sets the lightness band
        #[arg(long, value_enum, default_value_t = ThemeArg::Light)]
        theme: ThemeArg,
        /// Which pairs must be told apart: neighbours, as in bars and lines, or all, as in scatter plots and maps
        #[arg(long, value_enum, default_value_t = PairsArg::Adjacent)]
        pairs: PairsArg,
        /// Check an ordinal ramp instead: one hue stepping steadily in lightness
        #[arg(long)]
        ordinal: bool,
        /// Print the checks as JSON, `{"passed": …, "checks": [{"name", "verdict", "detail"}]}`
        #[arg(long)]
        json: bool,
    },
    /// Text contrast of one colour on another: WCAG 2's ratio and APCA's lightness contrast
    Contrast {
        /// The text's colour, `#rrggbb`
        text: String,
        /// The colour behind it, `#rrggbb`
        ground: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ThemeArg {
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PairsArg {
    Adjacent,
    All,
}

pub fn run(command: DesignCommand) -> Result<(), Exn<CliError>> {
    match command {
        DesignCommand::Palette { colors, surface, theme, pairs, ordinal, json } => {
            let colors = palette::parse_list(&colors).map_err(|error| fail(error.to_string()))?;
            let surface = Srgb::parse(&surface).map_err(|error| fail(error.to_string()))?;
            let theme = match theme {
                ThemeArg::Light => Theme::Light,
                ThemeArg::Dark => Theme::Dark,
            };
            let pairs = match pairs {
                PairsArg::Adjacent => Pairs::Adjacent,
                PairsArg::All => Pairs::All,
            };
            let report = if ordinal {
                palette::ordinal(&colors, surface, theme)
            } else {
                palette::categorical(&colors, surface, theme, pairs)
            };
            if json {
                println!("{}", serde_json::json!({ "passed": report.passed(), "checks": report.checks }));
            } else {
                print!("{report}");
            }
            if report.passed() { Ok(()) } else { Err(fail("the palette fails a check; fix each FAIL")) }
        }
        DesignCommand::Contrast { text, ground } => {
            let text = Srgb::parse(&text).map_err(|error| fail(error.to_string()))?;
            let ground = Srgb::parse(&ground).map_err(|error| fail(error.to_string()))?;
            let ratio = palette::contrast(text, ground);
            let wcag = if ratio >= 4.5 {
                "passes for text"
            } else if ratio >= 3.0 {
                "passes for large text and marks only"
            } else {
                "fails for text"
            };
            println!("WCAG 2  {ratio:.2}:1  {wcag}");
            println!("APCA    Lc {:.1}", palette::apca(text, ground));
            Ok(())
        }
    }
}
