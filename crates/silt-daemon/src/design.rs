//! The design systems an artifact is styled with (ADR 0071): a
//! repository's or the person's, the one that applies to a run's new
//! version, and what Settings shows of them and starts (ADR 0088).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use silt_design::SystemName;

use crate::core::CoreError;
use crate::protocol::{
    ArtifactSettings, DesignAssetView, DesignComponentView, DesignDetail, DesignFrom, DesignKeep, DesignSource, DesignStart,
    DesignSummary, DesignSystemView, DesignTheme, DesignToken, DesignTypeStyle,
};

/// A repository's systems, under its checkout.
pub(crate) const PROJECT_DIR: &str = ".silt/design-systems";
/// The person's own, under the data dir.
pub(crate) const PERSONAL_DIR: &str = "design-systems";
/// A system's components' CSS and their examples, in its folder (ADR 0086).
pub(crate) const COMPONENTS_CSS: &str = "components.css";
pub(crate) const COMPONENTS_DIR: &str = "components";
/// A system's cover and its assets (ADR 0088).
pub(crate) const COVER: &str = "cover.html";
pub(crate) const ASSETS_DIR: &str = "assets";
/// What `design_system` takes to style nothing.
pub(crate) const NONE: &str = "none";
/// The system the person picked for each project on this machine.
const PROJECT_CHOICES: &str = "project-design-systems.json";
/// How this machine's frames load libraries and fonts.
const SETTINGS: &str = "artifacts.json";
const README_BYTES: u64 = 256 << 10;
const TOKENS_BYTES: u64 = 512 << 10;

/// A design system: a folder of a repository's or the person's. Silt ships
/// none of its own (ADR 0087).
#[derive(Debug, Clone)]
pub(crate) struct System {
    pub(crate) name: SystemName,
    pub(crate) source: DesignSource,
    dir: PathBuf,
}

impl System {
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn readme(&self) -> String {
        read_capped(&self.dir.join("README.md"), README_BYTES).unwrap_or_default()
    }

    pub(crate) fn tokens(&self) -> Result<silt_design::Tokens, CoreError> {
        let json = read_capped(&self.dir.join("tokens.json"), TOKENS_BYTES)
            .ok_or_else(|| CoreError::Invalid(format!("the design system {} has no readable tokens.json", self.name)))?;
        silt_design::parse(&json).map_err(|error| CoreError::Invalid(format!("the design system {}: {error}", self.name)))
    }

    /// The font files its tokens name, each under its own `fonts/`.
    fn fonts(&self, tokens: &silt_design::Tokens) -> BTreeMap<String, Vec<u8>> {
        let Ok(root) = silt_os::canonicalize(&self.dir.join("fonts")) else {
            return BTreeMap::new();
        };
        let files = tokens.typography.iter().flat_map(|typography| typography.fonts.iter()).filter_map(|font| font.file.clone());
        let mut out = BTreeMap::new();
        for file in files {
            let Some(inside) = file.strip_prefix("fonts/") else {
                continue;
            };
            let Ok(real) = silt_os::canonicalize(&root.join(inside)) else {
                continue;
            };
            if !real.starts_with(&root) {
                continue;
            }
            if let Ok(meta) = fs::metadata(&real)
                && meta.is_file()
                && meta.len() <= silt_design::FONT_BYTES as u64
                && let Ok(bytes) = fs::read(&real)
            {
                out.insert(file, bytes);
            }
        }
        out
    }

    /// Its `components.css`, empty when it has none (ADR 0086).
    fn components_css(&self) -> String {
        read_capped(&self.dir.join(COMPONENTS_CSS), silt_design::COMPONENTS_CSS_BYTES as u64).unwrap_or_default()
    }

    /// Its components' examples and guidelines, and what was left out of
    /// them.
    pub(crate) fn components(&self) -> (Vec<silt_design::Component>, Vec<String>) {
        let dir = self.dir.join(COMPONENTS_DIR);
        let files = component_files(&dir);
        let guides = component_guides(&dir, files.keys());
        silt_design::components(files.iter().map(|(name, html)| silt_design::ComponentFiles {
            name,
            html,
            guide: guides.get(name).map(String::as_str),
        }))
    }

    /// Its cover, when it has one that can be drawn (ADR 0088).
    pub(crate) fn cover(&self) -> Option<String> {
        let html = read_capped(&self.dir.join(COVER), silt_design::COVER_BYTES as u64)?;
        silt_design::cover(&html).ok().filter(|cover| !cover.is_empty())
    }

