//! Installing plugins (ADR 0103): from a folder being developed, a git
//! repository pinned to a commit, or a release asset checked against its
//! hash. A repository's code is built in a sandbox that reaches nothing: its
//! crates are fetched first, outside, which runs none of them.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use silt_plugin::{Build, Manifest};

use super::installed::{self, Installed, PluginSource};
use crate::CoreError;

/// A plugin's files, ready to install.
pub struct Package {
    pub manifest_text: String,
    pub manifest: Manifest,
    pub wasm: Vec<u8>,
    pub readme: Option<String>,
}

fn invalid(why: impl Into<String>) -> CoreError {
    CoreError::Invalid(why.into())
}

/// Reads and builds the plugin in `dir`. `trusted_code` is false for a
/// repository someone else wrote, whose build then runs sandboxed.
pub async fn package(dir: &Path, trusted_code: bool) -> Result<Package, CoreError> {
    let manifest_text = std::fs::read_to_string(dir.join("silt-plugin.toml"))
        .map_err(|error| invalid(format!("{} has no silt-plugin.toml: {error}", dir.display())))?;
    let manifest = Manifest::parse(&manifest_text).map_err(|error| invalid(error.to_string()))?;
    let readme = std::fs::read_to_string(dir.join(&manifest.docs.readme)).ok();
    let wasm = match &manifest.build {
        Build::Shipped => return Err(invalid("a plugin shipped with Silt comes with Silt")),
        Build::Asset { url, sha256 } => fetch_asset(url, sha256).await?,
        Build::Cargo { package, toolchain } => {
            let dir = dir.to_owned();
            let (package, toolchain) = (package.clone(), toolchain.clone());
            tokio::task::spawn_blocking(move || build_cargo(&dir, &package, &toolchain, trusted_code))
                .await
                .map_err(|error| invalid(error.to_string()))??
        }
    };
    Ok(Package { manifest_text, manifest, wasm, readme })
}

async fn fetch_asset(url: &str, sha256: &str) -> Result<Vec<u8>, CoreError> {
    let response = reqwest::get(url).await.map_err(|error| invalid(format!("{url}: {error}")))?;
    if !response.status().is_success() {
        return Err(invalid(format!("{url} answered {}", response.status())));
    }
    let bytes = response.bytes().await.map_err(|error| invalid(error.to_string()))?.to_vec();
    let got = silt_plugin::sha256_hex(&bytes);
    if !got.eq_ignore_ascii_case(sha256) {
        return Err(invalid(format!(
            "{url} isn't what its manifest names (sha256 {} expected, {} received); nothing was installed",
            short(sha256),
            short(&got)
        )));
    }
    Ok(bytes)
}

fn short(hash: &str) -> String {
    if hash.len() > 12 { format!("{}…{}", &hash[..4], &hash[hash.len() - 4..]) } else { hash.to_owned() }
}

fn cargo(toolchain: &str) -> Command {
    let mut command = Command::new("cargo");
    command.arg(format!("+{toolchain}"));
    command
}

