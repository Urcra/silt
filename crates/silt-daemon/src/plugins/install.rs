//! Installing plugins (ADR 0100): from a folder being developed, a git
//! repository pinned to a commit, or a release asset checked against its
//! hash. A repository's code is built in a sandbox that reaches nothing: its
//! crates are fetched first, outside, which runs none of them.

use std::path::{Path, PathBuf};
use std::process::Command;

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
        // macros. It's the only step that reaches the network.
        let mut fetch = cargo(toolchain);
        fetch.args(["fetch", "--target", "wasm32-wasip2", "--manifest-path"]).arg(&manifest).env("CARGO_HOME", &cargo_home);
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
    for tool in [home.join(".rustup"), home.join(".cargo/bin")] {
        sandbox.arg("--ro-bind-try").arg(&tool).arg(&tool);
    }
    sandbox.arg("--bind").arg(dir).arg(dir);
    sandbox.env_clear();
    for (name, value) in command.get_envs() {
        if let Some(value) = value {
            sandbox.env(name, value);
        }
    }
    sandbox.env("HOME", &home).env("PATH", format!("{}:/usr/bin:/bin", home.join(".cargo/bin").display()));
    if let Ok(rustup) = std::env::var("RUSTUP_HOME") {
        sandbox.env("RUSTUP_HOME", rustup);
    }
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
        },
    };
    let mut all = installed::load(data_dir);
    all.retain(|other| other.id != id);
    all.push(record.clone());
    all.sort_by(|one, other| one.id.cmp(&other.id));
    installed::save(data_dir, &all)?;
    Ok(record)
}

/// Removes plugin `id` and everything it kept. Silt's own can't be removed,
/// only turned off.
pub fn remove(data_dir: &Path, id: &str) -> Result<(), CoreError> {
    let mut all = installed::load(data_dir);
    let found = all.iter().find(|installed| installed.id == id).ok_or_else(|| invalid(format!("no plugin {id}")))?;
    if matches!(found.source, PluginSource::Shipped) {
        return Err(invalid(format!("{id} is shipped with Silt: turn it off in a workspace instead")));
    }
    all.retain(|installed| installed.id != id);
    installed::save(data_dir, &all)?;
    let dir = installed::dir(data_dir, id);
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
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