    /// Its assets by path under `assets/`, with their media types and sizes,
    /// and what was left out.
    pub(crate) fn assets(&self) -> (Vec<(String, &'static str, usize)>, Vec<String>) {
        silt_design::assets(asset_files(&self.dir.join(ASSETS_DIR)))
    }

    /// One asset's bytes and media type, when the system keeps it.
    pub(crate) fn asset(&self, path: &str) -> Option<(Vec<u8>, &'static str)> {
        let (kept, _) = self.assets();
        let (_, media_type, _) = kept.into_iter().find(|(kept, _, _)| kept == path)?;
        let file = self.dir.join(ASSETS_DIR).join(path);
        let meta = fs::symlink_metadata(&file).ok()?;
        (meta.is_file() && meta.len() <= silt_design::ASSET_BYTES as u64)
            .then(|| fs::read(&file).ok())
            .flatten()
            .map(|bytes| (bytes, media_type))
    }

    /// The CSS its tokens and components compile to, for a frame.
    pub(crate) fn compiled(&self) -> Result<silt_design::Compiled, CoreError> {
        let tokens = self.tokens()?;
        let fonts = self.fonts(&tokens);
        let mut compiled = silt_design::compile(&tokens, &fonts);
        compiled.add_components(&self.components_css());
        Ok(compiled)
    }

    pub(crate) fn view(&self, with_readme: bool) -> DesignSystemView {
        let tokens = self.tokens().ok();
        DesignSystemView {
            name: self.name.to_string(),
            title: tokens.as_ref().map_or_else(|| self.name.to_string(), |tokens| tokens.name.clone()),
            source: self.source,
            readme: with_readme.then(|| self.readme()),
            tokens: if with_readme { tokens.as_ref().map(silt_design::token_names).unwrap_or_default() } else { BTreeMap::new() },
            components: if with_readme {
                self.components()
                    .0
                    .into_iter()
                    .map(|component| DesignComponentView {
                        name: component.name,
                        about: component.about,
                        html: component.html,
                        guide: component.guide,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            dir: with_readme.then(|| self.dir.display().to_string()),
            assets: if with_readme { self.assets().0.into_iter().map(|(path, _, _)| path).collect() } else { Vec::new() },
        }
    }
}

/// The guidelines beside the components named, `<name>.md`, each read
/// only when it's small enough to be kept.
pub(crate) fn component_guides<'a>(dir: &Path, names: impl Iterator<Item = &'a String>) -> BTreeMap<String, String> {
    names
        .filter_map(|name| {
            let guide = read_capped(&dir.join(format!("{name}.md")), silt_design::COMPONENT_BYTES as u64)?;
            Some((name.clone(), guide))
        })
        .collect()
}

/// The regular files directly in each group folder under a system's
/// `assets/`, as `<Group>/<file>` with their sizes. Links aren't followed.
pub(crate) fn asset_files(dir: &Path) -> Vec<(String, usize)> {
    let Ok(groups) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for group in groups.flatten() {
        let Ok(kind) = group.file_type() else { continue };
        let Some(group_name) = group.file_name().to_str().map(str::to_owned) else { continue };
        if !kind.is_dir() {
            continue;
        }
        let Ok(files) = fs::read_dir(group.path()) else { continue };
        for file in files.flatten() {
            let Ok(meta) = fs::symlink_metadata(file.path()) else { continue };
            let Some(file_name) = file.file_name().to_str().map(str::to_owned) else { continue };
            if meta.is_file() {
                out.push((format!("{group_name}/{file_name}"), usize::try_from(meta.len()).unwrap_or(usize::MAX)));
            }
            // A folder's worth of names is enough to report; the rest isn't kept.
            if out.len() > 4 * silt_design::MAX_ASSETS {
                return out;
            }
        }
    }
    out
}

/// The `.html` files directly in a system's `components/`, by stem: the
/// first that a system may hold, each read only when it's small enough to
/// be kept.
pub(crate) fn component_files(dir: &Path) -> BTreeMap<String, String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return BTreeMap::new();
    };
    let names: std::collections::BTreeSet<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.strip_suffix(".html").map(str::to_owned))
        .filter(|name| silt_design::valid_component_name(name))
        .collect();
    names
        .into_iter()
        .take(silt_design::MAX_COMPONENTS)
        .filter_map(|name| {
            let html = read_capped(&dir.join(format!("{name}.html")), silt_design::COMPONENT_BYTES as u64)?;
            Some((name, html))
        })
        .collect()
}

impl System {
    /// It as a row in Settings, with what picks it (ADR 0088).
    pub(crate) fn summary(&self, uses: Vec<String>, thread: Option<String>, unmerged: bool) -> DesignSummary {
        let tokens = self.tokens().ok();
        let rows = tokens.as_ref().map(silt_design::token_rows).unwrap_or_default();
        let first = tokens.as_ref().and_then(|tokens| silt_design::theme_list(tokens).into_iter().next()).map(|(id, _)| id);
        let value = |row: &silt_design::TokenRow| {
            first.as_ref().and_then(|theme| row.values.get(theme)).or_else(|| row.values.get("*")).cloned()
        };
        let colors: Vec<&silt_design::TokenRow> = rows.iter().filter(|row| row.family == "color").collect();
        let preferred = ["bg", "surface-1", "fg-1", "accent", "chart-1", "chart-2", "chart-3", "chart-5"];
        let mut swatches: Vec<String> =
            preferred.iter().filter_map(|name| colors.iter().find(|row| row.name == *name).and_then(|row| value(row))).collect();
        if swatches.len() < 4 {
            swatches = colors.iter().filter_map(|row| value(row)).take(8).collect();
        }
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        DesignSummary {
            name: self.name.to_string(),
            title: tokens.as_ref().map_or_else(|| self.name.to_string(), |tokens| tokens.name.clone()),
            source: self.source,
            dir: self.dir.display().to_string(),
            swatches,
            colors: count(colors.len()),
            type_styles: count(tokens.as_ref().map(silt_design::type_rows).unwrap_or_default().len()),
            components: count(component_files(&self.dir.join(COMPONENTS_DIR)).len()),
            uses,
            thread,
            unmerged,
            chat: false,
        }
    }

