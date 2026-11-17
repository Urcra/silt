//! Credentials the gateway adds for a workspace's sandboxed agents (ADR
//! 0093), kept per machine: which host and path each is for and where its
//! secret comes from, in `credentials.json`; the secret itself in the
//! system's keyring, or from a command run on this machine as it's needed.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::CoreError;

const FILE: &str = "credentials.json";

/// Where a credential's secret comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Source {
    /// Kept in the system's keyring under the credential's name.
    Keyring,
    /// What a command prints, run on this machine outside every sandbox,
    /// such as `gh auth token`.
    Command { argv: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCredential {
    /// Its name, unique on this machine.
    pub name: String,
    /// The workspace whose agents it's for.
    pub workspace: String,
    /// A host, or `*.example.com`.
    pub host: String,
    #[serde(default = "root")]
    pub path: String,
    #[serde(default = "authorization")]
    pub header: String,
    /// Put before the secret, such as `Bearer`; none when empty.
    #[serde(default)]
    pub scheme: String,
    pub source: Source,
    /// A variable set to a placeholder in the sandbox, for tools that want
    /// one before they send anything, such as `GH_TOKEN`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

fn root() -> String {
    "/".into()
}

fn authorization() -> String {
    "authorization".into()
}

pub fn load(data_dir: &Path) -> Vec<StoredCredential> {
    std::fs::read(data_dir.join(FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

pub fn save(data_dir: &Path, all: &[StoredCredential]) -> Result<(), CoreError> {
    let bytes = serde_json::to_vec_pretty(all).map_err(|error| CoreError::Invalid(error.to_string()))?;
    silt_os::write_private(&data_dir.join(FILE), &bytes)?;
    Ok(())
}

fn keyring_name(name: &str) -> String {
    format!("credential:{name}")
}

/// Keeps `secret` in the keyring for `name`.
pub fn set_secret(name: &str, secret: Option<&str>) -> Result<(), CoreError> {
    silt_os::set_secret(&keyring_name(name), secret)
        .map_err(|error| CoreError::Invalid(format!("the keyring refused it: {error}")))
}

/// The gateway's credentials for `workspace`, with their secrets, and the
/// placeholders the sandbox gets. One whose secret can't be had is left
/// out, saying why.
pub fn resolve(data_dir: &Path, workspace: &str) -> (Vec<silt_gateway::Credential>, Vec<(String, String)>, Vec<String>) {
    let mut credentials = Vec::new();
    let mut placeholders = Vec::new();
    let mut missing = Vec::new();
    for stored in load(data_dir).into_iter().filter(|stored| stored.workspace == workspace) {
        let Some(credential) = secret_of(&stored) else {
            missing.push(stored.name.clone());
            continue;
        };
        if let Some(variable) = &stored.placeholder {
            placeholders.push((variable.clone(), "silt-placeholder".to_owned()));
        }
        credentials.push(credential);
    }
    (credentials, placeholders, missing)
}

/// Workspace `workspace`'s credential `name`, with its secret, for a
/// plugin's requests; `None` when it has none here.
pub fn resolve_named(data_dir: &Path, workspace: &str, name: &str) -> Option<silt_gateway::Credential> {
    load(data_dir)
        .into_iter()
        .find(|stored| stored.workspace == workspace && stored.name == name)
        .and_then(|stored| secret_of(&stored))
}

/// Workspace `workspace`'s credential `name`'s secret alone, for a plugin's
/// program that reads it from a variable.
pub fn secret_named(data_dir: &Path, workspace: &str, name: &str) -> Option<String> {
    load(data_dir)
        .into_iter()
        .find(|stored| stored.workspace == workspace && stored.name == name)
        .and_then(|stored| raw_secret(&stored))
}

fn secret_of(stored: &StoredCredential) -> Option<silt_gateway::Credential> {
    let secret = raw_secret(stored)?;
    let value = if stored.scheme.is_empty() { secret } else { format!("{} {secret}", stored.scheme) };
    Some(silt_gateway::Credential {
        host: stored.host.clone(),
        path: stored.path.clone(),
        header: stored.header.to_ascii_lowercase(),
        value,
    })
}

fn raw_secret(stored: &StoredCredential) -> Option<String> {
    match &stored.source {
        Source::Keyring => silt_os::secret(&keyring_name(&stored.name)).ok().flatten(),
        // Run outside every sandbox, as the person, with what the run's
        // own environment wouldn't hold.
        Source::Command { argv } => argv.split_first().and_then(|(program, args)| {
            let output = std::process::Command::new(program).args(args).stdin(std::process::Stdio::null()).output().ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
                .filter(|secret| !secret.is_empty())
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commands_secret_goes_to_its_workspace_only() {
        let dir = tempfile::tempdir().unwrap();
        let stored = |name: &str, workspace: &str| StoredCredential {
            name: name.into(),
            workspace: workspace.into(),
            host: "api.github.com".into(),
            path: root(),
            header: "Authorization".into(),
            scheme: "Bearer".into(),
            source: Source::Command { argv: vec!["echo".into(), "s3cret".into()] },
            placeholder: Some("GH_TOKEN".into()),
        };
        save(dir.path(), &[stored("work-gh", "work"), stored("own-gh", "personal")]).unwrap();
        let (credentials, placeholders, missing) = resolve(dir.path(), "work");
        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].value, "Bearer s3cret");
        assert_eq!(credentials[0].header, "authorization");
        assert_eq!(placeholders, vec![("GH_TOKEN".to_owned(), "silt-placeholder".to_owned())]);
        assert!(missing.is_empty());
        let broken = StoredCredential { source: Source::Command { argv: vec!["false".into()] }, ..stored("broken", "work") };
        save(dir.path(), &[broken]).unwrap();
        assert_eq!(resolve(dir.path(), "work").2, vec!["broken".to_owned()]);
    }
}
