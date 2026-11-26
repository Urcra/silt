//! Updates waiting for plugins added from git (ADR 0104). One added at a
//! branch follows its head, one added at a version tag is offered the
//! highest version tagged since, and one pinned to a commit stays there.
//! Silt looks with `git ls-remote`, reads a newer commit's manifest without
//! building it, and says what it would add; nothing updates until the
//! person says so.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use silt_plugin::Manifest;
use tracing::warn;

use super::install;
use super::installed::{self, Installed, PluginSource};
use crate::core::CoreError;

const FILE: &str = "updates.json";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

fn invalid(why: impl Into<String>) -> CoreError {
    CoreError::Invalid(why.into())
}

/// A newer version of a plugin added from git.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct GitUpdate {
    pub id: String,
    /// The commit installed when it was found.
    pub from_commit: String,
    pub from: String,
    pub to: String,
    /// The tag or branch it's at.
    pub reference: String,
    pub commit: String,
    /// What came in since, newest first, as `<short id> <subject>`.
    pub commits: Vec<String>,
    /// What it asks for that the installed version doesn't: updating it
    /// needs agreeing to these.
    pub adds: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Kept {
    checked_ms: i64,
    updates: Vec<GitUpdate>,
}

fn file(data_dir: &Path) -> PathBuf {
    data_dir.join("plugins").join(FILE)
}