    /// Its page: everything Settings shows of it.
    pub(crate) fn detail(&self, summary: DesignSummary) -> DesignDetail {
        let mut dropped = Vec::new();
        let tokens = match self.tokens() {
            Ok(tokens) => Some(tokens),
            Err(error) => {
                dropped.push(error.to_string());
                None
            }
        };
        let css = match self.compiled() {
            Ok(compiled) => {
                dropped.extend(compiled.dropped);
                compiled.css
            }
            Err(_) => String::new(),
        };
        let (components, left_out) = self.components();
        dropped.extend(left_out);
        let (assets, left_out) = self.assets();
        dropped.extend(left_out);
        DesignDetail {
            summary,
            readme: self.readme(),
            made_from: tokens.as_ref().and_then(silt_design::made_from),
            themes: tokens
                .as_ref()
                .map(silt_design::theme_list)
                .unwrap_or_default()
                .into_iter()
                .map(|(id, name)| DesignTheme { id, name })
                .collect(),
            tokens: tokens
                .as_ref()
                .map(silt_design::token_rows)
                .unwrap_or_default()
                .into_iter()
                .map(|row| DesignToken { family: row.family, name: row.name, values: row.values, usage: row.usage })
                .collect(),
            type_styles: tokens
                .as_ref()
                .map(silt_design::type_rows)
                .unwrap_or_default()
                .into_iter()
                .map(|row| DesignTypeStyle {
                    name: row.name,
                    group: row.group,
                    family: row.family,
                    font_size: row.font_size,
                    line_height: row.line_height,
                    font_weight: row.font_weight,
                    letter_spacing: row.letter_spacing,
                    usage: row.usage,
                })
                .collect(),
            css,
            components: components
                .into_iter()
                .map(|component| DesignComponentView {
                    name: component.name,
                    about: component.about,
                    html: component.html,
                    guide: component.guide,
                })
                .collect(),
            cover: self.cover(),
            assets: assets
                .into_iter()
                .map(|(path, media_type, size)| DesignAssetView {
                    path,
                    media_type: media_type.to_owned(),
                    size: u32::try_from(size).unwrap_or(u32::MAX),
                })
                .collect(),
            files: folder_files(&self.dir),
            dropped,
        }
    }
}

/// The files in a system's folder by relative path, `/`-separated, without
/// following links: at most a few hundred, which is more than a system has.
fn folder_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
            let Ok(meta) = fs::symlink_metadata(entry.path()) else { continue };
            let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            if meta.is_dir() {
                pending.push((entry.path(), path));
            } else if meta.is_file() {
                out.push(path);
            }
            if out.len() >= 400 {
                out.sort();
                return out;
            }
        }
    }
    out.sort();
    out
}

/// A new system's folder name from what people call it: "Harbor" is
/// `harbor`, "Acme Web 2" is `acme-web-2`.
pub(crate) fn slug(title: &str) -> Option<SystemName> {
    let mut name = String::new();
    for c in title.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            name.push(c);
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    let name: String = name.trim_end_matches('-').chars().take(64).collect();
    name.trim_end_matches('-').parse().ok()
}

/// The first prompt of a thread that makes `name` in `dir`, or changes it
/// (ADR 0088). The skill holds the how; this says what and where.
pub(crate) fn start_prompt(start: &DesignStart, name: &SystemName, dir: &str, project: Option<&str>) -> String {
    let title = start.title.trim();
    let notes = start.notes.as_deref().map(str::trim).filter(|notes| !notes.is_empty());
    if let Some(change) = start.change.as_deref().map(str::trim) {
        let mut out = format!(
            "Change the design system {title} (`{name}`), in `{dir}`, with Silt's design-system skill: load it with \
silt_use_skill before anything else, and follow its \"Revising and syncing again\". Read its README, its tokens and \
the files you'll change first, change only what's asked, and keep everything else as it is.\n\nWhat to change:\n\n{change}\n"
        );
        if !start.attachments.is_empty() {
            out.push_str("\nThe files attached are part of the request.\n");
        }
        return out;
    }
    let from = match start.from.unwrap_or(DesignFrom::Code) {
        DesignFrom::Code => format!(
            "{}'s code, following the skill's `references/from-code.md`",
            project.map_or_else(|| "this project".to_owned(), |project| format!("the {project} project"))
        ),
        DesignFrom::Website => match start.link.as_deref().map(str::trim).filter(|link| !link.is_empty()) {
            Some(link) if start.attachments.is_empty() => {
                format!("the website at {link}, following the skill's `references/from-website.md`")
            }
            Some(link) => format!(
                "the website at {link} and the files attached (a brand guide or screenshots), following the skill's \
`references/from-website.md`"
            ),
            None => {
                "the files attached (a brand guide or screenshots), following the skill's `references/from-website.md`".to_owned()
            }
        },
        DesignFrom::Description => {
            "a description, as a small first system the skill describes, which says it's a starting point".to_owned()
        }
    };
    let keep = match start.keep {
        DesignKeep::Repo => {
            "It's this repository's, so it's part of this thread's changes. Don't make it the repository's \
default in `.silt/design-systems/default` unless I ask."
        }
        DesignKeep::Personal => {
            "It's mine, kept in Silt's data folder and synced to my other devices: write only inside that \
folder, and change nothing in this project."
        }
    };
    let mut out = format!(
        "Make a design system called {title}, with Silt's design-system skill: load it with silt_use_skill before anything \
else, and follow it.\n\nBuild it from {from}.\n\nWrite it in `{dir}`. {keep}\n"
    );
    match (start.from, notes) {
        (Some(DesignFrom::Description), Some(notes)) => {
            let _ = write!(out, "\nThe description:\n\n{notes}\n");
        }
        (_, Some(notes)) => {
            let _ = write!(out, "\nAlso:\n\n{notes}\n");
        }
        _ => {}
    }
    out.push_str(
        "\nShow me the inventory before writing, and ask before anything that would change much. When it's written, tell me \
what you built it from, what you assumed and what's left.\n",
    );
    out
}

