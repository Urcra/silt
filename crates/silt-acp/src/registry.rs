//! The agents Silt can start (ADR 0005): the public ACP registry, cached for
//! a day with a bundled snapshot to fall back on, plus the user's own.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;

pub const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
const SNAPSHOT: &str = include_str!("../registry-snapshot.json");
const CACHE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("the agent {0} has no build for this machine")]
    NoBuild(String),
    #[error("the download of {0} failed: {1}")]
    Download(String, String),
    #[error("the download of {0} did not match its checksum")]
    Checksum(String),
    #[error("{0} has no checksum to verify its download against; set SILT_ALLOW_UNVERIFIED_AGENTS=1 to install it anyway")]
    Unverified(String),
    #[error("the registry entry for {0} names paths outside its own directory")]
    UnsafeEntry(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// An agent to offer, and how to start it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// The command to run, unless it must be downloaded first.
    #[serde(skip)]
    pub distribution: Distribution,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Distribution {
    /// A command on this machine.
    Command { program: String, args: Vec<String>, env: Vec<(String, String)> },
    /// A per-platform archive to download, check and unpack.
    Binary { archive: String, cmd: String, args: Vec<String>, sha256: Option<String> },
    #[default]
    Unavailable,
}

/// The registry's platform name for this machine, if it names one.
fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("macos", "aarch64") => Some("darwin-aarch64"),
        ("macos", "x86_64") => Some("darwin-x86_64"),
        _ => None,
    }
}

