//! `silt-plugin.toml`: what a plugin is, how it's built, what it may do and
//! what it says about itself. Silt shows all of it before a plugin is added,
//! and holds the plugin to it.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::glob::Glob;
use crate::rules::Rule;

/// The world this Silt hosts; older ones keep loading as long as Silt does.
pub const WORLD: &str = "silt:plugin@1";

const KNOWN_WORLDS: [&str; 1] = [WORLD];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `publisher.name`, such as `silt.github` or `acme.deploys`.
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub description: String,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// The WIT world it was built for, such as `silt:plugin@1`.
    pub world: String,
    pub build: Build,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub actions: Vec<ActionSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environments: Option<Environments>,
    #[serde(default)]
    pub settings: Vec<SettingSpec>,
    #[serde(default)]
    pub credentials: Vec<CredentialSpec>,
    #[serde(default)]
    pub docs: Docs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Build {
    /// Built with cargo for `wasm32-wasip2`, with the toolchain named.
    Cargo { package: String, toolchain: String },
    /// A component built elsewhere, checked against its hash.
    Asset { url: String, sha256: String },
    /// Built into Silt, as its own plugins are.
    Shipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case", tag = "kind", deny_unknown_fields)]
pub enum Capability {
    Http { hosts: Vec<String>, reason: String },
    Git { hosts: Vec<String>, reason: String },
    AgentFiles { reason: String },
    ThreadRead { reason: String },
    Reach { reason: String },
    Sockets { hosts: Vec<String>, reason: String },
    Ssh { hosts: Vec<String>, reason: String },
    McpProcess { command: Vec<String>, hosts: Vec<String>, reason: String },
}

impl Capability {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Http { .. } => "http",
            Self::Git { .. } => "git",
            Self::AgentFiles { .. } => "agent-files",
            Self::ThreadRead { .. } => "thread-read",
            Self::Reach { .. } => "reach",
            Self::Sockets { .. } => "sockets",
            Self::Ssh { .. } => "ssh",
            Self::McpProcess { .. } => "mcp-process",
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            Self::Http { reason, .. }
            | Self::Git { reason, .. }
            | Self::AgentFiles { reason }
            | Self::ThreadRead { reason }
            | Self::Reach { reason }
            | Self::Sockets { reason, .. }
            | Self::Ssh { reason, .. }
            | Self::McpProcess { reason, .. } => reason,
        }
    }

    /// The hosts it reaches; patterns such as `*.acme.dev` allowed.
    pub fn hosts(&self) -> &[String] {
        match self {
            Self::Http { hosts, .. }
            | Self::Git { hosts, .. }
            | Self::Sockets { hosts, .. }
            | Self::Ssh { hosts, .. }
            | Self::McpProcess { hosts, .. } => hosts,
            Self::AgentFiles { .. } | Self::ThreadRead { .. } | Self::Reach { .. } => &[],
        }
    }

    /// Its calls are opaque: Silt can't check what they send.
    pub fn opaque(&self) -> bool {
        matches!(self, Self::Sockets { .. } | Self::Ssh { .. } | Self::McpProcess { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Read,
    Change,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ActionSpec {
    pub id: String,
    pub label: String,
    /// Markdown.
    pub description: String,
    pub kind: ActionKind,
    /// What it starts as, once the person trusts the publisher.
    pub default: Rule,
    /// Each call approved on its own; no grant covers it.
    #[serde(default)]
    pub once: bool,
    /// The call's fields its rules may match on, such as `branch`.
    #[serde(default)]
    pub fields: Vec<String>,
}

/// What an environment is for this plugin, and the field that picks one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Environments {
    /// Such as "repos" for GitHub, or "sites".
    pub label: String,
    /// The prepared call's field matched against each environment's
    /// patterns, such as `repo`.
    pub field: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SettingKind {
    String,
    Number,
    Boolean,
    Enum,
    Path,
    List,
    /// Kept in the keyring, handed to the host's capabilities only.
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SettingScope {
    #[default]
    Plugin,
    Environment,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct SettingSpec {
    pub key: String,
    pub label: String,
    /// Markdown.
    pub description: String,
    #[serde(rename = "type")]
    pub kind: SettingKind,
    #[serde(default)]
    pub scope: SettingScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub default: Option<serde_json::Value>,
    /// An enum's choices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

impl Eq for SettingSpec {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Send {
    /// `<header>: <scheme> <secret>`, such as `Authorization: Bearer …`.
    Header {
        name: String,
        scheme: Option<String>,
    },
    /// HTTP Basic, the secret as the password of `user`.
    Basic {
        user: String,
    },
    Query {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct CredentialSpec {
    pub id: String,
    pub label: String,
    /// The hosts Silt sends it to, and no others.
    pub hosts: Vec<String>,
    pub send: Send,
    /// Markdown: how a person gets one.
    pub how: String,
    /// Each environment has its own.
    #[serde(default)]
    pub per_environment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct Docs {
    #[serde(default = "readme")]
    pub readme: String,
    /// Folders of skills teaching agents to use it.
    #[serde(default)]
    pub skills: Vec<String>,
}

fn readme() -> String {
    "README.md".into()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("silt-plugin.toml doesn't parse: {0}")]
    Parse(String),
    #[error("silt-plugin.toml's {field}: {why}")]
    Field { field: String, why: String },
}

fn bad(field: impl Into<String>, why: impl Into<String>) -> ManifestError {
    ManifestError::Field { field: field.into(), why: why.into() }
}

fn word(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && text.starts_with(|c: char| c.is_ascii_lowercase())
}

/// A DNS name or a `*.` pattern of one, lowercase, with no scheme, path or
/// port.
pub(crate) fn host(text: &str) -> bool {
    let name = text.strip_prefix("*.").unwrap_or(text);
    !name.is_empty()
        && name.len() <= 253
        && name.contains('.')
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

fn version(text: &str) -> bool {
    let core = text.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: Self = toml::from_str(text).map_err(|error| ManifestError::Parse(error.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        let (publisher, name) =
            self.id.split_once('.').ok_or_else(|| bad("id", "is `publisher.name`, such as `acme.deploys`"))?;
        if !word(publisher) || !word(name) {
            return Err(bad("id", "is lowercase letters, digits, `-` and `_`, starting with a letter, in two parts"));
        }
        if publisher == "silt" && self.build != Build::Shipped {
            return Err(bad("id", "`silt.` is for the plugins shipped with Silt"));
        }
        if publisher != self.publisher {
            return Err(bad("publisher", format!("must be the id's first part, {publisher}")));
        }
        for (field, text) in [("name", &self.name), ("description", &self.description), ("license", &self.license)] {
            if text.trim().is_empty() {
                return Err(bad(field, "is empty"));
            }
        }
        if !version(&self.version) {
            return Err(bad("version", "is `major.minor.patch`, such as `2.1.0`"));
        }
        if !KNOWN_WORLDS.contains(&self.world.as_str()) {
            return Err(bad("world", format!("{} isn't a world this Silt hosts ({})", self.world, KNOWN_WORLDS.join(", "))));
        }
        match &self.build {
            Build::Cargo { package, toolchain } if package.is_empty() || toolchain.is_empty() => {
                return Err(bad("build.cargo", "names a package and a pinned toolchain"));
            }
            Build::Asset { url, sha256 } => {
                if !url.starts_with("https://") {
                    return Err(bad("build.asset.url", "is an https:// URL"));
                }
                if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(bad("build.asset.sha256", "is a SHA-256 in hex"));
                }
            }
            _ => {}
        }
        self.validate_capabilities()?;
        self.validate_actions()?;
        self.validate_settings()?;
        self.validate_credentials()?;
        Ok(())
    }

    fn validate_capabilities(&self) -> Result<(), ManifestError> {
        let mut seen = BTreeSet::new();
        for (index, capability) in self.capabilities.iter().enumerate() {
            let field = format!("capabilities[{index}] ({})", capability.name());
            if !seen.insert(capability.name()) {
                return Err(bad(field, "is declared twice; list every host in one"));
            }
            if capability.reason().trim().is_empty() {
                return Err(bad(field, "needs a reason, shown when it's added"));
            }
            let needs_hosts = matches!(
                capability,
                Capability::Http { .. } | Capability::Git { .. } | Capability::Sockets { .. } | Capability::Ssh { .. }
            );
            if needs_hosts && capability.hosts().is_empty() {
                return Err(bad(field, "names the hosts it reaches"));
            }
            if let Some(host) = capability.hosts().iter().find(|name| !self::host(name)) {
                return Err(bad(field, format!("{host:?} isn't a host name (no scheme, port or path; `*.` for subdomains)")));
            }
            if let Capability::McpProcess { command, .. } = capability
                && command.first().is_none_or(|program| program.trim().is_empty())
            {
                return Err(bad(field, "names the command that starts the MCP server"));
            }
        }
        Ok(())
    }

    fn validate_actions(&self) -> Result<(), ManifestError> {
        let mut ids = BTreeSet::new();
        for (index, action) in self.actions.iter().enumerate() {
            let field = format!("actions[{index}]");
            if !word(&action.id) {
                return Err(bad(format!("{field}.id"), "is lowercase letters, digits, `-` and `_`"));
            }
            if !ids.insert(action.id.as_str()) {
                return Err(bad(format!("{field}.id"), format!("{} is declared twice", action.id)));
            }
            if action.label.trim().is_empty() || action.description.trim().is_empty() {
                return Err(bad(&field, "needs a label and a description"));
            }
            if action.kind == ActionKind::Read && action.once {
                return Err(bad(&field, "a read can't be once only; that's for changes"));
            }
            if let Some(name) = action.fields.iter().find(|name| !word(name)) {
                return Err(bad(format!("{field}.fields"), format!("{name:?} isn't a field name")));
            }
        }
        if let Some(environments) = &self.environments {
            if !word(&environments.field) {
                return Err(bad("environments.field", "is a field name"));
            }
            if let Some(action) = self.actions.iter().find(|action| !action.fields.contains(&environments.field)) {
                return Err(bad(
                    "environments.field",
                    format!(
                        "every action declares {}, the field picking an environment; {} doesn't",
                        environments.field, action.id
                    ),
                ));
            }
        }
        Ok(())
    }

    fn validate_settings(&self) -> Result<(), ManifestError> {
        let mut keys = BTreeSet::new();
        for (index, setting) in self.settings.iter().enumerate() {
            let field = format!("settings[{index}]");
            if !word(&setting.key) || !keys.insert(setting.key.as_str()) {
                return Err(bad(format!("{field}.key"), "is a field name, used once"));
            }
            if setting.label.trim().is_empty() || setting.description.trim().is_empty() {
                return Err(bad(&field, "needs a label and a description"));
            }
            if setting.kind == SettingKind::Enum && setting.options.is_empty() {
                return Err(bad(format!("{field}.options"), "an enum lists its choices"));
            }
            if setting.kind == SettingKind::Secret && setting.default.is_some() {
                return Err(bad(format!("{field}.default"), "a secret has no default"));
            }
            if let Some(default) = &setting.default
                && let Err(why) = crate::settings::fits(setting, default)
            {
                return Err(bad(format!("{field}.default"), why));
            }
        }
        if self.environments.is_none() && self.settings.iter().any(|setting| setting.scope == SettingScope::Environment) {
            return Err(bad("settings", "a setting per environment needs `environments`"));
        }
        Ok(())
    }

    fn validate_credentials(&self) -> Result<(), ManifestError> {
        let reached: Vec<Glob> =
            self.capabilities.iter().flat_map(|capability| capability.hosts()).map(|host| Glob::new(host.clone())).collect();
        let mut ids = BTreeSet::new();
        for (index, credential) in self.credentials.iter().enumerate() {
            let field = format!("credentials[{index}]");
            if !word(&credential.id) || !ids.insert(credential.id.as_str()) {
                return Err(bad(format!("{field}.id"), "is a field name, used once"));
            }
            if credential.hosts.is_empty() {
                return Err(bad(format!("{field}.hosts"), "names the hosts it's sent to"));
            }
            for host in &credential.hosts {
                let declared = reached.iter().any(|pattern| pattern.0 == *host || host_glob(&pattern.0).matches(host));
                if !self::host(host) || !declared {
                    return Err(bad(format!("{field}.hosts"), format!("{host} isn't among the hosts its capabilities reach")));
                }
            }
            if credential.per_environment && self.environments.is_none() {
                return Err(bad(&field, "a credential per environment needs `environments`"));
            }
        }
        Ok(())
    }

    /// Every host it may reach, through any capability.
    pub fn hosts(&self) -> Vec<&str> {
        let mut hosts: Vec<&str> =
            self.capabilities.iter().flat_map(|capability| capability.hosts()).map(String::as_str).collect();
        hosts.sort_unstable();
        hosts.dedup();
        hosts
    }

    pub fn capability(&self, name: &str) -> Option<&Capability> {
        self.capabilities.iter().find(|capability| capability.name() == name)
    }

    pub fn action(&self, id: &str) -> Option<&ActionSpec> {
        self.actions.iter().find(|action| action.id == id)
    }

    /// What `newer` asks for that this didn't: shown before an update.
    pub fn added_in(&self, newer: &Self) -> Vec<String> {
        let mut added = Vec::new();
        for capability in &newer.capabilities {
            match self.capability(capability.name()) {
                None => added.push(format!("the {} capability: {}", capability.name(), capability.reason())),
                Some(old) => {
                    for host in capability.hosts().iter().filter(|host| !old.hosts().contains(host)) {
                        added.push(format!("{} reaches a new host, {host}", capability.name()));
                    }
                    if let (Capability::McpProcess { command: before, .. }, Capability::McpProcess { command: after, .. }) =
                        (old, capability)
                        && before != after
                    {
                        added.push(format!("its MCP server is now `{}`", after.join(" ")));
                    }
                }
            }
        }
        for action in newer.actions.iter().filter(|action| self.action(&action.id).is_none()) {
            added.push(format!(
                "a new action, {} ({})",
                action.label,
                if action.kind == ActionKind::Read { "a read" } else { "a change" }
            ));
        }
        for credential in
            newer.credentials.iter().filter(|credential| !self.credentials.iter().any(|old| old.id == credential.id))
        {
            added.push(format!("a new credential, {}", credential.label));
        }
        added
    }
}

/// `*.acme.dev` as a glob over host names: any subdomain, at any depth.
pub(crate) fn host_glob(pattern: &str) -> Glob {
    match pattern.strip_prefix("*.") {
        Some(domain) => Glob::new(format!("**.{domain}")),
        None => Glob::new(pattern),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const DEPLOYS: &str = r#"
id = "acme.deploys"
name = "acme deploys"
version = "2.1.0"
publisher = "acme"
description = "Roll out, pause and roll back acme's services."
license = "Apache-2.0"
repository = "https://github.com/acme/silt-deploys"
world = "silt:plugin@1"
build = { cargo = { package = "silt-deploys", toolchain = "1.99.1" } }

[[capabilities]]
kind = "http"
hosts = ["deploy.acme.internal"]
reason = "Reads and changes rollouts."

[[capabilities]]
kind = "thread-read"
reason = "Reads the release notes in the thread's checkout."

[[actions]]
id = "status"
label = "See a service's status"
description = "The versions running, and any rollout."
kind = "read"
default = "allow"
fields = ["environment", "service"]

[[actions]]
id = "rollout"
label = "Roll out"
description = "Starts a rollout of a version, in canary steps."
kind = "change"
default = "ask"
once = true
fields = ["environment", "service"]

[environments]
label = "clusters"
field = "environment"

[[settings]]
key = "default_canary"
label = "First step"
description = "The percentage a rollout starts at."
type = "number"
default = 10

[[settings]]
key = "token"
label = "Deploy token"
description = "From the deploy console."
type = "secret"
scope = "environment"

[[credentials]]
id = "token"
label = "Deploy token"
hosts = ["deploy.acme.internal"]
send = { header = { name = "Authorization", scheme = "Bearer" } }
how = "Create one in the deploy console."
per_environment = true
"#;

    #[test]
    fn a_full_manifest_parses() {
        let manifest = Manifest::parse(DEPLOYS).unwrap();
        assert_eq!(manifest.hosts(), ["deploy.acme.internal"]);
        assert!(manifest.action("rollout").is_some_and(|action| action.once));
        assert_eq!(manifest.capability("thread-read").map(Capability::opaque), Some(false));
    }

    fn broken(from: &str, to: &str) -> ManifestError {
        assert!(DEPLOYS.contains(from), "{from}");
        Manifest::parse(&DEPLOYS.replacen(from, to, 1)).unwrap_err()
    }

    fn field_of(error: ManifestError) -> String {
        match error {
            ManifestError::Field { field, .. } => field,
            ManifestError::Parse(why) => panic!("parse: {why}"),
        }
    }

    #[test]
    fn each_mistake_names_its_field() {
        assert_eq!(field_of(broken("id = \"acme.deploys\"", "id = \"Acme\"")), "id");
        assert_eq!(field_of(broken("publisher = \"acme\"", "publisher = \"other\"")), "publisher");
        assert_eq!(field_of(broken("version = \"2.1.0\"", "version = \"2\"")), "version");
        assert_eq!(field_of(broken("world = \"silt:plugin@1\"", "world = \"silt:plugin@9\"")), "world");
        assert_eq!(
            field_of(broken("hosts = [\"deploy.acme.internal\"]\nreason", "hosts = [\"https://x.dev\"]\nreason")),
            "capabilities[0] (http)"
        );
        assert_eq!(field_of(broken("reason = \"Reads and changes rollouts.\"", "reason = \" \"")), "capabilities[0] (http)");
        assert_eq!(field_of(broken("id = \"rollout\"", "id = \"status\"")), "actions[1].id");
        assert_eq!(
            field_of(broken(
                "fields = [\"environment\", \"service\"]\n\n[environments]",
                "fields = [\"service\"]\n\n[environments]"
            )),
            "environments.field"
        );
        assert_eq!(field_of(broken("default = 10", "default = \"ten\"")), "settings[0].default");
        assert_eq!(
            field_of(broken("hosts = [\"deploy.acme.internal\"]\nsend", "hosts = [\"evil.example.com\"]\nsend")),
            "credentials[0].hosts"
        );
        assert_eq!(
            field_of(broken("kind = \"read\"\ndefault = \"allow\"", "kind = \"read\"\ndefault = \"allow\"\nonce = true")),
            "actions[0]"
        );
        assert_eq!(field_of(broken("id = \"acme.deploys\"", "id = \"silt.deploys\"")), "id");
        assert!(matches!(broken("license = \"Apache-2.0\"", "license = \"Apache-2.0\"\nsurprise = 1"), ManifestError::Parse(_)));
    }

    #[test]
    fn an_update_says_what_it_adds() {
        let old = Manifest::parse(DEPLOYS).unwrap();
        let mut new = old.clone();
        new.capabilities[0] =
            Capability::Http { hosts: vec!["deploy.acme.internal".into(), "uploads.acme.internal".into()], reason: "x".into() };
        new.capabilities.push(Capability::AgentFiles { reason: "Runbooks for agents.".into() });
        new.actions.push(ActionSpec {
            id: "abort".into(),
            label: "Abort a rollout".into(),
            description: "d".into(),
            kind: ActionKind::Change,
            default: Rule::Ask,
            once: false,
            fields: vec!["environment".into()],
        });
        let added = old.added_in(&new);
        assert_eq!(added.len(), 3, "{added:?}");
        assert!(added[0].contains("uploads.acme.internal"));
        assert!(added[1].contains("agent-files"));
        assert!(added[2].contains("Abort a rollout"));
        assert!(old.added_in(&old).is_empty());
    }

    #[test]
    fn hosts_are_names_or_subdomain_patterns() {
        assert!(host("api.github.com"));
        assert!(host("*.acme.dev"));
        assert!(!host("localhost"), "a bare name could be anything on the network");
        assert!(!host("API.github.com"));
        assert!(!host("github.com:443"));
        assert!(!host("*"));
        assert!(host_glob("*.acme.dev").matches("a.b.acme.dev"));
        assert!(!host_glob("*.acme.dev").matches("acme.dev.evil.com"));
    }
}