/// The design system each thread was started to make or change, by thread.
const DESIGN_THREADS: &str = "design-system-threads.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DesignThread {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) keep: DesignKeep,
    pub(crate) changing: bool,
}

pub(crate) fn design_threads(data_dir: &Path) -> BTreeMap<String, DesignThread> {
    read_capped(&data_dir.join(DESIGN_THREADS), 4 << 20).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

pub(crate) fn record_design_thread(data_dir: &Path, thread: &str, record: DesignThread) -> Result<(), CoreError> {
    let mut all = design_threads(data_dir);
    all.insert(thread.to_owned(), record);
    let json = serde_json::to_vec_pretty(&all).map_err(|error| CoreError::Invalid(error.to_string()))?;
    write_atomically(&data_dir.join(DESIGN_THREADS), &json)
}

/// When each thread first loaded the design-system skill, by thread, for
/// threads asked for a system in their own words rather than from Settings.
const SKILL_THREADS: &str = "design-skill-threads.json";

pub(crate) fn skill_threads(data_dir: &Path) -> BTreeMap<String, i64> {
    read_capped(&data_dir.join(SKILL_THREADS), 4 << 20).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

/// Notes that `thread` loaded the design-system skill, keeping the first
/// time it did, so what it writes from then on shows beside it.
pub fn note_design_skill(data_dir: &Path, thread: &str) -> Result<(), CoreError> {
    let mut all = skill_threads(data_dir);
    if all.contains_key(thread) {
        return Ok(());
    }
    all.insert(thread.to_owned(), crate::core::now_ms());
    let json = serde_json::to_vec_pretty(&all).map_err(|error| CoreError::Invalid(error.to_string()))?;
    fs::create_dir_all(data_dir)?;
    write_atomically(&data_dir.join(SKILL_THREADS), &json)
}

/// When a system's files were first and last written, in ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Written {
    pub(crate) oldest_ms: i64,
    pub(crate) newest_ms: i64,
}

/// [`Written`] for the files in `dir`, looking at a bounded number of them.
pub(crate) fn written(dir: &Path) -> Option<Written> {
    const MOST_FILES: usize = 4096;
    let mut span: Option<Written> = None;
    let mut pending = vec![dir.to_owned()];
    let mut seen = 0;
    while let Some(folder) = pending.pop() {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MOST_FILES {
                return span;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let Some(ms) = meta.modified().ok().and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok()) else {
                continue;
            };
            let ms = i64::try_from(ms.as_millis()).unwrap_or(i64::MAX);
            span = Some(span.map_or(Written { oldest_ms: ms, newest_ms: ms }, |span| Written {
                oldest_ms: span.oldest_ms.min(ms),
                newest_ms: span.newest_ms.max(ms),
            }));
        }
    }
    span
}

/// Of `systems`, the one written last at or after `since_ms`, and whether
/// it held files from before then, so it was changed rather than made.
pub(crate) fn written_since<T>(systems: Vec<(T, Written)>, since_ms: i64) -> Option<(T, bool)> {
    systems
        .into_iter()
        .filter(|(_, written)| written.newest_ms >= since_ms)
        .max_by_key(|(_, written)| written.newest_ms)
        .map(|(system, written)| (system, written.oldest_ms < since_ms))
}

pub(crate) fn read_capped(path: &Path, limit: u64) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() <= limit).then(|| fs::read_to_string(path).ok()).flatten()
}

/// Every system a run in `checkout` may use, a name found first hiding the
/// same name later: the repository's, then the person's.
pub(crate) fn systems(checkout: Option<&Path>, data_dir: &Path) -> Vec<System> {
    let mut found: BTreeMap<SystemName, System> = BTreeMap::new();
    let roots = checkout
        .map(|checkout| (DesignSource::Project, checkout.join(PROJECT_DIR)))
        .into_iter()
        .chain([(DesignSource::Personal, data_dir.join(PERSONAL_DIR))]);
    for (source, root) in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        let mut folders: Vec<PathBuf> =
            entries.flatten().map(|entry| entry.path()).filter(|path| path.join("tokens.json").is_file()).collect();
        folders.sort();
        for dir in folders {
            let Some(name) = dir.file_name().and_then(|name| name.to_str()).and_then(|name| name.parse::<SystemName>().ok())
            else {
                continue;
            };
            found.entry(name.clone()).or_insert(System { name, source, dir });
        }
    }
    found.into_values().collect()
}

/// The system a repository names as its default, in
/// `.silt/design-systems/default`.
pub(crate) fn project_default(checkout: &Path) -> Option<String> {
    let text = read_capped(&checkout.join(PROJECT_DIR).join("default"), 1024)?;
    let name = text.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// The systems the person picked for projects on this machine, by project.
pub(crate) fn project_choices(data_dir: &Path) -> BTreeMap<String, String> {
    read_capped(&data_dir.join(PROJECT_CHOICES), 1 << 20).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

/// Picks `name` for `repo` on this machine, or drops its pick.
pub(crate) fn set_project_choice(data_dir: &Path, repo: &str, name: Option<&str>) -> Result<(), CoreError> {
    let mut choices = project_choices(data_dir);
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) if name == NONE || name.parse::<SystemName>().is_ok() => {
            choices.insert(repo.to_owned(), name.to_owned());
        }
        Some(name) => return Err(CoreError::Invalid(format!("{name:?} isn't a design system's name"))),
        None => {
            choices.remove(repo);
        }
    }
    let json = serde_json::to_vec_pretty(&choices).map_err(|error| CoreError::Invalid(error.to_string()))?;
    write_atomically(&data_dir.join(PROJECT_CHOICES), &json)
}

