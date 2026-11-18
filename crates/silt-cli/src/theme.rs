//! `silt theme`: themes for Silt's UI, listed, started from another,
//! imported from VS Code or Zed, and checked (ADR 0098).

use std::fs;
use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use exn::Exn;
use silt_daemon::paths;
use silt_theme::check::Kind;
use silt_theme::import::{self, Options};
use silt_theme::{Appearance, Family, ThemeId};

use crate::CliError;
use crate::commands::fail;

/// The largest file `import` and `check` read.
const MOST_BYTES: u64 = 4 << 20;

#[derive(Debug, Subcommand)]
pub enum ThemeCommand {
    /// List the themes for Silt's UI, Silt's and yours, with the light and dark ones picked
    List {
        /// Print the list as JSON, as the web UI reads it
        #[arg(long)]
        json: bool,
    },
    /// Start a theme of your own from one of Silt's or yours, in your themes folder
    New {
        /// The new theme's id: lowercase letters, digits and hyphens
        id: String,
        /// The theme to start from
        #[arg(long, default_value = silt_theme::DEFAULT_LIGHT)]
        from: String,
        /// Its name, as Settings shows it; from the id when left out
        #[arg(long)]
        name: Option<String>,
        /// Replace a file of yours with the same name
        #[arg(long)]
        force: bool,
    },
    /// Read a VS Code colour theme or a Zed theme family into Silt's format, in your themes folder
    Import {
        /// The theme's JSON file; VS Code's may have comments
        file: PathBuf,
        /// The id of the theme, or the stem of a family's ids; from its name when left out
        #[arg(long)]
        to: Option<String>,
        /// Whether a VS Code theme that doesn't say is light or dark; guessed from its background when left out
        #[arg(long, value_enum)]
        appearance: Option<AppearanceArg>,
        /// Move the colours it sets that fail as text until they reach 4.5:1
        #[arg(long)]
        fix_contrast: bool,
        /// Replace a file of yours with the same name
        #[arg(long)]
        force: bool,
    },
    /// Check a theme file in Silt's format: what Silt can't read in it, and its text's contrast
    Check {
        file: PathBuf,
        /// Move the colours it sets that fail as text until they reach 4.5:1, and save the file
        #[arg(long)]
        fix_contrast: bool,
        /// Print the warnings and checks as JSON
        #[arg(long)]
        json: bool,
    },
    /// Print the folder your themes go in
    Path,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AppearanceArg {
    Light,
    Dark,
}

pub fn run(command: ThemeCommand) -> Result<(), Exn<CliError>> {
    let data = paths::data_dir();
    match command {
        ThemeCommand::List { json } => list(&data, json),
        ThemeCommand::New { id, from, name, force } => new(&data, &id, &from, name, force),
        ThemeCommand::Import { file, to, appearance, fix_contrast, force } => {
            let options = Options {
                id: to.map(|to| to.parse::<ThemeId>().map_err(|error| fail(error.to_string()))).transpose()?,
                appearance: appearance.map(|appearance| match appearance {
                    AppearanceArg::Light => Appearance::Light,
                    AppearanceArg::Dark => Appearance::Dark,
                }),
            };
            import_file(&data, &file, &options, fix_contrast, force)
        }
        ThemeCommand::Check { file, fix_contrast, json } => check_file(&file, fix_contrast, json),
        ThemeCommand::Path => {
            println!("{}", silt_daemon::theme::themes_dir(&data).display());
            Ok(())
        }
    }
}

fn list(data: &Path, json: bool) -> Result<(), Exn<CliError>> {
    let catalog = silt_daemon::theme::catalog(data);
    if json {
        println!("{}", serde_json::to_string_pretty(&catalog).map_err(|error| fail(error.to_string()))?);
        return Ok(());
    }
    println!("{:<24}  {:<10}  {:<12}  ", "ID", "APPEARANCE", "FROM");
    for theme in &catalog.themes {
        let from = match theme.source {
            silt_proto::view::ThemeSource::Bundled => "bundled",
            silt_proto::view::ThemeSource::Yours => "yours",
            silt_proto::view::ThemeSource::ReplacesBundled => "yours, over Silt's",
        };
        let mut notes = Vec::new();
        if theme.id == catalog.appearance.light {
            notes.push("● light theme".to_owned());
        }
        if theme.id == catalog.appearance.dark {
            notes.push("● dark theme".to_owned());
        }
        if !theme.warnings.is_empty() {
            notes.push(format!("{} warnings", theme.warnings.len()));
        }
        if theme.failing > 0 {
            notes.push(format!("{} below 4.5:1", theme.failing));
        }
        println!("{:<24}  {:<10}  {:<12}  {}", theme.id, theme.appearance, from, notes.join(", "));
    }
    for problem in &catalog.problems {
        println!("unreadable: {}: {}", problem.file, problem.message);
    }
    Ok(())
}

fn new(data: &Path, id: &str, from: &str, name: Option<String>, force: bool) -> Result<(), Exn<CliError>> {
    let id: ThemeId = id.parse().map_err(|error: silt_theme::model::BadThemeId| fail(error.to_string()))?;
    let (entries, _) = silt_daemon::theme::entries(data);
    let source = entries
        .iter()
        .find(|entry| entry.theme.id.as_str() == from)
        .ok_or_else(|| fail(format!("no theme {from}; `silt theme list` shows them")))?;
    let mut theme = source.theme.clone();
    let name = name.unwrap_or_else(|| title(id.as_str()));
    theme.id = id.clone();
    theme.name = name.clone();
    let faces = theme.fonts.faces.clone();
    let family = Family::new(name.clone(), vec![theme]);
    let fonts_from = silt_daemon::theme::source_of(data, from).and_then(|found| found.root).filter(|_| !faces.is_empty());
    let path = match fonts_from {
        // A theme with fonts stays a folder, its fonts copied beside it.
        Some(from_root) => {
            let root = silt_daemon::theme::themes_dir(data).join(id.as_str());
            if root.exists() && !force {
                return Err(fail(format!("{} exists; --force replaces it", root.display())));
            }
            silt_daemon::theme::copy_fonts(&from_root, &root, &faces).map_err(|error| fail(error.to_string()))?;
            let path = root.join(silt_daemon::theme::FOLDER_FILE);
            fs::write(&path, family.to_json()).map_err(|error| fail(format!("{}: {error}", path.display())))?;
            path
        }
        None => write_family(data, id.as_str(), &family, force)?,
    };
    println!("Wrote {} ({name}, {}), from {}.", path.display(), source.theme.appearance, source.theme.name);
    println!("Silt shows changes to it as you save, once you pick it in Settings › Appearance.");
    Ok(())
}

fn title(id: &str) -> String {
    id.split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn read(file: &Path) -> Result<String, Exn<CliError>> {
    let meta = fs::metadata(file).map_err(|error| fail(format!("{}: {error}", file.display())))?;
    if meta.len() > MOST_BYTES {
        return Err(fail(format!("{} is over {} MiB", file.display(), MOST_BYTES >> 20)));
    }
    fs::read_to_string(file).map_err(|error| fail(format!("{}: {error}", file.display())))
}

fn write_family(data: &Path, stem: &str, family: &Family, force: bool) -> Result<PathBuf, Exn<CliError>> {
    let dir = silt_daemon::theme::themes_dir(data);
    fs::create_dir_all(&dir).map_err(|error| fail(format!("{}: {error}", dir.display())))?;
    let path = dir.join(format!("{stem}.json"));
    if path.exists() && !force {
        return Err(fail(format!("{} exists; --force replaces it", path.display())));
    }
    fs::write(&path, family.to_json()).map_err(|error| fail(format!("{}: {error}", path.display())))?;
    Ok(path)
}

fn import_file(data: &Path, file: &Path, options: &Options, fix_contrast: bool, force: bool) -> Result<(), Exn<CliError>> {
    let text = read(file)?;
    let mut imported = import::import(&text, options).map_err(|error| fail(error.to_string()))?;
    let fixes = if fix_contrast { import::fix_contrast(&mut imported.family) } else { Vec::new() };
    let stem = options
        .id
        .clone()
        .or_else(|| ThemeId::from_name(&imported.family.name))
        .ok_or_else(|| fail("the theme's name makes no file name; give one with --to"))?;
    let path = write_family(data, stem.as_str(), &imported.family, force)?;
    let themes: Vec<String> =
        imported.family.themes.iter().map(|theme| format!("{} ({})", theme.name, theme.appearance)).collect();
    println!("Read a {} theme: {}", imported.format.name(), themes.join(", "));
    for mapped in &imported.mapped {
        let derived: Vec<&str> = mapped.derived.iter().map(|token| token.as_str()).collect();
        println!("  mapped   {}: {} colours, {} syntax tags", mapped.id, mapped.colors, mapped.syntax);
        if !derived.is_empty() {
            println!("  derived  {}", derived.join(", "));
        }
    }
    for ignored in &imported.ignored {
        println!("  ignored  {ignored}");
    }
    for note in &imported.notes {
        println!("  guessed  {note}");
    }
    for fix in &fixes {
        println!("  nudged   {} {}: {} → {}", fix.theme, fix.what, fix.from, fix.to);
    }
    let failing = report_failures(&imported.family);
    println!("Wrote {}", path.display());
    if failing > 0 && !fix_contrast {
        println!("Fix the failures with `silt theme check {} --fix-contrast`, or edit the file.", path.display());
    }
    Ok(())
}

/// Prints the text pairs below 4.5:1; their count.
fn report_failures(family: &Family) -> usize {
    let mut failing = 0;
    for theme in &family.themes {
        for finding in silt_theme::check(&silt_theme::derive(theme)) {
            if finding.kind == Kind::Text && !finding.passes() {
                failing += 1;
                println!("  fails    {} {}: {:.2}:1, needs {}", theme.id, finding.pair, finding.ratio, finding.needs);
            }
        }
    }
    failing
}

fn check_file(path: &Path, fix_contrast: bool, json: bool) -> Result<(), Exn<CliError>> {
    let source = silt_daemon::theme::Source::at(path);
    let file = source.json.as_path();
    let text = read(file)?;
    // Fixed as written, with every face, whichever of them the check below leaves out.
    let (mut written, _) = silt_theme::parse(&text).map_err(|error| fail(format!("{}: {error}", file.display())))?;
    let fixes = if fix_contrast { import::fix_contrast(&mut written) } else { Vec::new() };
    if !fixes.is_empty() {
        fs::write(file, written.to_json()).map_err(|error| fail(format!("{}: {error}", file.display())))?;
    }
    let (family, warnings) =
        silt_daemon::theme::read_source(&source).map_err(|error| fail(format!("{}: {error}", file.display())))?;
    let findings: Vec<_> =
        family.themes.iter().map(|theme| (theme.id.to_string(), silt_theme::check(&silt_theme::derive(theme)))).collect();
    let passed = findings.iter().all(|(_, findings)| silt_theme::passes(findings));
    if json {
        let themes: Vec<_> = findings.iter().map(|(id, findings)| serde_json::json!({ "id": id, "checks": findings })).collect();
        println!("{}", serde_json::json!({ "passed": passed, "warnings": warnings, "fixes": fixes, "themes": themes }));
    } else {
        for warning in &warnings {
            println!("  warning  {warning}");
        }
        for fix in &fixes {
            println!("  nudged   {} {}: {} → {}", fix.theme, fix.what, fix.from, fix.to);
        }
        report_failures(&family);
        for (id, findings) in &findings {
            for finding in findings.iter().filter(|finding| finding.kind == Kind::Mark && !finding.passes()) {
                println!("  advice   {id} {}: {:.2}:1, marks want {}", finding.pair, finding.ratio, finding.needs);
            }
            if silt_theme::passes(findings) {
                println!("{id} passes.");
            }
        }
    }
    if passed { Ok(()) } else { Err(fail("text below 4.5:1; fix each failure, or run with --fix-contrast")) }
}
