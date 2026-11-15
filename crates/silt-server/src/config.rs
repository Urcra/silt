//! `server.toml` in the server's data dir (ADR 0013).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CONFIG_FILE: &str = "server.toml";
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7450";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: String,
    /// The URL people and devices reach the server at, used in links and
    /// sign-in redirects; defaults to `http://<listen>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
    #[serde(default)]
    pub signup: Signup,
    #[serde(default)]
    pub auth: Auth,
    /// Days a deleted thread stays in the trash before it is purged.
    #[serde(default = "default_trash_days")]
    pub trash_days: u32,
    /// The forges whose pull requests get a comment naming their thread,
    /// for projects that turn it on (ADR 0042).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forges: Vec<Forge>,
    /// Artifacts' frames load libraries and fonts through this server,
    /// which fetches and keeps each file, rather than from the CDNs, so a
    /// viewer never contacts one (ADR 0071).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub artifact_proxy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Forge {
    /// The host projects name it by, such as `github.com`.
    pub host: String,
    pub kind: crate::forge::ForgeKind,
    /// Its API's root; by default the forge's usual one on `host`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_file: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            public_url: None,
            signup: Signup::default(),
            auth: Auth::default(),
            trash_days: default_trash_days(),
            forges: Vec::new(),
            artifact_proxy: false,
        }
    }
}

fn default_listen() -> String {
    DEFAULT_LISTEN.to_owned()
}

fn default_trash_days() -> u32 {
    30
}

/// Who may create an account on first sign-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Signup {
    #[serde(default)]
    pub policy: SignupPolicy,
    /// With `domains`, people whose verified email is at one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SignupPolicy {
    /// Only people with an invite.
    #[default]
    Invite,
    /// Anyone who can reach the server and sign in.
    Open,
    /// People whose verified email is in one of `domains`, plus invites.
    Domains,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    /// Whether local accounts may sign in with a password.
    #[serde(default = "yes")]
    pub passwords: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GitHub>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc: Option<Oidc>,
}

impl Default for Auth {
    fn default() -> Self {
        Self { passwords: true, github: None, oidc: None }
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHub {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret_file: Option<PathBuf>,
    /// For GitHub Enterprise.
    #[serde(default = "github_web")]
    pub web_url: String,
    #[serde(default = "github_api")]
    pub api_url: String,
}

fn github_web() -> String {
    "https://github.com".into()
}

fn github_api() -> String {
    "https://api.github.com".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Oidc {
    /// What the sign-in button says.
    #[serde(default = "oidc_name")]
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret_file: Option<PathBuf>,
    #[serde(default = "oidc_scopes")]
    pub scopes: String,
}

fn oidc_name() -> String {
    "single sign-on".into()
}

fn oidc_scopes() -> String {
    "openid email profile".into()
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read {path}")]
    Read { path: PathBuf, source: io::Error },
    #[error("{path} is not valid: {source}")]
    Parse { path: PathBuf, source: toml::de::Error },
    #[error("could not read the secret in {path}")]
    Secret { path: PathBuf, source: io::Error },
}

impl Config {
    /// The config in `data_dir`, or the defaults when there is none.
    pub fn load(data_dir: &Path) -> Result<Self, ConfigError> {
        let path = data_dir.join(CONFIG_FILE);
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|source| ConfigError::Parse { path, source }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Read { path, source }),
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("the config serializes")
    }

    pub fn public_url(&self) -> String {
        self.public_url.clone().unwrap_or_else(|| format!("http://{}", self.listen)).trim_end_matches('/').to_owned()
    }
}

/// A client secret given inline or in a file.
pub fn secret(inline: Option<&String>, file: Option<&PathBuf>) -> Result<String, ConfigError> {
    match (inline, file) {
        (Some(secret), _) => Ok(secret.clone()),
        (None, Some(path)) => fs::read_to_string(path)
            .map(|text| text.trim().to_owned())
            .map_err(|source| ConfigError::Secret { path: path.clone(), source }),
        (None, None) => Ok(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_fill_in_a_small_config() {
        let config: Config = toml::from_str(
            r#"
            public_url = "https://silt.example.com/"
            [signup]
            policy = "domains"
            domains = ["example.com"]
            [auth.oidc]
            issuer = "https://sso.example.com"
            client_id = "silt"
            client_secret_file = "/etc/silt/oidc"
            [[forges]]
            host = "codeberg.org"
            kind = "forgejo"
            token_file = "/etc/silt/codeberg"
            "#,
        )
        .expect("parses");
        assert_eq!(config.listen, DEFAULT_LISTEN);
        assert_eq!(config.public_url(), "https://silt.example.com");
        assert_eq!(config.signup.policy, SignupPolicy::Domains);
        assert!(config.auth.passwords);
        assert_eq!(config.auth.oidc.as_ref().map(|oidc| oidc.scopes.as_str()), Some("openid email profile"));
        assert_eq!(config.trash_days, 30);
        assert_eq!(config.forges[0].kind, crate::forge::ForgeKind::Forgejo);
        let again: Config = toml::from_str(&config.to_toml()).expect("round trips");
        assert_eq!(again, config);
        assert!(toml::from_str::<Config>("listn = \"x\"").is_err(), "typos are caught");
    }
}