/// A registry document. Agents are read one by one, so one this version
/// can't read leaves the others.
#[derive(Deserialize)]
struct Document {
    #[serde(default)]
    agents: Vec<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Entry {
    id: String,
    name: String,
    description: String,
    distribution: Distributions,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Distributions {
    npx: Option<Package>,
    uvx: Option<Package>,
    /// Builds by platform name.
    binary: HashMap<String, Build>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Package {
    package: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Build {
    archive: String,
    cmd: String,
    args: Vec<String>,
    sha256: Option<String>,
}

impl Distributions {
    fn for_this_machine(self) -> Distribution {
        if let Some(npx) = self.npx {
            let args = ["-y".to_owned(), npx.package].into_iter().chain(npx.args).collect();
            return Distribution::Command { program: "npx".into(), args, env: npx.env.into_iter().collect() };
        }
        if let Some(uvx) = self.uvx {
            let args = std::iter::once(uvx.package).chain(uvx.args).collect();
            return Distribution::Command { program: "uvx".into(), args, env: uvx.env.into_iter().collect() };
        }
        let mut binary = self.binary;
        match platform().and_then(|platform| binary.remove(platform)) {
            Some(build) => {
                Distribution::Binary { archive: build.archive, cmd: build.cmd, args: build.args, sha256: build.sha256 }
            }
            None => Distribution::Unavailable,
        }
    }
}

/// The agents in a registry document.
pub fn parse(text: &str) -> Vec<AgentInfo> {
    let Ok(document) = serde_json::from_str::<Document>(text) else {
        return Vec::new();
    };
    document
        .agents
        .into_iter()
        .filter_map(|agent| serde_json::from_value::<Entry>(agent).ok())
        .map(|entry| AgentInfo {
            id: entry.id,
            name: entry.name,
            description: entry.description,
            distribution: entry.distribution.for_this_machine(),
        })
        .filter(|agent| agent.distribution != Distribution::Unavailable)
        .collect()
}

/// The registry as cached in `data_dir`, or the bundled snapshot.
pub fn cached(data_dir: &Path) -> Vec<AgentInfo> {
    let from_cache = std::fs::read_to_string(data_dir.join("acp-registry.json")).map(|text| parse(&text)).unwrap_or_default();
    if from_cache.is_empty() { parse(SNAPSHOT) } else { from_cache }
}

/// Downloads the registry into the cache when it is older than a day.
pub async fn refresh(data_dir: PathBuf) {
    let cache = data_dir.join("acp-registry.json");
    let fresh = std::fs::metadata(&cache)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| SystemTime::now().duration_since(modified).is_ok_and(|age| age < CACHE_AGE));
    if !fresh
        && let Some(text) = download_text(REGISTRY_URL).await
        && !parse(&text).is_empty()
    {
        let _ = std::fs::write(&cache, &text);
    }
}

/// The environment an agent starts with (ADR 0016): what tools need, proxy
/// and certificate settings, its own vendor's variables, the names listed
/// in `<data>/agent-env`, one per line. Other credentials in the daemon's
/// environment stay with the daemon.
pub fn agent_environment(agent_id: &str, data_dir: &Path) -> Vec<(String, String)> {
    const TOOLS: [&str; 22] = [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "LANG",
        "LANGUAGE",
        "TERM",
        "TZ",
        "COLORTERM",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "GOPATH",
        "GOROOT",
        "JAVA_HOME",
        "VIRTUAL_ENV",
        "CONDA_PREFIX",
        "NVM_DIR",
        "PNPM_HOME",
        "NODE_PATH",
        "PYTHONPATH",
        "DENO_DIR",
    ];
    const SHARED: [&str; 8] = [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "NODE_EXTRA_CA_CERTS",
    ];
    let vendor: &[&str] = match agent_id {
        id if id.contains("claude") => &["ANTHROPIC_", "CLAUDE_"],
        id if id.contains("codex") || id.contains("openai") => &["OPENAI_", "CODEX_"],
        id if id.contains("gemini") => &["GEMINI_", "GOOGLE_"],
        id if id.contains("copilot") => &["GITHUB_", "GH_", "COPILOT_"],
        _ => &[],
    };
    let listed: Vec<String> = std::fs::read_to_string(data_dir.join("agent-env"))
        .map(|text| {
            text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).map(str::to_owned).collect()
        })
        .unwrap_or_default();
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| {
            TOOLS.contains(&name.as_str())
                || name.starts_with("LC_")
                || SHARED.contains(&name.as_str())
                || name.starts_with("XDG_")
                || vendor.iter().any(|prefix| name.starts_with(prefix))
                || listed.contains(name)
        })
        .collect()
}

/// Agents the user added in `<data>/agents.json`:
/// `[{"id", "name", "command", "args": [], "env": {}, "pass_env": []}]`;
/// `pass_env` names variables of the daemon's environment the agent gets.
pub fn custom(data_dir: &Path) -> Vec<AgentInfo> {
    let Ok(text) = std::fs::read_to_string(data_dir.join("agents.json")) else {
        return Vec::new();
    };
    let Ok(agents) = serde_json::from_str::<Vec<Value>>(&text) else {
        tracing::warn!("agents.json is not a list of agents");
        return Vec::new();
    };
    agents.into_iter().filter_map(|agent| serde_json::from_value::<Custom>(agent).ok()).map(Custom::into_info).collect()
}

/// An agent in `agents.json`.
#[derive(Deserialize)]
struct Custom {
    id: String,
    name: Option<String>,
    #[serde(default)]
    description: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    pass_env: Vec<String>,
}

impl Custom {
    fn into_info(self) -> AgentInfo {
        let passed = self.pass_env.into_iter().filter_map(|name| {
            let value = std::env::var(&name).ok()?;
            Some((name, value))
        });
        AgentInfo {
            name: self.name.unwrap_or_else(|| self.id.clone()),
            id: self.id,
            description: self.description,
            distribution: Distribution::Command {
                program: self.command,
                args: self.args,
                env: self.env.into_iter().chain(passed).collect(),
            },
        }
    }
}

/// Every agent to offer: custom ones first, then the registry's.
pub fn all(data_dir: &Path) -> Vec<AgentInfo> {
    let mut agents = custom(data_dir);
    agents.extend(cached(data_dir));
    agents
}

/// The text at `url`, or none when it can't be had; the bundled snapshot
/// stands in for it then.
async fn download_text(url: &str) -> Option<String> {
    let output = tokio::process::Command::new("curl").args(["-fsSL", "--max-time", "5", url]).output().await.ok()?;
    output.status.success().then(|| String::from_utf8(output.stdout).ok()).flatten()
}

/// How to start an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// The command that starts `agent`, downloading and unpacking a binary
/// distribution into `data_dir` first when needed.
pub async fn command(agent: &AgentInfo, data_dir: &Path) -> Result<AgentCommand, RegistryError> {
    match &agent.distribution {
        Distribution::Command { program, args, env } => {
            Ok(AgentCommand { program: program.clone(), args: args.clone(), env: env.clone() })
        }
        Distribution::Binary { archive, cmd, args, sha256 } => {
            let program = installed(agent, archive, cmd, sha256.as_deref(), data_dir).await?;
            Ok(AgentCommand { program: program.to_string_lossy().into_owned(), args: args.clone(), env: Vec::new() })
        }
        Distribution::Unavailable => Err(RegistryError::NoBuild(agent.id.clone())),
    }
}

fn digest(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Whether `path` is relative and made only of plain names.
fn is_plain_relative(path: &str) -> bool {
    !path.is_empty() && Path::new(path).components().all(|component| matches!(component, std::path::Component::Normal(_)))
}

/// The agent's program, installed and verified (ADR 0005, ADR 0016).
/// Registry entries come from the network, so the id and command must stay
/// inside the agent's directory, the download must match its checksum,
/// and the installed program is checked again before every start.
async fn installed(
    agent: &AgentInfo,
    archive: &str,
    cmd: &str,
    sha256: Option<&str>,
    data_dir: &Path,
) -> Result<PathBuf, RegistryError> {
    if !is_plain_relative(&agent.id) || agent.id.contains(std::path::MAIN_SEPARATOR) || !is_plain_relative(cmd) {
        return Err(RegistryError::UnsafeEntry(agent.id.clone()));
    }
    let unverified_allowed = std::env::var_os("SILT_ALLOW_UNVERIFIED_AGENTS").is_some_and(|value| value == "1");
    if sha256.is_none() && !unverified_allowed {
        return Err(RegistryError::Unverified(agent.id.clone()));
    }
    let dir = data_dir.join("agents").join(&agent.id).join(digest(archive.as_bytes()));
    let program = dir.join(cmd);
    let stamp = dir.join(".silt-program-sha256");
    let intact = match (tokio::fs::read(&program).await, tokio::fs::read_to_string(&stamp).await) {
        (Ok(bytes), Ok(expected)) => digest(&bytes) == expected.trim(),
        _ => false,
    };
    if !intact {
        install(archive, cmd, sha256, &dir).await?;
    }
    Ok(program)
}

/// Downloads, checks and unpacks into a directory of its own, then moves
/// the result into place whole, so a failed or concurrent install never
/// leaves a half-unpacked program behind.
async fn install(archive: &str, cmd: &str, sha256: Option<&str>, dir: &Path) -> Result<(), RegistryError> {
    let parent = dir.parent().unwrap_or(dir);
    tokio::fs::create_dir_all(parent).await?;
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_nanos());
    let staging = parent.join(format!(".install-{}-{nanos}", std::process::id()));
    let unpacked = staging.join("unpacked");
    tokio::fs::create_dir_all(&unpacked).await?;
    let result = stage(archive, cmd, sha256, &staging, &unpacked).await;
    if result.is_ok() {
        let _ = tokio::fs::remove_dir_all(dir).await;
        tokio::fs::rename(&unpacked, dir).await?;
    }
    let _ = tokio::fs::remove_dir_all(&staging).await;
    result
}

async fn stage(archive: &str, cmd: &str, sha256: Option<&str>, staging: &Path, unpacked: &Path) -> Result<(), RegistryError> {
    let file = staging.join("download");
    let status = tokio::process::Command::new("curl")
        .args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--max-time", "900", "-o"])
        .arg(&file)
        .arg("--")
        .arg(archive)
        .status()
        .await?;
    if !status.success() {
        return Err(RegistryError::Download(archive.to_owned(), status.to_string()));
    }
    let bytes = tokio::fs::read(&file).await?;
    if let Some(expected) = sha256
        && !digest(&bytes).eq_ignore_ascii_case(expected)
    {
        return Err(RegistryError::Checksum(archive.to_owned()));
    }
    let name = archive.rsplit('/').next().unwrap_or(archive);
    let unpack = |program: &str| {
        let mut command = tokio::process::Command::new(program);
        command.current_dir(unpacked);
        command
    };
    let unpacked_ok = if name.ends_with(".zip") {
        unpack("unzip").arg("-q").arg(&file).arg("-d").arg(unpacked).status().await?.success()
    } else if [".tar", ".tar.gz", ".tgz", ".tar.xz", ".tar.bz2", ".tbz2"].iter().any(|suffix| name.ends_with(suffix)) {
        unpack("tar").arg("-xf").arg(&file).arg("-C").arg(unpacked).arg("--no-same-owner").status().await?.success()
    } else {
        // A bare program rather than an archive.
        let target = unpacked.join(cmd);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::copy(&file, &target).await?;
        true
    };
    if !unpacked_ok {
        return Err(RegistryError::Download(archive.to_owned(), "could not unpack it".into()));
    }
    let program = unpacked.join(cmd);
    let real = tokio::fs::canonicalize(&program).await?;
    let root = tokio::fs::canonicalize(unpacked).await?;
    let metadata = tokio::fs::symlink_metadata(&program).await?;
    if !real.starts_with(&root) || !metadata.is_file() {
        return Err(RegistryError::UnsafeEntry(archive.to_owned()));
    }
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o700);
        tokio::fs::set_permissions(&program, permissions).await?;
    }
    let program_digest = digest(&tokio::fs::read(&program).await?);
    tokio::fs::write(unpacked.join(".silt-program-sha256"), program_digest).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn entries_that_could_escape_or_skip_verification_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = |id: &str, cmd: &str, sha256: Option<&str>| AgentInfo {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            distribution: Distribution::Binary {
                archive: "https://example.invalid/agent.tar.gz".into(),
                cmd: cmd.into(),
                args: Vec::new(),
                sha256: sha256.map(str::to_owned),
            },
        };
        for (id, cmd) in [("../x", "agent"), ("ok", "../../bin/sh"), ("ok", "/bin/sh")] {
            let refused = command(&agent(id, cmd, Some("00")), dir.path()).await;
            assert!(matches!(refused, Err(RegistryError::UnsafeEntry(_))), "{id} {cmd}: {refused:?}");
        }
        let unverified = command(&agent("ok", "agent", None), dir.path()).await;
        assert!(matches!(unverified, Err(RegistryError::Unverified(_))), "{unverified:?}");
    }

    #[test]
    fn an_agent_this_version_cannot_read_leaves_the_others() {
        let document = r#"{"agents": [
            {"id": "odd", "distribution": {"npx": {"package": 7}}},
            {"id": "fine", "name": "Fine", "distribution": {"uvx": {"package": "fine", "args": ["--acp"], "env": {"A": "1"}}}},
            {"id": "elsewhere", "distribution": {"binary": {"plan9-mips": {"archive": "x", "cmd": "y"}}}}
        ]}"#;
        let agents = parse(document);
        assert_eq!(agents.len(), 1, "{agents:?}");
        assert_eq!(
            agents[0].distribution,
            Distribution::Command {
                program: "uvx".into(),
                args: vec!["fine".into(), "--acp".into()],
                env: vec![("A".into(), "1".into())]
            }
        );
    }

    #[test]
    fn the_snapshot_lists_agents_with_their_commands() {
        let agents = parse(SNAPSHOT);
        let claude = agents.iter().find(|agent| agent.id == "claude-acp").expect("Claude is listed");
        assert_eq!(
            claude.distribution,
            Distribution::Command {
                program: "npx".into(),
                args: vec!["-y".into(), "@agentclientprotocol/claude-agent-acp@0.94.0".into()],
                env: Vec::new(),
            }
        );
        assert!(agents.iter().any(|agent| matches!(agent.distribution, Distribution::Binary { .. })));
        assert!(agents.len() > 20);
    }
}
