//! The design systems an artifact is styled with (ADR 0070): a
//! repository's, the person's, or one built in, and the one that applies to
//! a run's new version.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use silt_design::{Builtin, SystemName};

use crate::core::CoreError;
use crate::protocol::{DesignSource, DesignSystemView};

/// A repository's systems, under its checkout.
pub(crate) const PROJECT_DIR: &str = ".silt/design-systems";
/// The person's own, under the data dir.
pub(crate) const PERSONAL_DIR: &str = "design-systems";
/// What `design_system` takes to style nothing.
pub(crate) const NONE: &str = "none";
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

fn read_capped(path: &Path, limit: u64) -> Option<String> {
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

/// Which system a new version uses: the one asked for, else the one its
/// last version used, else the repository's default, else the
/// workspace's; `none` at any step styles nothing.
pub(crate) fn chosen<'a>(
    asked: Option<&'a str>,
    previous: Option<&'a str>,
    project: Option<&'a str>,
    workspace: Option<&'a str>,
) -> Option<&'a str> {
    [asked, previous, project, workspace]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|name| !name.is_empty())
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
        assert_eq!(chosen(Some("plain"), Some("nous"), Some("acme"), Some("x")), Some("plain"));
        assert_eq!(chosen(None, Some("nous"), Some("acme"), None), Some("nous"));
        assert_eq!(chosen(None, None, Some("acme"), Some("x")), Some("acme"));
        assert_eq!(chosen(None, None, None, Some("x")), Some("x"));
        assert_eq!(chosen(Some("none"), Some("nous"), Some("acme"), None), None);
        assert_eq!(chosen(None, Some("none"), Some("acme"), None), None, "a version styled with nothing stays so");
        assert_eq!(chosen(Some(" "), None, None, None), None);
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
