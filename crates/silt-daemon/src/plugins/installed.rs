//! The plugins installed on this machine (ADR 0103): what each is, where it
//! came from, and what the person decided about it. A plugin's files live in
//! `plugins/<id>/` in Silt's data: its manifest, its component, its README,
//! its own data folder, and a folder per thread for what it keeps for that
//! thread, such as GitHub's mirrors.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use silt_plugin::{Manifest, ScopedRule};

use crate::CoreError;

const RECORDS: &str = "plugins/installed.json";
const TRUSTED: &str = "plugins/trusted.json";

/// Where an installed plugin came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum PluginSource {
    /// Built into Silt, and updated with it.
    Shipped,
    /// A git repository, pinned to `commit`, following `follow` (a tag
    /// pattern or a branch) only if the person chose to.
    Git {
        url: String,
        commit: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        follow: Option<String>,
    },
    /// A folder being developed, rebuilt when it changes.
    Dev { path: String },
    /// A marketplace, served by a Silt server (ADR 0104).
    Marketplace { server: String },
    /// An org's policy, which installs and locks it in its workspaces, from
    /// git at `commit`.
    Org {
        org: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        commit: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Installed {
    pub id: String,
    pub source: PluginSource,
    pub version: String,
    /// The capabilities the person granted, by name.
    #[serde(default)]
    pub granted: BTreeSet<String>,
    /// The workspaces it's on in, by id.
    #[serde(default)]
    pub workspaces: Vec<String>,
    /// Its environments, each with the patterns that pick it.
    #[serde(default)]
    pub environments: BTreeMap<String, Vec<String>>,
    /// Which of `silt credential`'s credentials each of its own is: by the
    /// credential's id, or `id@environment` for one per environment.
    #[serde(default)]
    pub credentials: BTreeMap<String, String>,
    /// The person's values of its settings; never secrets.
    #[serde(default)]
    #[cfg_attr(test, ts(type = "Record<string, unknown>"))]
    pub settings: BTreeMap<String, Value>,
    /// The person's rules for its actions.
    #[serde(default)]
    pub rules: Vec<ScopedRule>,
    /// What the person agreed to for this version, in the words they saw:
    /// each capability with its hosts, credential and action.
    #[serde(default)]
    pub consented: Vec<String>,
}

impl Installed {
    /// Whether it's on in `workspace`; `*` is every workspace.
    pub fn on_in(&self, workspace: &str) -> bool {
        self.workspaces.iter().any(|on| on == workspace || on == "*")
    }

    /// The `silt credential` holding its credential `id` for `environment`.
    pub fn credential_for(&self, id: &str, environment: Option<&str>) -> Option<&str> {
        environment
            .and_then(|environment| self.credentials.get(&format!("{id}@{environment}")))
            .or_else(|| self.credentials.get(id))
            .map(String::as_str)
    }

    /// Its environments, with their patterns as globs.
    pub fn environment_globs(&self) -> Vec<(String, Vec<silt_plugin::Glob>)> {
        self.environments
            .iter()
            .map(|(name, patterns)| (name.clone(), patterns.iter().map(silt_plugin::Glob::new).collect()))
            .collect()
    }
}

pub fn load(data_dir: &Path) -> Vec<Installed> {
    std::fs::read(data_dir.join(RECORDS)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

pub fn save(data_dir: &Path, all: &[Installed]) -> Result<(), CoreError> {
    let bytes = serde_json::to_vec_pretty(all).map_err(|error| CoreError::Invalid(error.to_string()))?;
    let path = data_dir.join(RECORDS);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    silt_os::write_private(&path, &bytes)?;
    Ok(())
}

pub fn find(data_dir: &Path, id: &str) -> Option<Installed> {
    load(data_dir).into_iter().find(|installed| installed.id == id)
}

/// Changes the record of `id` with `change`, and keeps it.
pub fn update(
    data_dir: &Path,
    id: &str,
    change: impl FnOnce(&mut Installed) -> Result<(), CoreError>,
) -> Result<Installed, CoreError> {
    let mut all = load(data_dir);
    let installed =
        all.iter_mut().find(|installed| installed.id == id).ok_or_else(|| CoreError::Invalid(format!("no plugin {id}")))?;
    change(installed)?;
    let updated = installed.clone();
    save(data_dir, &all)?;
    Ok(updated)
}

/// Turns plugin `id` on or off in `workspace`. One on everywhere, turned
/// off somewhere, is on in each of `every` but that one. An org's stays
/// on in its workspaces.
pub fn turn(data_dir: &Path, id: &str, workspace: &str, on: bool, every: &[String]) -> Result<(), CoreError> {
    if let Some(PluginSource::Org { org, .. }) = find(data_dir, id).map(|record| record.source)
        && !on
    {
        return Err(CoreError::Invalid(format!("{org}'s policy keeps {id} on in its workspaces")));
    }
    update(data_dir, id, |record| {
        if !on && record.workspaces.iter().any(|other| other == "*") {
            record.workspaces = every.to_vec();
        }
        record.workspaces.retain(|other| other != workspace);
        if on {
            record.workspaces.push(workspace.to_owned());
            record.workspaces.sort();
        }
        Ok(())
    })
    .map(|_| ())
}

/// A plugin's own folder: its manifest, component, README and data.
pub fn dir(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join("plugins").join(id)
}

/// What a plugin keeps for one thread; `agent/` in it is shown to that
/// thread's agents, read-only, where the plugin may.
pub fn thread_dir(data_dir: &Path, id: &str, thread: &str) -> PathBuf {
    dir(data_dir, id).join("threads").join(thread)
}

pub fn manifest(data_dir: &Path, id: &str) -> Result<Manifest, CoreError> {
    let text = std::fs::read_to_string(dir(data_dir, id).join("silt-plugin.toml"))
        .map_err(|error| CoreError::Invalid(format!("{id}'s manifest can't be read: {error}")))?;
    Manifest::parse(&text).map_err(|error| CoreError::Invalid(format!("{id}: {error}")))
}

/// When `id`'s component was last put in place, so a dev folder's rebuild
/// at the same version is loaded again.
pub fn built(data_dir: &Path, id: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(dir(data_dir, id).join("plugin.wasm")).and_then(|metadata| metadata.modified()).ok()
}

pub fn component(data_dir: &Path, id: &str) -> Result<Vec<u8>, CoreError> {
    std::fs::read(dir(data_dir, id).join("plugin.wasm"))
        .map_err(|error| CoreError::Invalid(format!("{id}'s component can't be read: {error}")))
}

pub fn readme(data_dir: &Path, id: &str) -> Option<String> {
    std::fs::read_to_string(dir(data_dir, id).join("README.md")).ok()
}

/// Puts a plugin's files in place, replacing those of an older version.
pub fn write_files(data_dir: &Path, manifest_text: &str, wasm: &[u8], readme: Option<&str>) -> Result<Manifest, CoreError> {
    let manifest = Manifest::parse(manifest_text).map_err(|error| CoreError::Invalid(error.to_string()))?;
    let dir = dir(data_dir, &manifest.id);
    std::fs::create_dir_all(dir.join("data"))?;
    std::fs::write(dir.join("silt-plugin.toml"), manifest_text)?;
    std::fs::write(dir.join("plugin.wasm"), wasm)?;
    match readme {
        Some(text) => std::fs::write(dir.join("README.md"), text)?,
        None => {
            let _ = std::fs::remove_file(dir.join("README.md"));
        }
    }
    Ok(manifest)
}

/// A plugin as the web UI shows it: what's installed, its manifest, its
/// README, whether its publisher is trusted, and what adding it agreed to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PluginView {
    pub installed: Installed,
    /// `None` when its files can't be read here.
    pub manifest: Option<Manifest>,
    pub readme: Option<String>,
    pub trusted: bool,
    /// Its hosts, those its settings name included.
    pub hosts: Vec<String>,
}

/// Every plugin installed here, as the web UI shows them.
pub fn views(data_dir: &Path) -> Vec<PluginView> {
    let trusted = trusted(data_dir);
    load(data_dir)
        .into_iter()
        .map(|installed| {
            let manifest = manifest(data_dir, &installed.id).ok();
            let mut hosts: Vec<String> =
                manifest.as_ref().map(|manifest| manifest.hosts().into_iter().map(str::to_owned).collect()).unwrap_or_default();
            for capability in manifest.iter().flat_map(|manifest| &manifest.capabilities) {
                let Some(key) = capability.hosts_from() else { continue };
                for (stored, value) in &installed.settings {
                    if stored == key || stored.starts_with(&format!("{key}@")) || stored.starts_with(&format!("{key}#")) {
                        hosts.extend(silt_plugin::hosts_in(value));
                    }
                }
            }
            hosts.sort();
            hosts.dedup();
            PluginView {
                readme: readme(data_dir, &installed.id),
                trusted: manifest.as_ref().is_some_and(|manifest| trusted.contains(&manifest.publisher)),
                manifest,
                hosts,
                installed,
            }
        })
        .collect()
}

/// Sets the person's rule for plugin `id`'s `action`, for calls whose
/// `field` matches `pattern` when given; `None` takes theirs away.
pub fn set_rule(
    data_dir: &Path,
    id: &str,
    action: &str,
    rule: Option<silt_plugin::Rule>,
    matching: Option<(String, String)>,
) -> Result<(), CoreError> {
    let manifest = manifest(data_dir, id)?;
    let spec = manifest.action(action).ok_or_else(|| CoreError::Invalid(format!("{id} has no action {action}")))?;
    let (field, pattern) = match matching {
        Some((field, pattern)) => {
            if !spec.fields.contains(&field) {
                return Err(CoreError::Invalid(format!("{action}'s rules can match {}, not {field}", spec.fields.join(", "))));
            }
            (Some(field), Some(silt_plugin::Glob::new(pattern)))
        }
        None => (None, None),
    };
    update(data_dir, id, |record| {
        record.rules.retain(|existing| {
            !(existing.action == action
                && existing.field == field
                && existing.pattern == pattern
                && existing.source == silt_plugin::RuleSource::Yours)
        });
        if let Some(rule) = rule {
            record.rules.push(ScopedRule {
                action: action.to_owned(),
                field,
                pattern,
                rule,
                once: false,
                source: silt_plugin::RuleSource::Yours,
            });
        }
        Ok(())
    })
    .map(|_| ())
}

/// Sets plugin `id`'s setting `key`, checked against its manifest, for an
/// environment or workspace where it's set per one; `null` resets it.
pub fn set_setting(
    data_dir: &Path,
    id: &str,
    key: &str,
    value: &Value,
    environment: Option<&str>,
    workspace: Option<&str>,
) -> Result<(), CoreError> {
    let manifest = manifest(data_dir, id)?;
    let spec = manifest
        .settings
        .iter()
        .find(|spec| spec.key == key)
        .ok_or_else(|| CoreError::Invalid(format!("{id} has no setting {key}")))?;
    if spec.kind == silt_plugin::SettingKind::Secret {
        return Err(CoreError::Invalid(format!("{key} is a secret: keep it with `silt plugin secret`")));
    }
    let stored = silt_plugin::setting_key(spec, environment, workspace).map_err(CoreError::Invalid)?;
    if !value.is_null() {
        silt_plugin::fits(spec, value).map_err(|why| CoreError::Invalid(format!("{key} {why}")))?;
    }
    update(data_dir, id, |record| {
        if value.is_null() {
            record.settings.remove(&stored);
        } else {
            record.settings.insert(stored.clone(), value.clone());
        }
        Ok(())
    })
    .map(|_| ())
}

/// Sets the patterns that pick plugin `id`'s environment `name`; none
/// removes it.
pub fn set_environment(data_dir: &Path, id: &str, name: &str, patterns: &[String]) -> Result<(), CoreError> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)) {
        return Err(CoreError::Invalid(format!("{name} isn't an environment's name: letters, digits, `-` and `_`")));
    }
    update(data_dir, id, |record| {
        let patterns: Vec<String> =
            patterns.iter().map(|pattern| pattern.trim().to_owned()).filter(|pattern| !pattern.is_empty()).collect();
        if patterns.is_empty() {
            record.environments.remove(name);
        } else {
            record.environments.insert(name.to_owned(), patterns);
        }
        Ok(())
    })
    .map(|_| ())
}