fn kept(data_dir: &Path) -> Kept {
    std::fs::read(file(data_dir)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn keep(data_dir: &Path, kept: &Kept) {
    if let Ok(bytes) = serde_json::to_vec_pretty(kept) {
        let _ = std::fs::write(file(data_dir), bytes);
    }
}

/// `v1.2.3` or `1.2.3` as its numbers; a pre-release or anything else isn't
/// a version a plugin is offered.
fn version_of(tag: &str) -> Option<Vec<u64>> {
    let numbers = tag.strip_prefix('v').unwrap_or(tag);
    numbers.split('.').map(|part| part.parse::<u64>().ok()).collect::<Option<Vec<u64>>>().filter(|parts| !parts.is_empty())
}

/// The newest of `refs` (as `git ls-remote` lists them) that a plugin
/// following `follow` is offered: its branch's head, or the highest version
/// tag above it.
pub fn newest(refs: &[(String, String)], follow: &str) -> Option<(String, String)> {
    if let Some((_, commit)) = refs.iter().find(|(name, _)| *name == format!("refs/heads/{follow}")) {
        return Some((follow.to_owned(), commit.clone()));
    }
    let current = version_of(follow)?;
    let mut best: Option<(Vec<u64>, String, String)> = None;
    for (name, commit) in refs {
        let Some(tag) = name.strip_prefix("refs/tags/") else { continue };
        let (tag, peeled) = match tag.strip_suffix("^{}") {
            Some(tag) => (tag, true),
            None => (tag, false),
        };
        let Some(version) = version_of(tag) else { continue };
        if version <= current {
            continue;
        }
        match &mut best {
            // An annotated tag's peeled line names the commit itself.
            Some((kept, kept_tag, kept_commit)) if *kept == version && kept_tag == tag => {
                if peeled {
                    *kept_commit = commit.clone();
                }
            }
            Some((kept, ..)) if *kept >= version => {}
            _ => best = Some((version, tag.to_owned(), commit.clone())),
        }
    }
    best.map(|(_, tag, commit)| (tag, commit))
}

fn ls_remote(url: &str) -> Result<Vec<(String, String)>, CoreError> {
    if url.starts_with('-') {
        return Err(invalid("a repository can't start with `-`"));
    }
    let output =
        Command::new("git").args(["ls-remote", "--tags", "--heads", "--", url]).env("GIT_TERMINAL_PROMPT", "0").output()?;
    if !output.status.success() {
        return Err(invalid(format!("{url}: {}", String::from_utf8_lossy(&output.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(commit, name)| (name.to_owned(), commit.to_owned()))
        .collect())
}

fn scratch(data_dir: &Path) -> PathBuf {
    data_dir.join("plugins").join(".checkouts").join(uuid::Uuid::new_v4().simple().to_string())
}

/// The update waiting for `plugin`, if it was added from git and its
/// repository has a newer version; one already found is reused.
pub fn find(data_dir: &Path, plugin: &Installed, known: &[GitUpdate]) -> Result<Option<GitUpdate>, CoreError> {
    let PluginSource::Git { url, commit, follow: Some(follow) } = &plugin.source else { return Ok(None) };
    let Some((reference, newer)) = newest(&ls_remote(url)?, follow) else { return Ok(None) };
    if newer == *commit {
        return Ok(None);
    }
    if let Some(found) = known.iter().find(|known| known.id == plugin.id && known.commit == newer && known.from_commit == *commit)
    {
        return Ok(Some(found.clone()));
    }
    let into = scratch(data_dir);
    let read = (|| {
        let at = install::clone(url, Some(&reference), &into)?;
        let text = std::fs::read_to_string(into.join("silt-plugin.toml"))
            .map_err(|_| invalid(format!("{url} {reference} has no silt-plugin.toml")))?;
        let manifest = Manifest::parse(&text).map_err(|error| invalid(format!("{url} {reference}: {error}")))?;
        if manifest.id != plugin.id {
            return Err(invalid(format!("{url} {reference} holds {}, not {}", manifest.id, plugin.id)));
        }
        let adds = installed::manifest(data_dir, &plugin.id).map(|now| now.added_in(&manifest)).unwrap_or_default();
        Ok(GitUpdate {
            id: plugin.id.clone(),
            from_commit: commit.clone(),
            from: plugin.version.clone(),
            to: manifest.version,
            commits: install::commits_between(&into, commit, &at).unwrap_or_default(),
            reference: reference.clone(),
            commit: at,
            adds,
        })
    })();
    let _ = std::fs::remove_dir_all(&into);
    read.map(Some)
}

/// Looks for updates of every plugin added from git, and keeps what it
/// found for the web UI.
pub fn check(data_dir: &Path) -> Vec<GitUpdate> {
    let known = kept(data_dir).updates;
    let mut updates = Vec::new();
    for plugin in installed::load(data_dir) {
        match find(data_dir, &plugin, &known) {
            Ok(Some(update)) => updates.push(update),
            Ok(None) => {}
            Err(error) => warn!(plugin = %plugin.id, %error, "could not look for an update"),
        }
    }
    keep(data_dir, &Kept { checked_ms: crate::core::now_ms(), updates: updates.clone() });
    updates
}

/// The updates last found, still waiting: each for the commit installed now.
pub fn waiting(data_dir: &Path) -> Vec<GitUpdate> {
    let plugins = installed::load(data_dir);
    kept(data_dir)
        .updates
        .into_iter()
        .filter(|update| {
            plugins.iter().any(|plugin| {
                plugin.id == update.id
                    && matches!(&plugin.source, PluginSource::Git { commit, .. } if *commit == update.from_commit)
            })
        })
        .collect()
}

/// Keeps `update` as waiting, as a check would have.
pub fn remember(data_dir: &Path, update: &GitUpdate) {
    let mut kept = kept(data_dir);
    kept.updates.retain(|known| known.id != update.id);
    kept.updates.push(update.clone());
    keep(data_dir, &kept);
}

/// Starts a check in the background when none ran for a day.
pub fn check_if_due(data_dir: &Path) {
    static CHECKING: AtomicBool = AtomicBool::new(false);
    if !crate::network::allowed(data_dir).update_checks {
        return;
    }
    let due = crate::core::now_ms().saturating_sub(kept(data_dir).checked_ms) >= CHECK_EVERY.as_millis() as i64;
    let any = installed::load(data_dir).iter().any(|plugin| matches!(plugin.source, PluginSource::Git { follow: Some(_), .. }));
    if !due || !any || CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let data_dir = data_dir.to_owned();
    tokio::task::spawn_blocking(move || {
        check(&data_dir);
        CHECKING.store(false, Ordering::SeqCst);
    });
}

/// Updates plugin `id` to the update waiting for it, built offline in a
/// sandbox; what it adds needs `consented`. The commit is the one shown, or
/// nothing changes.
pub async fn apply(data_dir: &Path, id: &str, consented: bool) -> Result<Installed, CoreError> {
    let update = waiting(data_dir)
        .into_iter()
        .find(|update| update.id == id)
        .ok_or_else(|| invalid(format!("no update is waiting for {id}; look again")))?;
    let Some(Installed { source: PluginSource::Git { url, .. }, .. }) = installed::find(data_dir, id) else {
        return Err(invalid(format!("{id} wasn't added from git")));
    };
    if !update.adds.is_empty() && !consented {
        return Err(invalid(format!("{id} {} asks for more: {}. Agree to it to update", update.to, update.adds.join("; "))));
    }
    let into = scratch(data_dir);
    let (owned_url, reference, checkout) = (url.clone(), update.reference.clone(), into.clone());
    let updated = async {
        let at = tokio::task::spawn_blocking(move || install::clone(&owned_url, Some(&reference), &checkout))
            .await
            .map_err(|error| invalid(error.to_string()))??;
        if at != update.commit {
            return Err(invalid(format!("{url} {} moved since you looked; look again", update.reference)));
        }
        let package = install::package(&into, false).await?;
        let source = PluginSource::Git { url: url.clone(), commit: at, follow: Some(update.reference.clone()) };
        install::install(data_dir, &package, source, &[], true)
    }
    .await;
    let _ = std::fs::remove_dir_all(&into);
    if updated.is_ok() {
        let mut kept = kept(data_dir);
        kept.updates.retain(|known| known.id != id);
        keep(data_dir, &kept);
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(lines: &[(&str, &str)]) -> Vec<(String, String)> {
        lines.iter().map(|(name, commit)| ((*name).to_owned(), (*commit).to_owned())).collect()
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Ada", "-c", "user.email=ada@example.com", "-c", "init.defaultBranch=main"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    #[test]
    fn a_plugin_at_a_tag_is_offered_the_next_version_and_what_it_adds() {
        let repo = tempfile::tempdir().unwrap();
        let manifest = silt_plugin_host::testing::PROBE_MANIFEST;
        git(repo.path(), &["init", "-q"]);
        std::fs::write(repo.path().join("silt-plugin.toml"), manifest).unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "First"]);
        git(repo.path(), &["tag", "v1.0.0"]);
        let first = git(repo.path(), &["rev-parse", "HEAD"]);
        let wider = manifest
            .replace("version = \"1.0.0\"", "version = \"1.1.0\"")
            .replace("hosts = [\"api.probe.test\"]", "hosts = [\"api.probe.test\", \"elsewhere.test\"]");
        std::fs::write(repo.path().join("silt-plugin.toml"), &wider).unwrap();
        git(repo.path(), &["commit", "-qam", "Reach elsewhere"]);
        git(repo.path(), &["tag", "-a", "v1.1.0", "-m", "1.1.0"]);
        let second = git(repo.path(), &["rev-parse", "HEAD"]);
        let url = format!("file://{}", repo.path().display());

        let data = tempfile::tempdir().unwrap();
        installed::write_files(data.path(), manifest, b"", None).unwrap();
        let plugin = Installed {
            id: "test.probe".into(),
            source: PluginSource::Git { url: url.clone(), commit: first.clone(), follow: Some("v1.0.0".into()) },
            version: "1.0.0".into(),
            granted: ["http".to_owned()].into(),
            workspaces: Vec::new(),
            environments: Default::default(),
            credentials: Default::default(),
            settings: Default::default(),
            rules: Vec::new(),
            consented: Vec::new(),
        };
        installed::save(data.path(), std::slice::from_ref(&plugin)).unwrap();

        let found = check(data.path());
        assert_eq!(found.len(), 1);
        let update = &found[0];
        assert_eq!((update.from.as_str(), update.to.as_str(), update.reference.as_str()), ("1.0.0", "1.1.0", "v1.1.0"));
        assert_eq!(update.commit, second, "an annotated tag's commit, not the tag's own object");
        assert!(update.adds.iter().any(|line| line.contains("elsewhere.test")), "{:?}", update.adds);
        assert_eq!(update.commits.len(), 1);
        assert_eq!(waiting(data.path()), found);
        assert!(installed::views(data.path())[0].update.is_some());

        // What it adds needs agreeing to.
        let refused = tokio::runtime::Runtime::new().unwrap().block_on(apply(data.path(), "test.probe", false));
        assert!(refused.is_err_and(|error| error.to_string().contains("asks for more")));

        // A plugin pinned to a commit follows nothing.
        let pinned = Installed { source: PluginSource::Git { url, commit: first, follow: None }, ..plugin };
        assert_eq!(find(data.path(), &pinned, &[]).unwrap(), None);
    }

    #[test]
    fn a_tag_is_offered_the_highest_version_after_it_and_a_branch_its_head() {
        let listed = refs(&[
            ("refs/heads/main", "m"),
            ("refs/tags/v1.0.0", "a"),
            ("refs/tags/v1.10.0", "t"),
            ("refs/tags/v1.10.0^{}", "c"),
            ("refs/tags/v1.9.0", "b"),
            ("refs/tags/v2.0.0-rc1", "r"),
            ("refs/tags/nightly", "n"),
        ]);
        assert_eq!(newest(&listed, "v1.0.0"), Some(("v1.10.0".to_owned(), "c".to_owned())));
        assert_eq!(newest(&listed, "v1.10.0"), None);
        assert_eq!(newest(&listed, "main"), Some(("main".to_owned(), "m".to_owned())));
        assert_eq!(newest(&listed, "nightly"), None, "a tag that isn't a version is followed nowhere");
    }
}