pub(crate) fn artifact_settings(data_dir: &Path) -> ArtifactSettings {
    read_capped(&data_dir.join(SETTINGS), 64 << 10).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

pub(crate) fn set_artifact_settings(data_dir: &Path, settings: ArtifactSettings) -> Result<(), CoreError> {
    let json = serde_json::to_vec_pretty(&settings).map_err(|error| CoreError::Invalid(error.to_string()))?;
    write_atomically(&data_dir.join(SETTINGS), &json)
}

/// What the person allowed or refused artifacts' pages on this machine
/// (ADR 0072), by `<thread>/<artifact>`, then capability.
const GRANTS: &str = "artifact-grants.json";

type Grants = BTreeMap<String, BTreeMap<String, Grant>>;

/// A grant, with what the version it was given for declared of its
/// capability; one kept before declarations were recorded holds nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum Grant {
    Bound { state: String, declaration: String },
    Bare(String),
}

fn read_grants(data_dir: &Path) -> Grants {
    read_capped(&data_dir.join(GRANTS), 4 << 20).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

/// The grants on an artifact that still hold: each given for what `latest`
/// declares of its capability now, and for some, its page (ADR 0072).
pub(crate) fn artifact_grants(
    data_dir: &Path,
    thread: &str,
    artifact: &str,
    latest: Option<&silt_proto::view::ArtifactView>,
) -> BTreeMap<String, String> {
    let all = read_grants(data_dir);
    let Some(grants) = all.get(&format!("{thread}/{artifact}")) else {
        return BTreeMap::new();
    };
    grants
        .iter()
        .filter_map(|(capability, grant)| match grant {
            Grant::Bound { state, declaration } => (silt_proto::artifact::grant_basis(latest, capability).as_ref()
                == Some(declaration))
            .then(|| (capability.clone(), state.clone())),
            Grant::Bare(_) => None,
        })
        .collect()
}

/// Keeps a grant for what `seen`, the version the viewer saw, says of
/// `capability`, or forgets it with no `state`, so the page asks again.
pub(crate) fn set_artifact_grant(
    data_dir: &Path,
    thread: &str,
    artifact: &str,
    capability: &str,
    state: Option<&str>,
    seen: Option<&silt_proto::view::ArtifactView>,
) -> Result<(), CoreError> {
    if !silt_proto::artifact::CAPABILITIES.contains(&capability) || artifact.parse::<silt_types::ArtifactId>().is_err() {
        return Err(CoreError::Invalid("no such capability or artifact".into()));
    }
    let mut all = read_grants(data_dir);
    let grants = all.entry(format!("{thread}/{artifact}")).or_default();
    match state {
        Some(state @ ("granted" | "refused")) => {
            let Some(declaration) = silt_proto::artifact::grant_basis(seen, capability) else {
                return Err(CoreError::Invalid(format!("artifact {artifact} doesn't declare {capability}")));
            };
            grants.insert(capability.to_owned(), Grant::Bound { state: state.to_owned(), declaration });
        }
        Some(_) => return Err(CoreError::Invalid("a grant is granted or refused".into())),
        None => {
            grants.remove(capability);
        }
    }
    all.retain(|_, grants| !grants.is_empty());
    let json = serde_json::to_vec_pretty(&all).map_err(|error| CoreError::Invalid(error.to_string()))?;
    write_atomically(&data_dir.join(GRANTS), &json)
}

pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    let staging = path.with_extension("tmp");
    fs::write(&staging, bytes)?;
    fs::rename(&staging, path)?;
    Ok(())
}

/// Where the defaults below what's asked for come from, in order.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Defaults<'a> {
    /// The person's pick for the project on this machine.
    pub(crate) picked: Option<&'a str>,
    /// The repository's `.silt/design-systems/default`.
    pub(crate) repository: Option<&'a str>,
    pub(crate) workspace: Option<&'a str>,
}

impl<'a> Defaults<'a> {
    /// The first set, and why it applies.
    pub(crate) fn first(&self) -> Option<(&'a str, &'static str)> {
        [
            (self.picked, "the one picked for this project"),
            (self.repository, "the repository's default"),
            (self.workspace, "the workspace's default"),
        ]
        .into_iter()
        .find_map(|(name, why)| name.map(str::trim).filter(|name| !name.is_empty()).map(|name| (name, why)))
    }
}

/// Which system a new version uses: the one asked for, else the one its
/// last version used, else the defaults; `none` at any step styles nothing.
pub(crate) fn chosen<'a>(asked: Option<&'a str>, previous: Option<&'a str>, defaults: Defaults<'a>) -> Option<&'a str> {
    [asked, previous]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .or_else(|| defaults.first().map(|(name, _)| name))
        .filter(|name| *name != NONE)
}

