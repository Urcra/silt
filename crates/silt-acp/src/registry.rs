//! The agents Silt can start (ADR 0005): the public ACP registry, cached for
//! a day with a bundled snapshot to fall back on, plus Silt's own harness.

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

/// The registry's platform name for this machine.
fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "aarch64") => "darwin-aarch64",
        ("macos", _) => "darwin-x86_64",
        _ => "linux-x86_64",
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn env(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_object)
        .map(|vars| vars.iter().filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned()))).collect())
        .unwrap_or_default()
}

fn distribution(value: &Value) -> Distribution {
    if let Some(npx) = value.get("npx") {
        let mut args = vec!["-y".to_owned(), npx["package"].as_str().unwrap_or_default().to_owned()];
        args.extend(strings(npx.get("args")));
        return Distribution::Command { program: "npx".into(), args, env: env(npx.get("env")) };
    }
    if let Some(uvx) = value.get("uvx") {
        let mut args = vec![uvx["package"].as_str().unwrap_or_default().to_owned()];
        args.extend(strings(uvx.get("args")));
        return Distribution::Command { program: "uvx".into(), args, env: env(uvx.get("env")) };
    }
    if let Some(build) = value.get("binary").and_then(|binary| binary.get(platform())) {
        return Distribution::Binary {
            archive: build["archive"].as_str().unwrap_or_default().to_owned(),
            cmd: build["cmd"].as_str().unwrap_or_default().to_owned(),
            args: strings(build.get("args")),
            sha256: build["sha256"].as_str().map(str::to_owned),
        };
    }
    Distribution::Unavailable
}

/// The agents in a registry document.
pub fn parse(text: &str) -> Vec<AgentInfo> {
    let Ok(document) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    document["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|agent| AgentInfo {
            id: agent["id"].as_str().unwrap_or_default().to_owned(),
            name: agent["name"].as_str().unwrap_or_default().to_owned(),
            description: agent["description"].as_str().unwrap_or_default().to_owned(),
            distribution: distribution(&agent["distribution"]),
        })
        .filter(|agent| agent.distribution != Distribution::Unavailable)
        .collect()
}

/// Silt's own agent, `silt harness` (ADR 0014).
pub fn harness(silt: &Path) -> AgentInfo {
    AgentInfo {
        id: "silt-harness".into(),
        name: "Silt harness".into(),
        description: "Silt's own agent on your Anthropic or OpenAI-compatible API key, running commands in a sandbox".into(),
        distribution: Distribution::Command {
            program: silt.to_string_lossy().into_owned(),
            args: vec!["harness".into()],
            env: Vec::new(),
        },
    }
}

/// The registry as cached in `data_dir`, or the bundled snapshot.
pub fn cached(data_dir: &Path) -> Vec<AgentInfo> {
    let from_cache = std::fs::read_to_string(data_dir.join("acp-registry.json"))
        .map(|text| parse(&text))
        .unwrap_or_default();
    if from_cache.is_empty() { parse(SNAPSHOT) } else { from_cache }
}

/// Downloads the registry into the cache when it is older than a day.
pub async fn refresh(data_dir: &Path) {
    let cache = data_dir.join("acp-registry.json");
    let fresh = std::fs::metadata(&cache)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| SystemTime::now().duration_since(modified).is_ok_and(|age| age < CACHE_AGE));
    if !fresh && let Ok(text) = download_text(REGISTRY_URL).await
        && !parse(&text).is_empty()
    {
        let _ = std::fs::write(&cache, &text);
    }
}

/// Agents the user added in `<data>/agents.json`:
/// `[{"id", "name", "command", "args": [], "env": {}}]`.
pub fn custom(data_dir: &Path) -> Vec<AgentInfo> {
    let Ok(text) = std::fs::read_to_string(data_dir.join("agents.json")) else {
        return Vec::new();
    };
    let Ok(Value::Array(agents)) = serde_json::from_str::<Value>(&text) else {
        tracing::warn!("agents.json is not a list of agents");
        return Vec::new();
    };
    agents
        .iter()
        .filter_map(|agent| {
            Some(AgentInfo {
                id: agent["id"].as_str()?.to_owned(),
                name: agent["name"].as_str().unwrap_or_else(|| agent["id"].as_str().unwrap_or_default()).to_owned(),
                description: agent["description"].as_str().unwrap_or_default().to_owned(),
                distribution: Distribution::Command {
                    program: agent["command"].as_str()?.to_owned(),
                    args: strings(agent.get("args")),
                    env: env(agent.get("env")),
                },
            })
        })
        .collect()
}

/// Every agent to offer: custom ones first, then Silt's harness, then the
/// registry's.
pub fn all(data_dir: &Path, silt: &Path) -> Vec<AgentInfo> {
    let mut agents = custom(data_dir);
    agents.push(harness(silt));
    agents.extend(cached(data_dir));
    agents
}

async fn download_text(url: &str) -> Result<String, String> {
    let output = tokio::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "5", url])
        .output()
        .await
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| error.to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

/// The command that starts `agent`, downloading and unpacking a binary
/// distribution into `data_dir` first when needed.
pub async fn command(agent: &AgentInfo, data_dir: &Path) -> Result<(String, Vec<String>, Vec<(String, String)>), RegistryError> {
    match &agent.distribution {
        Distribution::Command { program, args, env } => Ok((program.clone(), args.clone(), env.clone())),
        Distribution::Binary { archive, cmd, args, sha256 } => {
            let dir = data_dir.join("agents").join(&agent.id).join(digest(archive.as_bytes()));
            let program = dir.join(cmd);
            if !program.exists() {
                install(archive, sha256.as_deref(), &dir).await?;
            }
            Ok((program.to_string_lossy().into_owned(), args.clone(), Vec::new()))
        }
        Distribution::Unavailable => Err(RegistryError::NoBuild(agent.id.clone())),
    }
}

fn digest(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn install(archive: &str, sha256: Option<&str>, dir: &Path) -> Result<(), RegistryError> {
    tokio::fs::create_dir_all(dir).await?;
    let file: PathBuf = dir.join("download");
    let status = tokio::process::Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&file)
        .arg(archive)
        .status()
        .await?;
    if !status.success() {
        return Err(RegistryError::Download(archive.to_owned(), status.to_string()));
    }
    if let Some(expected) = sha256
        && !digest(&tokio::fs::read(&file).await?).eq_ignore_ascii_case(expected)
    {
        let _ = tokio::fs::remove_file(&file).await;
        return Err(RegistryError::Checksum(archive.to_owned()));
    }
    let unpack = if archive.ends_with(".zip") {
        tokio::process::Command::new("unzip").arg("-q").arg(&file).arg("-d").arg(dir).status().await?
    } else {
        tokio::process::Command::new("tar").arg("-xf").arg(&file).arg("-C").arg(dir).status().await?
    };
    let _ = tokio::fs::remove_file(&file).await;
    if unpack.success() {
        Ok(())
    } else {
        Err(RegistryError::Download(archive.to_owned(), "could not unpack it".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