fn run(mut command: Command, what: &str) -> Result<(), CoreError> {
    let output = command.output().map_err(|error| invalid(format!("{what} didn't start: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(12).collect();
        Err(invalid(format!("{what} failed:\n{}", tail.into_iter().rev().collect::<Vec<_>>().join("\n"))))
    }
}

/// Builds `package` for `wasm32-wasip2`. Someone else's code builds offline
/// in bubblewrap, with only its own folder and the toolchain.
fn build_cargo(dir: &Path, package: &str, toolchain: &str, trusted_code: bool) -> Result<Vec<u8>, CoreError> {
    let target = dir.join("target");
    let cargo_home = dir.join(".silt-cargo-home");
    let manifest = dir.join("Cargo.toml");
    if !trusted_code {
        // Fetching runs none of the plugin's code: no build scripts, no
        // macros. It's the only step that reaches the network. Every
        // target's crates, as build scripts and macros build for this one.
        let mut fetch = cargo(toolchain);
        fetch.args(["fetch", "--manifest-path"]).arg(&manifest).env("CARGO_HOME", &cargo_home);
        run(fetch, "fetching its crates")?;
    }
    let mut build = cargo(toolchain);
    build
        .args(["build", "--release", "--target", "wasm32-wasip2", "-p", package, "--manifest-path"])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target);
    let build = if trusted_code {
        build
    } else {
        build.arg("--offline").env("CARGO_HOME", &cargo_home);
        sandboxed(build, dir)?
    };
    run(build, "building it")?;
    let name = package.replace('-', "_");
    std::fs::read(target.join("wasm32-wasip2/release").join(format!("{name}.wasm")))
        .map_err(|error| invalid(format!("the build made no {name}.wasm: {error}")))
}

/// `command` in bubblewrap: the system and the toolchain read-only, the
/// plugin's folder writable, nothing else of this machine, and no network.
fn sandboxed(command: Command, dir: &Path) -> Result<Command, CoreError> {
    let bwrap = which("bwrap")
        .ok_or_else(|| invalid("building someone else's plugin needs bubblewrap (bwrap), which this machine lacks"))?;
    let home = silt_os::home_dir().ok_or_else(|| invalid("no home folder"))?;
    let mut sandbox = Command::new(bwrap);
    sandbox.args(["--die-with-parent", "--unshare-all", "--new-session", "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);
    for system in ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/nix", "/opt"] {
        sandbox.args(["--ro-bind-try", system, system]);
    }
    // The toolchain where it is: `RUSTUP_HOME` and the `cargo` on `PATH`,
    // which needn't be under the home.
    let rustup = std::env::var_os("RUSTUP_HOME").map_or_else(|| home.join(".rustup"), PathBuf::from);
    let cargo_bin =
        which("cargo").and_then(|cargo| cargo.parent().map(Path::to_owned)).unwrap_or_else(|| home.join(".cargo/bin"));
    for tool in [&rustup, &cargo_bin] {
        sandbox.arg("--ro-bind-try").arg(tool).arg(tool);
    }
    sandbox.arg("--bind").arg(dir).arg(dir);
    sandbox.env_clear();
    for (name, value) in command.get_envs() {
        if let Some(value) = value {
            sandbox.env(name, value);
        }
    }
    sandbox.env("HOME", &home).env("PATH", format!("{}:/usr/bin:/bin", cargo_bin.display())).env("RUSTUP_HOME", &rustup);
    sandbox.arg("--chdir").arg(dir).arg("--").arg(command.get_program()).args(command.get_args());
    Ok(sandbox)
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(program)).find(|candidate| candidate.is_file())
}

/// Clones `url` at `reference` (a tag, branch or commit; the default
/// branch when none) with the person's own git sign-in, outside every
/// sandbox, into `into`. Answers the commit.
pub fn clone(url: &str, reference: Option<&str>, into: &Path) -> Result<String, CoreError> {
    if url.starts_with('-') || reference.is_some_and(|reference| reference.starts_with('-')) {
        return Err(invalid("a repository or reference can't start with `-`"));
    }
    let mut command = Command::new("git");
    command.args(["clone", "--quiet", "--no-hardlinks", "--", url]).arg(into).env("GIT_TERMINAL_PROMPT", "0");
    run(command, "cloning it")?;
    let git = |args: &[&str]| -> Result<String, CoreError> {
        let output = Command::new("git").arg("-C").arg(into).args(["-c", "core.hooksPath=/dev/null"]).args(args).output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            Err(invalid(String::from_utf8_lossy(&output.stderr).trim().to_owned()))
        }
    };
    if let Some(reference) = reference {
        git(&["checkout", "--quiet", "--detach", reference])?;
    }
    git(&["rev-parse", "HEAD"])
}