/// The system named `name`, or why there's none.
pub(crate) fn find(systems: &[System], name: &str) -> Result<System, CoreError> {
    systems.iter().find(|system| system.name.as_str() == name).cloned().ok_or_else(|| {
        let names: Vec<&str> = systems.iter().map(|system| system.name.as_str()).collect();
        CoreError::Invalid(format!("there's no design system named {name}; there are {} and none", names.join(", ")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version declaring `capabilities`, whose page is `page`.
    fn version_with(capabilities: &str, page: &str) -> silt_proto::view::ArtifactView {
        let capabilities: serde_json::Value = serde_json::from_str(capabilities).expect("json");
        serde_json::from_value(serde_json::json!({
            "id": "0123456789ab", "version": 1, "title": "Page", "kind": "html", "media_type": "text/html",
            "blob": page.repeat(32), "size": 1, "capabilities": capabilities
        }))
        .expect("a version")
    }

    #[test]
    fn a_grant_of_samples_holds_for_its_page() {
        let dir = tempfile::tempdir().expect("dir");
        let v1 = version_with(r#"{"sample":{},"comments":{}}"#, "ab");
        let (thread, artifact) = ("t", "0123456789ab");
        set_artifact_grant(dir.path(), thread, artifact, "sample", Some("granted"), Some(&v1)).expect("kept");
        set_artifact_grant(dir.path(), thread, artifact, "comments", Some("granted"), Some(&v1)).expect("kept");
        let v2 = version_with(r#"{"sample":{},"comments":{}}"#, "cd");
        let held = artifact_grants(dir.path(), thread, artifact, Some(&v2));
        assert!(!held.contains_key("sample"), "a new page asks again before it asks an agent");
        assert_eq!(held.get("comments").map(String::as_str), Some("granted"), "the same declaration of comments holds");
    }

    #[test]
    fn a_grant_holds_until_a_version_declares_more() {
        let dir = tempfile::tempdir().expect("dir");
        let declared = |json: &str| version_with(json, "ab");
        let v1 = declared(r#"{"mcp":{"servers":[{"server":"gh","tools":["search"]}]},"sample":{}}"#);
        let (thread, artifact) = ("t", "0123456789ab");
        set_artifact_grant(dir.path(), thread, artifact, "mcp", Some("granted"), Some(&v1)).expect("kept");
        set_artifact_grant(dir.path(), thread, artifact, "sample", Some("granted"), Some(&v1)).expect("kept");
        assert_eq!(artifact_grants(dir.path(), thread, artifact, Some(&v1)).len(), 2);
        let v2 = declared(r#"{"sample":{},"mcp":{"servers":[{"server":"gh","tools":["*"]}]}}"#);
        let held = artifact_grants(dir.path(), thread, artifact, Some(&v2));
        assert_eq!(held.get("sample").map(String::as_str), Some("granted"), "sample declares the same");
        assert!(!held.contains_key("mcp"), "mcp declares more, so it asks again");
        assert!(set_artifact_grant(dir.path(), thread, artifact, "files", Some("granted"), Some(&v1)).is_err(), "undeclared");
        assert!(set_artifact_grant(dir.path(), thread, "../x", "mcp", Some("granted"), Some(&v1)).is_err());
    }

    #[test]
    fn the_one_asked_for_wins_then_the_last_versions_then_the_defaults() {
        let all = Defaults { picked: Some("mine"), repository: Some("acme"), workspace: Some("x") };
        let repo = Defaults { picked: None, repository: Some("acme"), workspace: Some("x") };
        let workspace = Defaults { workspace: Some("x"), ..Defaults::default() };
        assert_eq!(chosen(Some("plain"), Some("nous"), all), Some("plain"));
        assert_eq!(chosen(None, Some("nous"), all), Some("nous"));
        assert_eq!(chosen(None, None, all), Some("mine"));
        assert_eq!(chosen(None, None, repo), Some("acme"));
        assert_eq!(chosen(None, None, workspace), Some("x"));
        assert_eq!(chosen(Some("none"), Some("nous"), all), None);
        assert_eq!(chosen(None, Some("none"), all), None, "a version styled with nothing stays so");
        assert_eq!(chosen(Some(" "), None, Defaults::default()), None);
        assert_eq!(Defaults { picked: Some("none"), ..repo }.first(), Some(("none", "the one picked for this project")));
    }

    #[test]
    fn a_projects_pick_is_kept_and_dropped() {
        let data = tempfile::tempdir().expect("temp dir");
        set_project_choice(data.path(), "github.com/acme/web", Some("nous")).expect("pick");
        set_project_choice(data.path(), "github.com/acme/api", Some("none")).expect("none");
        assert!(set_project_choice(data.path(), "github.com/acme/api", Some("../x")).is_err());
        assert_eq!(project_choices(data.path()).get("github.com/acme/web").map(String::as_str), Some("nous"));
        set_project_choice(data.path(), "github.com/acme/web", None).expect("drop");
        assert_eq!(project_choices(data.path()).len(), 1);
    }

    #[test]
    fn a_repository_hides_the_persons_and_the_built_in_ones() {
        let checkout = tempfile::tempdir().expect("temp dir");
        let data = tempfile::tempdir().expect("temp dir");
        let write = |dir: &Path, name: &str| {
            fs::create_dir_all(dir).expect("dir");
            fs::write(
                dir.join("tokens.json"),
                format!(
                    r##"{{"name":"{name}","color":{{"themes":[{{"id":"light"}}],"tokens":[{{"name":"bg","value":"#fff"}}]}}}}"##
                ),
            )
            .expect("tokens");
            fs::write(dir.join("README.md"), format!("# {name}")).expect("readme");
        };
        write(&checkout.path().join(PROJECT_DIR).join("nous"), "Our Nous");
        write(&data.path().join(PERSONAL_DIR).join("mine"), "Mine");
        write(&data.path().join(PERSONAL_DIR).join("Bad Name"), "Bad");
        fs::write(checkout.path().join(PROJECT_DIR).join("default"), "nous\n").expect("default");

        write(&data.path().join(PERSONAL_DIR).join("nous"), "My Nous");

        let all = systems(Some(checkout.path()), data.path());
        let names: Vec<(&str, DesignSource)> = all.iter().map(|system| (system.name.as_str(), system.source)).collect();
        assert_eq!(names, [("mine", DesignSource::Personal), ("nous", DesignSource::Project)]);
        assert_eq!(find(&all, "nous").expect("nous").view(true).title, "Our Nous");
        assert!(find(&all, "acme").is_err());
        assert!(find(&all, "plain").is_err(), "Silt ships no system of its own");
        assert_eq!(project_default(checkout.path()).as_deref(), Some("nous"));
    }

    #[test]
    fn a_new_systems_folder_is_named_from_its_title() {
        assert_eq!(slug("Harbor").map(|name| name.to_string()).as_deref(), Some("harbor"));
        assert_eq!(slug("  Acme Web 2!  ").map(|name| name.to_string()).as_deref(), Some("acme-web-2"));
        assert_eq!(slug("Café — Brand").map(|name| name.to_string()).as_deref(), Some("caf-brand"));
        assert!(slug("—").is_none());
        assert_eq!(slug(&"a".repeat(80)).map(|name| name.as_str().len()), Some(64));
    }

    fn start(from: Option<DesignFrom>, keep: DesignKeep) -> DesignStart {
        DesignStart {
            title: "Harbor".into(),
            agent: "claude".into(),
            keep,
            clone: None,
            from,
            link: None,
            notes: None,
            attachments: Vec::new(),
            change: None,
            name: None,
        }
    }

    #[test]
    fn the_first_prompt_says_what_to_build_from_and_where_and_leaves_the_how_to_the_skill() {
        let name: SystemName = "harbor".parse().expect("name");
        let code =
            start_prompt(&start(Some(DesignFrom::Code), DesignKeep::Repo), &name, ".silt/design-systems/harbor", Some("web"));
        assert!(
            code.starts_with("Make a design system called Harbor, with Silt's design-system skill: load it with silt_use_skill"),
            "{code}"
        );
        assert!(
            code.contains("Build it from the web project's code, following the skill's `references/from-code.md`."),
            "{code}"
        );
        assert!(code.contains("Write it in `.silt/design-systems/harbor`. It's this repository's"), "{code}");

        let mut site = start(Some(DesignFrom::Website), DesignKeep::Personal);
        site.link = Some("https://acme.example".into());
        site.notes = Some("Only the marketing pages.".into());
        let text = start_prompt(&site, &name, "/home/me/.local/share/silt/design-systems/harbor", None);
        assert!(
            text.contains("the website at https://acme.example, following the skill's `references/from-website.md`"),
            "{text}"
        );
        assert!(
            text.contains("It's mine, kept in Silt's data folder") && text.contains("Also:\n\nOnly the marketing pages."),
            "{text}"
        );

        let mut words = start(Some(DesignFrom::Description), DesignKeep::Personal);
        words.notes = Some("Calm, nautical, one teal accent.".into());
        let text = start_prompt(&words, &name, "/x/harbor", None);
        assert!(
            text.contains("a small first system") && text.contains("The description:\n\nCalm, nautical, one teal accent."),
            "{text}"
        );

        let mut change = start(None, DesignKeep::Repo);
        change.change = Some("Make the accent darker.".into());
        let text = start_prompt(&change, &name, ".silt/design-systems/harbor", None);
        assert!(text.starts_with("Change the design system Harbor (`harbor`), in `.silt/design-systems/harbor`"), "{text}");
        assert!(text.contains("\"Revising and syncing again\"") && text.ends_with("Make the accent darker.\n"), "{text}");
    }

    #[test]
    fn a_systems_page_shows_its_values_and_what_it_left_out() {
        let checkout = tempfile::tempdir().expect("temp dir");
        let dir = checkout.path().join(PROJECT_DIR).join("acme");
        fs::create_dir_all(dir.join(COMPONENTS_DIR)).expect("dir");
        fs::write(
            dir.join("tokens.json"),
            r##"{"name":"Acme","meta":{"source":"code"},"color":{"themes":[{"id":"light","name":"Light"},{"id":"dark","name":"Dark"}],
                "tokens":[{"name":"bg","value":{"light":"#fff","dark":"#111"}},{"name":"fg-1","value":"#222"},{"name":"accent","value":"#d9883a"},
                {"name":"bad","value":"red"}]},"spacing":{"tokens":[{"name":"space-4","value":"16px"}]}}"##,
        )
        .expect("tokens");
        fs::write(dir.join("README.md"), "Use `accent` once.").expect("readme");
        fs::write(dir.join(COMPONENTS_DIR).join("button.html"), "<button>Go</button>").expect("button");
        let all = systems(Some(checkout.path()), checkout.path());
        let acme = find(&all, "acme").expect("acme");
        let summary = acme.summary(vec!["repo's default".into()], None, false);
        assert_eq!((summary.title.as_str(), summary.colors, summary.components), ("Acme", 4, 1));
        assert_eq!(summary.swatches.first().map(String::as_str), Some("#fff"), "the first theme's");
        let page = acme.detail(summary);
        assert_eq!(page.made_from.as_deref(), Some("code"));
        assert_eq!(page.themes.len(), 2);
        assert!(page.tokens.iter().any(|token| token.name == "space-4" && token.family == "spacing"));
        assert!(page.dropped.iter().any(|why| why.contains("bad")), "{:?}", page.dropped);
        assert_eq!(page.files, ["README.md", "components/button.html", "tokens.json"]);
        assert!(page.css.contains("--bg: #fff;"));
    }

    #[test]
    fn a_folders_components_reach_its_frame_and_its_view() {
        let checkout = tempfile::tempdir().expect("temp dir");
        let dir = checkout.path().join(PROJECT_DIR).join("acme");
        fs::create_dir_all(dir.join(COMPONENTS_DIR).join("nested.html")).expect("dir");
        fs::write(dir.join("tokens.json"), r#"{"name":"Acme"}"#).expect("tokens");
        fs::write(dir.join(COMPONENTS_CSS), ".a-button { padding: 4px; }").expect("css");
        fs::write(dir.join(COMPONENTS_DIR).join("button.html"), "<!-- The button. -->\n<button class=\"a-button\">Go</button>")
            .expect("button");
        fs::write(dir.join(COMPONENTS_DIR).join("notes.txt"), "not a component").expect("notes");
        fs::write(dir.join(COMPONENTS_DIR).join("button.md"), "Starts an action.\n").expect("guide");
        fs::write(dir.join(COVER), "<svg class=\"cover\"></svg>").expect("cover");
        fs::create_dir_all(dir.join(ASSETS_DIR).join("Logos")).expect("assets");
        fs::write(dir.join(ASSETS_DIR).join("Logos/mark.svg"), "<svg/>").expect("mark");
        fs::write(dir.join(ASSETS_DIR).join("Logos/run.sh"), "echo").expect("script");
        fs::write(dir.join(ASSETS_DIR).join("loose.png"), "png").expect("loose");
        fs::write(dir.join(COMPONENTS_DIR).join("Big Name.html"), "<b></b>").expect("bad name");

        let all = systems(Some(checkout.path()), checkout.path());
        let acme = find(&all, "acme").expect("acme");
        let css = acme.compiled().expect("css").css;
        assert!(css.ends_with("/* components */\n.a-button { padding: 4px; }\n"), "{css}");
        let view = acme.view(true);
        assert_eq!(
            view.components,
            [DesignComponentView {
                name: "button".into(),
                about: "The button.".into(),
                html: "<!-- The button. -->\n<button class=\"a-button\">Go</button>".into(),
                guide: Some("Starts an action.".into()),
            }]
        );
        assert!(acme.view(false).components.is_empty(), "a listing leaves them out");
        assert_eq!(acme.cover().as_deref(), Some("<svg class=\"cover\"></svg>"));
        let (kept, dropped) = acme.assets();
        assert_eq!(kept, [("Logos/mark.svg".to_owned(), "image/svg+xml", 6)]);
        assert_eq!(dropped.len(), 1, "a script isn't an asset: {dropped:?}");
        assert_eq!(acme.asset("Logos/mark.svg"), Some((b"<svg/>".to_vec(), "image/svg+xml")));
        assert_eq!(acme.asset("Logos/run.sh"), None);
        assert_eq!(acme.asset("../tokens.json"), None);
    }

    /// Silt's own repository names its own Nous, components and all, as its
    /// default, as any repository may (ADR 0087).
    #[test]
    fn silts_own_repository_styles_its_artifacts_with_nous() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let elsewhere = tempfile::tempdir().expect("temp dir");
        let all = systems(Some(&repository), elsewhere.path());
        assert_eq!(project_default(&repository).as_deref(), Some("nous"));
        let nous = find(&all, "nous").expect("nous");
        assert_eq!(nous.source, DesignSource::Project);
        let compiled = nous.compiled().expect("css");
        assert!(compiled.dropped.is_empty(), "{:?}", compiled.dropped);
        assert!(compiled.css.contains("--bg: #f6f1e8;") && compiled.css.contains(".n-button"), "{}", compiled.css);
        assert!(nous.view(true).components.iter().any(|component| component.name == "row"));
    }

    #[test]
    fn a_font_outside_the_systems_own_folder_is_never_read() {
        let checkout = tempfile::tempdir().expect("temp dir");
        let dir = checkout.path().join(PROJECT_DIR).join("acme");
        fs::create_dir_all(dir.join("fonts")).expect("dir");
        fs::write(checkout.path().join("secret.woff2"), b"secret").expect("secret");
        fs::write(dir.join("fonts/acme.woff2"), b"wOF2").expect("font");
        fs::write(
            dir.join("tokens.json"),
            r#"{"name":"Acme","type":{"fonts":[{"family":"Acme","file":"fonts/acme.woff2"},{"family":"Leak","file":"fonts/../../../../secret.woff2"}]}}"#,
        )
        .expect("tokens");
        let all = systems(Some(checkout.path()), checkout.path());
        let css = find(&all, "acme").expect("acme").compiled().expect("css");
        assert!(css.css.contains("d09GMg=="), "{}", css.css);
        assert!(!css.css.contains("c2VjcmV0"), "{}", css.css);
    }

    #[test]
    fn a_thread_shows_the_system_it_wrote_last_since_it_loaded_the_skill() {
        let written = |oldest_ms, newest_ms| Written { oldest_ms, newest_ms };
        let systems = vec![("before", written(10, 90)), ("changed", written(50, 150)), ("made", written(120, 140))];
        assert_eq!(written_since(systems.clone(), 100), Some(("changed", true)));
        assert_eq!(written_since(systems[2..].to_vec(), 100), Some(("made", false)));
        assert_eq!(written_since(systems[..1].to_vec(), 100), None, "nothing written since");
    }

    #[test]
    fn a_thread_keeps_when_it_first_loaded_the_skill() {
        let data = tempfile::tempdir().expect("tempdir");
        note_design_skill(data.path(), "t1").expect("noted");
        let first = skill_threads(data.path())["t1"];
        std::thread::sleep(std::time::Duration::from_millis(5));
        note_design_skill(data.path(), "t1").expect("again");
        note_design_skill(data.path(), "t2").expect("another");
        let all = skill_threads(data.path());
        assert_eq!((all["t1"], all.len()), (first, 2));
    }
}