/// Puts the plugins shipped with this Silt in place: new ones installed,
/// on everywhere if they're meant to be; those of another version replaced,
/// keeping what the person chose for them.
pub fn ensure_shipped(data_dir: &Path) -> Result<(), CoreError> {
    let mut all = load(data_dir);
    let mut changed = false;
    for shipped in silt_shipped::PLUGINS.iter().filter(|shipped| !shipped.wasm.is_empty()) {
        let manifest = Manifest::parse(shipped.manifest).map_err(|error| CoreError::Invalid(error.to_string()))?;
        let dir = dir(data_dir, &manifest.id);
        let current = std::fs::read(dir.join("plugin.wasm")).is_ok_and(|wasm| wasm == shipped.wasm)
            && std::fs::read_to_string(dir.join("silt-plugin.toml")).is_ok_and(|text| text == shipped.manifest);
        if !current {
            write_files(data_dir, shipped.manifest, shipped.wasm, Some(shipped.readme))?;
        }
        let declared: BTreeSet<String> = manifest.capabilities.iter().map(|capability| capability.name().to_owned()).collect();
        match all.iter_mut().find(|installed| installed.id == manifest.id) {
            Some(installed) => {
                if installed.version != manifest.version
                    || installed.granted != declared
                    || installed.source != PluginSource::Shipped
                {
                    installed.version = manifest.version.clone();
                    installed.granted = declared;
                    installed.source = PluginSource::Shipped;
                    changed = true;
                }
            }
            None => {
                all.push(Installed {
                    id: manifest.id.clone(),
                    source: PluginSource::Shipped,
                    version: manifest.version.clone(),
                    granted: declared,
                    workspaces: if shipped.everywhere { vec!["*".to_owned()] } else { Vec::new() },
                    environments: BTreeMap::new(),
                    credentials: BTreeMap::new(),
                    settings: BTreeMap::new(),
                    rules: Vec::new(),
                    consented: Vec::new(),
                });
                changed = true;
            }
        }
    }
    if changed {
        all.sort_by(|one, other| one.id.cmp(&other.id));
        save(data_dir, &all)?;
    }
    Ok(())
}

