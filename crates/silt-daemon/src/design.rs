//! The design systems an artifact is styled with (ADR 0071): a
//! repository's, the person's, or one built in, and the one that applies to
//! a run's new version.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use silt_design::{Builtin, SystemName};

use crate::core::CoreError;
use crate::protocol::{ArtifactSettings, DesignSource, DesignSystemView};

/// A repository's systems, under its checkout.
pub(crate) const PROJECT_DIR: &str = ".silt/design-systems";
/// The person's own, under the data dir.
pub(crate) const PERSONAL_DIR: &str = "design-systems";
/// What `design_system` takes to style nothing.
pub(crate) const NONE: &str = "none";
/// The system the person picked for each project on this machine.
const PROJECT_CHOICES: &str = "project-design-systems.json";
/// How this machine's frames load libraries and fonts.
const SETTINGS: &str = "artifacts.json";
const README_BYTES: u64 = 256 << 10;
const TOKENS_BYTES: u64 = 512 << 10;

#[derive(Debug, Clone)]
pub(crate) struct System {
    pub(crate) name: SystemName,
    pub(crate) source: DesignSource,
    place: Place,
}

#[derive(Debug, Clone)]
enum Place {
    Folder(PathBuf),
    Builtin(Builtin),
}

impl System {
    pub(crate) fn readme(&self) -> String {
        match &self.place {
            Place::Builtin(builtin) => builtin.readme.to_owned(),
            Place::Folder(dir) => read_capped(&dir.join("README.md"), README_BYTES).unwrap_or_default(),
        }
    }

    pub(crate) fn tokens(&self) -> Result<silt_design::Tokens, CoreError> {
        let json = match &self.place {
            Place::Builtin(builtin) => builtin.tokens.to_owned(),
            Place::Folder(dir) => read_capped(&dir.join("tokens.json"), TOKENS_BYTES)
                .ok_or_else(|| CoreError::Invalid(format!("the design system {} has no readable tokens.json", self.name)))?,
        };
        silt_design::parse(&json).map_err(|error| CoreError::Invalid(format!("the design system {}: {error}", self.name)))
    }

    /// The font files its tokens name, each under its own `fonts/`.
    fn fonts(&self, tokens: &silt_design::Tokens) -> BTreeMap<String, Vec<u8>> {
        let Place::Folder(dir) = &self.place else {
            return BTreeMap::new();
        };
        let Ok(root) = silt_os::canonicalize(&dir.join("fonts")) else {
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

    /// The CSS its tokens compile to, for a frame.
    pub(crate) fn compiled(&self) -> Result<silt_design::Compiled, CoreError> {
        let tokens = self.tokens()?;
        let fonts = self.fonts(&tokens);
        Ok(silt_design::compile(&tokens, &fonts))
    }

    pub(crate) fn view(&self, with_readme: bool) -> DesignSystemView {
        let tokens = self.tokens().ok();
        DesignSystemView {
            name: self.name.to_string(),
            title: tokens.as_ref().map_or_else(|| self.name.to_string(), |tokens| tokens.name.clone()),
            source: self.source,
            readme: with_readme.then(|| self.readme()),
            tokens: if with_readme { tokens.as_ref().map(silt_design::token_names).unwrap_or_default() } else { BTreeMap::new() },
        }
    }
}

pub(crate) fn read_capped(path: &Path, limit: u64) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() <= limit).then(|| fs::read_to_string(path).ok()).flatten()
}

/// Every system a run in `checkout` may use, a name found first hiding the
/// same name later: the repository's, the person's, then the built-in ones.
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
            found.entry(name.clone()).or_insert(System { name, source, place: Place::Folder(dir) });
        }
    }
    for builtin in silt_design::BUILTIN {
        if let Ok(name) = builtin.name.parse::<SystemName>() {
            found.entry(name.clone()).or_insert(System { name, source: DesignSource::Builtin, place: Place::Builtin(builtin) });
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

type Grants = BTreeMap<String, BTreeMap<String, String>>;

pub(crate) fn artifact_grants(data_dir: &Path, thread: &str, artifact: &str) -> BTreeMap<String, String> {
    let all: Grants =
        read_capped(&data_dir.join(GRANTS), 4 << 20).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
    all.get(&format!("{thread}/{artifact}")).cloned().unwrap_or_default()
}

/// Keeps a grant, or forgets it with no `state`, so the page asks again.
pub(crate) fn set_artifact_grant(
    data_dir: &Path,
    thread: &str,
    artifact: &str,
    capability: &str,
    state: Option<&str>,
) -> Result<(), CoreError> {
    if !silt_proto::artifact::CAPABILITIES.contains(&capability) || artifact.len() != 12 {
        return Err(CoreError::Invalid("no such capability or artifact".into()));
    }
    let mut all: Grants =
        read_capped(&data_dir.join(GRANTS), 4 << 20).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
    let grants = all.entry(format!("{thread}/{artifact}")).or_default();
    match state {
        Some(state @ ("granted" | "refused")) => {
            grants.insert(capability.to_owned(), state.to_owned());
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

        let all = systems(Some(checkout.path()), data.path());
        let names: Vec<(&str, DesignSource)> = all.iter().map(|system| (system.name.as_str(), system.source)).collect();
        assert_eq!(names, [("mine", DesignSource::Personal), ("nous", DesignSource::Project), ("plain", DesignSource::Builtin)]);
        assert_eq!(find(&all, "nous").expect("nous").view(true).title, "Our Nous");
        assert!(find(&all, "acme").is_err());
        assert_eq!(project_default(checkout.path()).as_deref(), Some("nous"));
        assert!(find(&all, "plain").expect("plain").compiled().expect("css").css.contains("--chart-1:"));
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
}