/// Installs or updates `package` from `source`. An update that asks for
/// what wasn't granted needs `consented`; what it adds is said.
pub fn install(
    data_dir: &Path,
    package: &Package,
    source: PluginSource,
    workspaces: &[String],
    consented: bool,
) -> Result<Installed, CoreError> {
    let id = package.manifest.id.clone();
    let previous = installed::find(data_dir, &id);
    if let Some(previous) = &previous {
        let before = installed::manifest(data_dir, &id).ok();
        let added = before.map(|before| before.added_in(&package.manifest)).unwrap_or_default();
        if !added.is_empty() && !consented {
            return Err(invalid(format!(
                "{} {} asks for more than it has: {}. Add it again agreeing to that",
                package.manifest.name,
                package.manifest.version,
                added.join("; ")
            )));
        }
        if matches!(previous.source, PluginSource::Shipped) && !matches!(source, PluginSource::Shipped) {
            return Err(invalid(format!("{id} is shipped with Silt; it updates with Silt")));
        }
    }
    installed::write_files(data_dir, &package.manifest_text, &package.wasm, package.readme.as_deref())?;
    let declared = super::wasm::declared(&package.manifest);
    let record = match previous {
        Some(mut previous) => {
            previous.source = source;
            previous.version = package.manifest.version.clone();
            // What it no longer declares, it no longer has.
            previous.granted.retain(|capability| declared.contains(capability));
            previous.granted.extend(declared);
            for workspace in workspaces {
                if !previous.on_in(workspace) {
                    previous.workspaces.push(workspace.clone());
                }
            }
            previous
        }
        None => Installed {
            id: id.clone(),
            source,
            version: package.manifest.version.clone(),
            granted: declared,
            workspaces: workspaces.to_vec(),
            environments: Default::default(),
            credentials: Default::default(),
            settings: Default::default(),
            rules: Vec::new(),
            consented: Vec::new(),
        },
    };
    let mut record = record;
    if consented {
        record.consented = consent(&package.manifest);
    }
    let mut all = installed::load(data_dir);
    all.retain(|other| other.id != id);
    all.push(record.clone());
    all.sort_by(|one, other| one.id.cmp(&other.id));
    installed::save(data_dir, &all)?;
    Ok(record)
}

/// What adding a plugin from git would give it, read before anything of it
/// is built or run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Preview {
    pub id: String,
    pub commit: String,
    /// What adding it agrees to, in words.
    pub consent: Vec<String>,
}

/// `url#reference` split, the reference a tag, branch or commit.
fn source_of(source: &str) -> (&str, Option<&str>) {
    match source.rsplit_once('#') {
        Some((url, reference)) => (url, Some(reference)),
        None => (source, None),
    }
}

/// A folder in Silt's data a repository is cloned into for a while.
fn scratch(data_dir: &Path) -> PathBuf {
    data_dir.join("plugins").join(".checkouts").join(uuid::Uuid::new_v4().simple().to_string())
}

/// Clones `source` (`url#tag`) and reads its manifest, building nothing.
pub async fn preview(data_dir: &Path, source: &str) -> Result<Preview, CoreError> {
    let (url, reference) = source_of(source);
    let (url, reference, into) = (url.to_owned(), reference.map(str::to_owned), scratch(data_dir));
    let checkout = into.clone();
    let read = tokio::task::spawn_blocking(move || {
        let commit = clone(&url, reference.as_deref(), &checkout)?;
        let text = std::fs::read_to_string(checkout.join("silt-plugin.toml"))
            .map_err(|_| invalid(format!("{url} has no silt-plugin.toml at its root")))?;
        let manifest = Manifest::parse(&text).map_err(|error| invalid(error.to_string()))?;
        Ok::<_, CoreError>(Preview { id: manifest.id.clone(), commit, consent: consent(&manifest) })
    })
    .await
    .map_err(|error| invalid(error.to_string()))?;
    let _ = std::fs::remove_dir_all(&into);
    read
}