/// The publishers whose plugins' defaults the person accepts. Silt is
/// always among them, as its plugins ship with it.
pub fn trusted(data_dir: &Path) -> BTreeSet<String> {
    let mut trusted: BTreeSet<String> =
        std::fs::read(data_dir.join(TRUSTED)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    if !trusted.contains("-silt") {
        trusted.insert("silt".into());
    }
    trusted.retain(|publisher| !publisher.starts_with('-'));
    trusted
}

/// Trusts `publisher`, or stops trusting it. Withdrawing Silt's trust is
/// kept as `-silt`.
pub fn set_trusted(data_dir: &Path, publisher: &str, trust: bool) -> Result<(), CoreError> {
    let path = data_dir.join(TRUSTED);
    let mut kept: BTreeSet<String> =
        std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    if publisher == "silt" {
        if trust {
            kept.remove("-silt");
        } else {
            kept.insert("-silt".into());
        }
    } else if trust {
        kept.insert(publisher.to_owned());
    } else {
        kept.remove(publisher);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(&kept).unwrap_or_default())?;
    Ok(())
}

/// The agent-facing tool name of a plugin's tool: the plugin's name part,
/// then the tool's, so `silt.github`'s `push` is `github_push`.
pub fn tool_name(id: &str, tool: &str) -> String {
    let short = id.split_once('.').map_or(id, |(_, name)| name).replace('-', "_");
    format!("{short}_{tool}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_and_credentials_resolve_by_environment() {
        let dir = tempfile::tempdir().unwrap();
        let installed = Installed {
            id: "silt.github".into(),
            source: PluginSource::Shipped,
            version: "1.0.0".into(),
            granted: BTreeSet::from(["http".into()]),
            workspaces: vec!["work".into()],
            environments: BTreeMap::from([("acme".into(), vec!["acme/*".into()])]),
            credentials: BTreeMap::from([("token".into(), "mine".into()), ("token@acme".into(), "work-github".into())]),
            settings: BTreeMap::new(),
            rules: Vec::new(),
            consented: Vec::new(),
        };
        save(dir.path(), std::slice::from_ref(&installed)).unwrap();
        assert_eq!(find(dir.path(), "silt.github"), Some(installed.clone()));
        assert_eq!(installed.credential_for("token", Some("acme")), Some("work-github"));
        assert_eq!(installed.credential_for("token", Some("personal")), Some("mine"));
        assert!(installed.on_in("work") && !installed.on_in("home"));
        assert_eq!(tool_name("silt.github", "push"), "github_push");
        assert_eq!(tool_name("acme.silt-deploys", "rollout"), "silt_deploys_rollout");
    }

    #[test]
    fn silt_is_trusted_until_the_person_says_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        assert!(trusted(dir.path()).contains("silt"));
        set_trusted(dir.path(), "tidewater", true).unwrap();
        set_trusted(dir.path(), "silt", false).unwrap();
        let now = trusted(dir.path());
        assert!(now.contains("tidewater") && !now.contains("silt"));
        set_trusted(dir.path(), "silt", true).unwrap();
        assert!(trusted(dir.path()).contains("silt"));
    }
}