/// Adds the plugin `source` (`url#tag`) names, built offline in a sandbox,
/// on in `workspaces`; the person agreed to what [`preview`] showed of
/// `commit`, and nothing else is added.
pub async fn add_from_git(data_dir: &Path, source: &str, commit: &str, workspaces: &[String]) -> Result<Installed, CoreError> {
    let (url, reference) = source_of(source);
    let into = scratch(data_dir);
    let (owned_url, owned_reference, checkout) = (url.to_owned(), reference.map(str::to_owned), into.clone());
    let added = async {
        let cloned = tokio::task::spawn_blocking(move || clone(&owned_url, owned_reference.as_deref(), &checkout))
            .await
            .map_err(|error| invalid(error.to_string()))??;
        if cloned != commit {
            return Err(invalid(format!(
                "{url} moved to {} since you looked at {}; look again",
                &cloned[..cloned.len().min(7)],
                &commit[..commit.len().min(7)]
            )));
        }
        let commit = cloned;
        let package = package(&into, false).await?;
        let follow = reference.filter(|reference| !reference.chars().all(|c| c.is_ascii_hexdigit())).map(str::to_owned);
        install(data_dir, &package, PluginSource::Git { url: url.to_owned(), commit, follow }, workspaces, true)
    }
    .await;
    let _ = std::fs::remove_dir_all(&into);
    added
}

/// Installs `plugin` as org `org`'s policy names it: cloned at its commit
/// and nothing else, built offline in a sandbox, on and locked in
/// `workspaces`.
pub async fn install_for_org(
    data_dir: &Path,
    org: &str,
    plugin: &silt_workspace::OrgPlugin,
    workspaces: &[String],
) -> Result<Installed, CoreError> {
    let checkout = data_dir.join("plugins").join(".org-checkouts").join(uuid::Uuid::new_v4().simple().to_string());
    let (url, commit) = (plugin.git.clone(), plugin.commit.clone());
    let into = checkout.clone();
    let cloned = tokio::task::spawn_blocking(move || clone(&url, Some(&commit), &into))
        .await
        .map_err(|error| invalid(error.to_string()))?;
    let result = async {
        let at = cloned?;
        if !at.starts_with(&plugin.commit) && !plugin.commit.starts_with(&at) {
            return Err(invalid(format!("{} isn't at {}", plugin.git, plugin.commit)));
        }
        let package = package(&checkout, false).await?;
        if package.manifest.id != plugin.id {
            return Err(invalid(format!("{} holds {}, not {}", plugin.git, package.manifest.id, plugin.id)));
        }
        let source = PluginSource::Org { org: org.to_owned(), commit: Some(plugin.commit.clone()) };
        install(data_dir, &package, source, workspaces, true)
    }
    .await;
    let _ = std::fs::remove_dir_all(&checkout);
    result
}

/// Removes plugin `id` and everything it kept. Silt's own can't be removed,
/// only turned off.
pub fn remove(data_dir: &Path, id: &str) -> Result<(), CoreError> {
    let mut all = installed::load(data_dir);
    let found = all.iter().find(|installed| installed.id == id).ok_or_else(|| invalid(format!("no plugin {id}")))?;
    if matches!(found.source, PluginSource::Shipped) {
        return Err(invalid(format!("{id} is shipped with Silt: turn it off in a workspace instead")));
    }
    if let PluginSource::Org { org, .. } = &found.source {
        return Err(invalid(format!("{org}'s policy installs {id}; it goes when the policy no longer names it")));
    }
    all.retain(|installed| installed.id != id);
    installed::save(data_dir, &all)?;
    let dir = installed::dir(data_dir, id);
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

/// The commits from `old` to `new` in the repository at `dir`, newest
/// first, as `<short id> <subject>`: what an update brings.
pub fn commits_between(dir: &Path, old: &str, new: &str) -> Result<Vec<String>, CoreError> {
    if [old, new].iter().any(|commit| commit.is_empty() || !commit.chars().all(|c| c.is_ascii_hexdigit())) {
        return Err(invalid("a commit is named by its id"));
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.hooksPath=/dev/null", "log", "--format=%h %s", "-n", "100"])
        .arg(format!("{old}..{new}"))
        .arg("--")
        .output()?;
    if !output.status.success() {
        return Err(invalid(String::from_utf8_lossy(&output.stderr).trim().to_owned()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).lines().map(str::to_owned).collect())
}

/// When the newest file of the plugin in `dir` changed, past its build
/// output and git's: what a dev folder is watched by.
pub fn changed_at(dir: &Path) -> Option<std::time::SystemTime> {
    let mut newest = None;
    let mut stack = vec![dir.to_owned()];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder).ok()?.flatten() {
            let name = entry.file_name();
            if matches!(name.to_str(), Some("target" | ".git" | ".silt-cargo-home")) {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) {
                newest = newest.max(Some(modified));
            }
        }
    }
    newest
}

/// What a person agrees to by adding `manifest`, in words.
pub fn consent(manifest: &Manifest) -> Vec<String> {
    let mut lines = vec![format!("{} {} by {}: {}", manifest.name, manifest.version, manifest.publisher, manifest.description)];
    for capability in &manifest.capabilities {
        let hosts = capability.hosts();
        let reach = if hosts.is_empty() { String::new() } else { format!(" ({})", hosts.join(", ")) };
        let opaque = if capability.opaque() { "; Silt can't check what these calls send" } else { "" };
        lines.push(format!("  {}{reach}: {}{opaque}", capability.name(), capability.reason()));
    }
    for credential in manifest.credentials.iter().filter(|credential| credential.same_as.is_none()) {
        let hosts: Vec<&str> = manifest
            .credentials
            .iter()
            .filter(|other| other.id == credential.id || other.same_as.as_deref() == Some(credential.id.as_str()))
            .flat_map(|other| other.hosts.iter().map(String::as_str))
            .collect();
        lines.push(format!("  needs {}, sent to {} only, never to the plugin", credential.label, hosts.join(", ")));
    }
    for action in &manifest.actions {
        let kind = if action.kind == silt_plugin::ActionKind::Read { "read" } else { "change" };
        lines.push(format!("  {} ({kind}): {}", action.label, action.description));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_repository_is_pinned_to_its_tag_and_an_update_names_what_came_since() {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q"]);
        std::fs::write(repo.path().join("silt-plugin.toml"), "v1").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "First"]);
        git(repo.path(), &["tag", "v1"]);
        for subject in ["Add a tool", "Fix its card"] {
            std::fs::write(repo.path().join("README.md"), subject).unwrap();
            git(repo.path(), &["add", "."]);
            git(repo.path(), &["commit", "-qm", subject]);
        }
        git(repo.path(), &["tag", "v2"]);
        let url = format!("file://{}", repo.path().display());

        let first = tempfile::tempdir().unwrap();
        let at_v1 = clone(&url, Some("v1"), &first.path().join("plugin")).unwrap();
        assert_eq!(at_v1, git(repo.path(), &["rev-parse", "v1^{commit}"]));
        let second = tempfile::tempdir().unwrap();
        let at_v2 = clone(&url, Some("v2"), &second.path().join("plugin")).unwrap();
        let since = commits_between(&second.path().join("plugin"), &at_v1, &at_v2).unwrap();
        assert_eq!(since.iter().map(|line| line.split_once(' ').unwrap().1).collect::<Vec<_>>(), ["Fix its card", "Add a tool"]);
        assert!(commits_between(&second.path().join("plugin"), "--all", &at_v2).is_err());
        assert!(clone("--upload-pack=touch /tmp/x", None, &first.path().join("bad")).is_err());
    }

    #[test]
    fn a_dev_folder_changes_with_its_files_but_not_its_build() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "1").unwrap();
        let before = changed_at(dir.path());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/out.wasm"), "built").unwrap();
        assert_eq!(changed_at(dir.path()), before, "its build isn't a change");
        std::fs::write(dir.path().join("src/lib.rs"), "2").unwrap();
        assert!(changed_at(dir.path()) > before);
    }
}
