use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Preset;
use crate::reach::{Backend, Devices, Extra, Network, both, lower};

/// An agent as rules name it: the same for its CLI and its ACP adapter
/// (`claude`, `codex`, `copilot`, …).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "string"))]
pub struct AgentKey(String);

impl Serialize for AgentKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AgentKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

impl AgentKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into().to_ascii_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The agents rules know by name, each with its CLI's and its ACP
/// adapter's ids, and how people call it.
pub const KNOWN_AGENTS: [(&str, &[&str], &str); 8] = [
    ("claude", &["claude-code", "claude-acp"], "Claude Code"),
    ("codex", &["codex", "codex-acp"], "Codex"),
    ("qudiks", &["qudiks", "qudiks-acp"], "Qudiks"),
    ("copilot", &["copilot-cli", "github-copilot-cli"], "GitHub Copilot"),
    ("gemini", &["gemini-cli", "gemini"], "Gemini CLI"),
    ("agy", &["antigravity-cli", "antigravity-acp"], "Antigravity"),
    ("opencode", &["opencode", "opencode-v2"], "OpenCode"),
    ("local", &["local"], "Silt on a local model"),
];

/// The key rules name an agent by, from its CLI adapter's or ACP registry
/// id; any other registry agent goes by its id.
pub fn agent_key(id: &str) -> AgentKey {
    KNOWN_AGENTS.iter().find(|(_, ids, _)| ids.contains(&id)).map_or_else(|| AgentKey::new(id), |(key, _, _)| AgentKey::new(*key))
}

/// The agent a start asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentChoice {
    Known {
        key: AgentKey,
        name: String,
    },
    /// One of the person's own, from `agents.json`.
    Custom {
        name: String,
    },
}

impl AgentChoice {
    fn name(&self) -> &str {
        match self {
            Self::Known { name, .. } | Self::Custom { name } => name,
        }
    }
}

/// What an agent may do in a workspace. A rule left unset restricts
/// nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Policy {
    /// The agents allowed; any when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<BTreeSet<AgentKey>>,
    /// Whether `agents.json`'s agents are allowed; they are when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_agents: Option<bool>,
    /// The most an agent may do without asking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_preset: Option<Preset>,
    /// The agent runs only with its own sandbox on, which Silt's sandbox
    /// also answers.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sandbox: bool,
    /// Agents run in Silt's own sandbox, and don't start where it can't be
    /// made (ADR 0093). The reach rules below apply only with it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub silt_sandbox: bool,
    /// What makes Silt's sandbox; bubblewrap when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<Backend>,
    /// "Hide other workspaces' files": the sandbox also keeps the agent out
    /// of Silt's data and other workspaces' folders; implies `sandbox`.
    /// History tools are scoped to their workspace without it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub isolated: bool,
    /// Projects keep the workspace's server: no other, and not this machine
    /// only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub home_locked: bool,
    /// Nothing Silt holds of its threads leaves this machine, and only
    /// local models answer (ADR 0092): no server, `local`, and Claude Code
    /// on a local model only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local_only: bool,
    /// In a Local only workspace, agents' history tools read the whole
    /// workspace rather than their own project.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wide_history: bool,
    /// The page connectors its artifacts' pages may call, by name; any
    /// when unset (ADR 0080).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connectors: Option<BTreeSet<String>>,
    /// The hosts a sandbox reaches "through Silt only"; any public host
    /// when unset (ADR 0093).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosts: Option<BTreeSet<String>>,
    /// What the sandbox can connect to: the person's choice, or an org's
    /// most; full internet when unset (ADR 0093).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
    /// Which devices the sandbox gets: the person's choice, or an org's most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devices: Option<Devices>,
    /// The device nodes [`Devices::Named`] gives.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub device_nodes: BTreeSet<String>,
    /// Address ranges allowed past those always blocked, such as a VPN's.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub ranges: BTreeSet<String>,
    /// Ports on this machine the sandbox reaches as its own `localhost`.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub ports: BTreeSet<u16>,
    /// Extra folders for the sandbox, each writable or not.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub paths: BTreeMap<String, bool>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gpu: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub docker: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ssh_agent: bool,
    /// What of the machine may never be handed to the sandbox.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub refuse: BTreeSet<Extra>,
    /// The agent's own sandbox may not run inside Silt's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_nested: bool,
    /// The folders agents may ask for, and how each is answered (ADR 0098).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<crate::FolderRule>,
    /// The folder rules of more than one policy, each set judged on its
    /// own, once policies are combined.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folder_layers: Vec<Vec<crate::FolderRule>>,
}

/// Which of a workspace's sandbox rules Silt can hold an agent to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SandboxSupport {
    None,
    /// Its own sandbox, turned on per run.
    Sandbox,
    /// Its sandbox, and paths it denies reading.
    Isolate,
}

/// The agents that run on local models: Silt's own, and Claude Code, which
/// must then be on one.
pub const LOCAL_AGENTS: [&str; 2] = ["local", "claude"];

impl Policy {
    pub fn sandbox_required(&self) -> bool {
        self.sandbox || self.isolated
    }

    /// Whether an agent with `support` can be held to these sandbox rules.
    pub fn sandboxes(&self, support: SandboxSupport) -> bool {
        match support {
            SandboxSupport::Isolate => true,
            SandboxSupport::Sandbox => !self.isolated,
            SandboxSupport::None => !self.sandbox_required(),
        }
    }

    pub fn allows_full_access(&self) -> bool {
        self.max_preset.is_none_or(|cap| cap == Preset::FullAccess)
    }

    /// Both policies at once: each rule as the stricter of the two sets it.
    pub fn stricter(&self, other: &Self) -> Self {
        let (folders, folder_layers) = self.both_folder_rules(other);
        Self {
            agents: match (&self.agents, &other.agents) {
                (Some(mine), Some(theirs)) => Some(mine.intersection(theirs).cloned().collect()),
                (one, other) => one.clone().or_else(|| other.clone()),
            },
            custom_agents: match (self.custom_agents, other.custom_agents) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (one, other) => one.or(other),
            },
            max_preset: match (self.max_preset, other.max_preset) {
                (Some(mine), Some(theirs)) => Some(mine.stricter(theirs)),
                (one, other) => one.or(other),
            },
            sandbox: self.sandbox || other.sandbox,
            silt_sandbox: self.silt_sandbox || other.silt_sandbox,
            // An org's choice holds on its members' machines.
            backend: other.backend.or(self.backend),
            isolated: self.isolated || other.isolated,
            home_locked: self.home_locked || other.home_locked,
            local_only: self.local_only || other.local_only,
            wide_history: self.wide_history && other.wide_history,
            connectors: match (&self.connectors, &other.connectors) {
                (Some(mine), Some(theirs)) => Some(mine.intersection(theirs).cloned().collect()),
                (one, other) => one.clone().or_else(|| other.clone()),
            },
            hosts: match (&self.hosts, &other.hosts) {
                (Some(mine), Some(theirs)) => Some(mine.intersection(theirs).cloned().collect()),
                (one, other) => one.clone().or_else(|| other.clone()),
            },
            network: lower(self.network, other.network),
            devices: lower(self.devices, other.devices),
            // What the person hands over is theirs to list; a rule can only
            // refuse it, through `refuse` and the caps above.
            device_nodes: both(&self.device_nodes, &other.device_nodes),
            ranges: both(&self.ranges, &other.ranges),
            ports: both(&self.ports, &other.ports),
            paths: other.paths.iter().chain(&self.paths).map(|(path, writable)| (path.clone(), *writable)).collect(),
            gpu: self.gpu || other.gpu,
            docker: self.docker || other.docker,
            ssh_agent: self.ssh_agent || other.ssh_agent,
            refuse: both(&self.refuse, &other.refuse),
            no_nested: self.no_nested || other.no_nested,
            folders,
            folder_layers,
        }
    }

    /// An org's policy as it applies: a workspace bound to its server can't
    /// be Local only, so that rule is the person's alone.
    pub fn without_local_only(&self) -> Self {
        Self { local_only: false, wide_history: false, ..self.clone() }
    }

    /// Whether agents' history tools read only their own project.
    pub fn history_by_project(&self) -> bool {
        self.local_only && !self.wide_history
    }

    /// Whether projects keep the workspace's home, a Local only one's being
    /// this machine.
    pub fn keeps_home(&self) -> bool {
        self.home_locked || self.local_only
    }
}

/// Where a rule came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Source {
    /// The person's own, on this machine.
    Local,
    /// An org's policy on its server.
    Org { org: String, server: String },
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => f.write_str("your rules"),
            Self::Org { org, server } => write!(f, "{org}'s policy on {}", host(server)),
        }
    }
}

impl Source {
    fn allows(&self) -> &'static str {
        match self {
            Self::Local => "allow",
            Self::Org { .. } => "allows",
        }
    }
}

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

/// An org's policy, as a machine last fetched it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OrgPolicy {
    pub org: String,
    pub server: String,
    pub policy: Policy,
}

/// What an agent's mode means for its permissions (ADR 0048, ADR 0049).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeMeaning {
    /// The mode answers as this preset would.
    Preset(Preset),
    /// The agent's modes say how it works, not what it asks.
    NotPermissions,
    /// A mode of an agent whose modes are its permissions, which Silt doesn't
    /// know.
    Unknown,
}

/// Which rule refused, and what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Broken {
    Agent {
        agent: String,
        allowed: Vec<AgentKey>,
    },
    CustomAgent {
        agent: String,
    },
    Preset {
        asked: Preset,
        cap: Preset,
    },
    Mode {
        mode: String,
        cap: Preset,
    },
    Flag {
        flag: String,
        agent: String,
        cap: Preset,
    },
    Sandbox {
        agent: String,
        hides_others: bool,
    },
    /// Silt's sandbox is required and this machine can't make one.
    SiltSandbox {
        why: String,
    },
    Home,
    /// An agent that doesn't run on a local model, in a Local only workspace.
    NotLocal {
        agent: String,
    },
    /// Claude Code on a model other than a local one there.
    OnlineModel {
        agent: String,
    },
    Move {
        project: String,
        servers: Vec<String>,
        to: Option<String>,
    },
    WaitingForPolicy,
    Connector {
        connector: String,
        allowed: Vec<String>,
    },
}

/// A start or a change the rules refuse, saying which rule and whose.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.message())]
pub struct Refusal {
    pub workspace: String,
    pub broken: Box<Broken>,
    pub origin: Source,
}

impl Refusal {
    pub fn message(&self) -> String {
        let Self { workspace, broken, origin: source } = self;
        let allows = source.allows();
        match broken.as_ref() {
            Broken::Agent { agent, allowed } => {
                let allowed: Vec<&str> = allowed.iter().map(AgentKey::as_str).collect();
                if allowed.is_empty() {
                    format!("{agent} isn't allowed in {workspace}: {source} {allows} no agent")
                } else {
                    format!("{agent} isn't allowed in {workspace}: {source} {allows} {}", allowed.join(", "))
                }
            }
            Broken::CustomAgent { agent } => format!("{agent} isn't allowed in {workspace}: {source} {allows} no custom agents"),
            Broken::Preset { asked, cap } => {
                format!("{} isn't allowed in {workspace}: {source} {allows} at most {}", asked.label(), cap.label())
            }
            Broken::Mode { mode, cap } => {
                format!("the {mode} mode isn't allowed in {workspace}: {source} {allows} at most {}", cap.label())
            }
            Broken::Flag { flag, agent, cap } => format!(
                "{flag} stops {agent} asking before it acts, which {workspace} doesn't allow: {source} {allows} at most {}",
                cap.label()
            ),
            Broken::Sandbox { agent, hides_others: true } => format!(
                "{workspace} hides other workspaces' files from its agents, which needs a sandbox Silt can't set up for {agent} ({source})"
            ),
            Broken::Sandbox { agent, hides_others: false } => {
                format!("{workspace} needs the agent's own sandbox, which Silt can't turn on for {agent} ({source})")
            }
            Broken::SiltSandbox { why } => {
                format!("{workspace} runs its agents in Silt's sandbox, which this machine can't make ({source}): {why}")
            }
            Broken::Home => format!("{workspace}'s projects sync with its server only ({source})"),
            Broken::NotLocal { agent } => {
                format!(
                    "{workspace} is local only: {agent} doesn't run on a local model; Silt (local model) and Claude Code on one do"
                )
            }
            Broken::OnlineModel { agent } => {
                format!("{workspace} is local only: choose a local model for {agent}, such as local:<name>")
            }
            Broken::Move { project, servers, to } => {
                let to =
                    to.as_deref().map_or("keeps its threads on this machine".to_owned(), |to| format!("syncs with {}", host(to)));
                let servers: Vec<&str> = servers.iter().map(|server| host(server)).collect();
                format!(
                    "{project} can't move to {workspace}: its threads sync with {}, and {workspace} {to}; threads never move between servers",
                    servers.join(", ")
                )
            }
            Broken::WaitingForPolicy => format!("{workspace} is waiting for {source}; signing in to it fetches it"),
            Broken::Connector { connector, allowed } if allowed.is_empty() => {
                format!("pages in {workspace} may not call {connector}: {source} {allows} no connector")
            }
            Broken::Connector { connector, allowed } => {
                format!("pages in {workspace} may not call {connector}: {source} {allows} {}", allowed.join(", "))
            }
        }
    }
}

/// The rules of one workspace: the person's own and its org's, which apply
/// together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules<'a> {
    pub workspace: &'a str,
    pub local: &'a Policy,
    /// The org's policy, when the workspace is bound to an org: `Err` until
    /// a machine has fetched it once.
    pub org: Option<Result<&'a OrgPolicy, Source>>,
}

impl Rules<'_> {
    /// The rules in effect, both at once.
    pub fn effective(&self) -> Policy {
        match &self.org {
            Some(Ok(org)) => self.local.stricter(&org.policy.without_local_only()),
            _ => self.local.clone(),
        }
    }

    fn each(&self) -> Result<Vec<(Policy, Source)>, Refusal> {
        let mut all = vec![(self.local.clone(), Source::Local)];
        match &self.org {
            Some(Ok(org)) => {
                all.push((org.policy.without_local_only(), Source::Org { org: org.org.clone(), server: org.server.clone() }));
            }
            Some(Err(source)) => return Err(self.refusal(Broken::WaitingForPolicy, source.clone())),
            None => {}
        }
        Ok(all)
    }

    fn refusal(&self, broken: Broken, source: Source) -> Refusal {
        Refusal { workspace: self.workspace.to_owned(), broken: Box::new(broken), origin: source }
    }

    /// The org's rules are checked first, so a refusal names them when both
    /// would refuse.
    fn check(&self, broken: impl Fn(&Policy) -> Option<Broken>) -> Result<(), Refusal> {
        for (policy, source) in self.each()?.into_iter().rev() {
            if let Some(broken) = broken(&policy) {
                return Err(self.refusal(broken, source));
            }
        }
        Ok(())
    }

    /// Whether this agent may start here, sandboxed if the rules say so.
    pub fn check_agent(&self, agent: &AgentChoice, support: SandboxSupport) -> Result<(), Refusal> {
        self.check(|policy| {
            let local = matches!(agent, AgentChoice::Known { key, .. } if LOCAL_AGENTS.contains(&key.as_str()));
            (policy.local_only && !local).then(|| Broken::NotLocal { agent: agent.name().to_owned() })
        })?;
        self.check(|policy| match agent {
            AgentChoice::Known { key, name } => policy
                .agents
                .as_ref()
                .filter(|allowed| !allowed.contains(key))
                .map(|allowed| Broken::Agent { agent: name.clone(), allowed: allowed.iter().cloned().collect() }),
            // A custom agent is never one of those a list allows.
            AgentChoice::Custom { name } => match (&policy.agents, policy.custom_agents) {
                (_, Some(false)) => Some(Broken::CustomAgent { agent: name.clone() }),
                (Some(allowed), _) => Some(Broken::Agent { agent: name.clone(), allowed: allowed.iter().cloned().collect() }),
                (None, _) => None,
            },
        })?;
        self.check(|policy| {
            (!policy.sandboxes(support))
                .then(|| Broken::Sandbox { agent: agent.name().to_owned(), hides_others: policy.isolated })
        })
    }

    /// Whether an agent may start here when this machine can make Silt's
    /// sandbox or, with `unavailable`, says why it can't.
    pub fn check_silt_sandbox(&self, unavailable: Option<&str>) -> Result<(), Refusal> {
        self.check(|policy| {
            let why = unavailable.filter(|_| policy.silt_sandbox)?;
            Some(Broken::SiltSandbox { why: why.to_owned() })
        })
    }

    /// Whether Claude Code may run on the model it's given: a local one,
    /// where the workspace is Local only.
    pub fn check_model(&self, agent: &AgentChoice, on_local_model: bool) -> Result<(), Refusal> {
        let is_claude = matches!(agent, AgentChoice::Known { key, .. } if key.as_str() == "claude");
        self.check(|policy| {
            (policy.local_only && is_claude && !on_local_model).then(|| Broken::OnlineModel { agent: agent.name().to_owned() })
        })
    }

    pub fn check_preset(&self, preset: Preset) -> Result<(), Refusal> {
        self.check(|policy| policy.max_preset.filter(|cap| !preset.within(*cap)).map(|cap| Broken::Preset { asked: preset, cap }))
    }

    pub fn check_mode(&self, mode: &str, meaning: ModeMeaning) -> Result<(), Refusal> {
        self.check(|policy| {
            let cap = policy.max_preset?;
            let refused = match meaning {
                ModeMeaning::Preset(preset) => !preset.within(cap),
                ModeMeaning::NotPermissions => false,
                ModeMeaning::Unknown => cap != Preset::FullAccess,
            };
            refused.then(|| Broken::Mode { mode: mode.to_owned(), cap })
        })
    }

    /// `flag` is one the agent's adapter knows to stop it asking.
    pub fn check_flag(&self, flag: &str, agent: &str) -> Result<(), Refusal> {
        self.check(|policy| {
            policy.max_preset.filter(|cap| *cap != Preset::FullAccess).map(|cap| Broken::Flag {
                flag: flag.to_owned(),
                agent: agent.to_owned(),
                cap,
            })
        })
    }

    /// Whether a page here may call page connector `connector`.
    pub fn check_connector(&self, connector: &str) -> Result<(), Refusal> {
        self.check(|policy| {
            policy
                .connectors
                .as_ref()
                .filter(|allowed| !allowed.contains(connector))
                .map(|allowed| Broken::Connector { connector: connector.to_owned(), allowed: allowed.iter().cloned().collect() })
        })
    }

    /// Whether a project's home may be set apart from the workspace's server.
    pub fn check_home(&self) -> Result<(), Refusal> {
        self.check(|policy| policy.keeps_home().then_some(Broken::Home))
    }
}

/// Whether a project whose threads already sync with `servers` may move to
/// a workspace syncing with `to`: threads never move between servers
/// (ADR 0032).
pub fn check_move(project: &str, servers: &[String], workspace: &str, to: Option<&str>) -> Result<(), Refusal> {
    let elsewhere: Vec<String> = servers.iter().filter(|server| Some(server.as_str()) != to).cloned().collect();
    if elsewhere.is_empty() {
        return Ok(());
    }
    Err(Refusal {
        workspace: workspace.to_owned(),
        broken: Box::new(Broken::Move { project: project.to_owned(), servers: elsewhere, to: to.map(str::to_owned) }),
        origin: Source::Local,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_or_its_org_may_name_the_connectors_its_pages_call() {
        let local =
            Policy { connectors: Some(BTreeSet::from(["tracker".to_owned(), "calendar".to_owned()])), ..Policy::default() };
        let org = OrgPolicy {
            org: "acme".into(),
            server: "https://silt.example".into(),
            policy: Policy { connectors: Some(BTreeSet::from(["tracker".to_owned()])), ..Policy::default() },
        };
        let rules = Rules { workspace: "Work", local: &local, org: Some(Ok(&org)) };
        assert!(rules.check_connector("tracker").is_ok());
        let refused = rules.check_connector("calendar").expect_err("the org's list leaves it out");
        assert!(
            refused.message().contains("may not call calendar") && refused.message().contains("tracker"),
            "{}",
            refused.message()
        );
        assert_eq!(rules.effective().connectors, Some(BTreeSet::from(["tracker".to_owned()])));
        let open = Rules { workspace: "Personal", local: &Policy::default(), org: None };
        assert!(open.check_connector("anything").is_ok(), "unset allows any");
    }

    fn keys(keys: &[&str]) -> Option<BTreeSet<AgentKey>> {
        Some(keys.iter().map(|key| AgentKey::new(*key)).collect())
    }

    fn known(key: &str, name: &str) -> AgentChoice {
        AgentChoice::Known { key: AgentKey::new(key), name: name.into() }
    }

    fn acme(policy: Policy) -> OrgPolicy {
        OrgPolicy { org: "acme".into(), server: "https://silt.acme.dev".into(), policy }
    }

    #[test]
    fn a_local_only_workspace_runs_only_local_models() {
        let policy = Policy { local_only: true, ..Policy::default() };
        let rules = Rules { workspace: "Logs", local: &policy, org: None };
        assert!(rules.check_agent(&known("local", "Silt (local model)"), SandboxSupport::None).is_ok());
        assert!(rules.check_agent(&known("claude", "Claude Code"), SandboxSupport::None).is_ok());
        let codex = rules.check_agent(&known("codex", "Codex"), SandboxSupport::None).expect_err("refused");
        assert!(codex.to_string().contains("Codex doesn't run on a local model"), "{codex}");
        let custom = rules.check_agent(&AgentChoice::Custom { name: "Mine".into() }, SandboxSupport::None);
        assert!(custom.is_err(), "a custom agent may reach any model");
        assert!(rules.check_model(&known("claude", "Claude Code"), true).is_ok());
        let online = rules.check_model(&known("claude", "Claude Code"), false).expect_err("refused");
        assert!(online.to_string().contains("choose a local model for Claude Code"), "{online}");
        assert!(rules.check_model(&known("local", "Silt (local model)"), false).is_ok(), "only Claude's model is checked");
        assert!(rules.check_home().is_err(), "its home is this machine");
        assert!(policy.stricter(&Policy::default()).local_only);
        let anywhere = Rules { workspace: "Default", local: &Policy::default(), org: None };
        assert!(anywhere.check_model(&known("claude", "Claude Code"), false).is_ok());
    }

    #[test]
    fn stricter_takes_the_stricter_of_each_rule() {
        let mine = Policy {
            agents: keys(&["claude", "copilot"]),
            max_preset: Some(Preset::FullAccess),
            sandbox: true,
            ..Policy::default()
        };
        let theirs = Policy {
            agents: keys(&["copilot", "codex"]),
            custom_agents: Some(false),
            max_preset: Some(Preset::AskBeforeCommands),
            home_locked: true,
            ..Policy::default()
        };
        let both = mine.stricter(&theirs);
        assert_eq!(both.agents, keys(&["copilot"]));
        assert_eq!(both.custom_agents, Some(false));
        assert_eq!(both.max_preset, Some(Preset::AskBeforeCommands));
        assert!(both.sandbox && both.home_locked && !both.isolated);
        assert_eq!(both, theirs.stricter(&mine));
        assert_eq!(Policy::default().stricter(&theirs), theirs);
    }

    #[test]
    fn the_org_policy_refuses_and_says_so() {
        let local = Policy { isolated: true, ..Policy::default() };
        let org = acme(Policy { agents: keys(&["copilot"]), max_preset: Some(Preset::AskBeforeCommands), ..Policy::default() });
        let rules = Rules { workspace: "Work", local: &local, org: Some(Ok(&org)) };

        let refusal = rules.check_agent(&known("claude", "Claude Code"), SandboxSupport::Isolate).unwrap_err();
        assert_eq!(refusal.to_string(), "Claude Code isn't allowed in Work: acme's policy on silt.acme.dev allows copilot");
        assert_eq!(rules.check_agent(&known("copilot", "GitHub Copilot"), SandboxSupport::Isolate), Ok(()));
        // Isolated is the person's own rule, and Copilot can't sandbox here.
        let refusal = rules.check_agent(&known("copilot", "GitHub Copilot"), SandboxSupport::None).unwrap_err();
        assert_eq!(refusal.origin, Source::Local);
        assert_eq!(
            refusal.to_string(),
            "Work hides other workspaces' files from its agents, which needs a sandbox Silt can't set up for GitHub Copilot (your rules)"
        );

        assert_eq!(rules.check_preset(Preset::Plan), Ok(()));
        assert_eq!(
            rules.check_preset(Preset::FullAccess).unwrap_err().to_string(),
            "Full access isn't allowed in Work: acme's policy on silt.acme.dev allows at most Ask before commands"
        );
        assert_eq!(
            rules.check_flag("--yolo", "Copilot").unwrap_err().to_string(),
            "--yolo stops Copilot asking before it acts, which Work doesn't allow: acme's policy on silt.acme.dev allows at most Ask before commands"
        );
        assert_eq!(rules.effective().agents, keys(&["copilot"]));
        assert!(rules.effective().isolated);
    }

    #[test]
    fn a_sandbox_refusal_names_the_rule_the_agent_cannot_meet() {
        let local = Policy { isolated: true, ..Policy::default() };
        let org = acme(Policy { sandbox: true, ..Policy::default() });
        let rules = Rules { workspace: "Work", local: &local, org: Some(Ok(&org)) };
        let refusal = rules.check_agent(&known("codex", "Codex"), SandboxSupport::Sandbox).unwrap_err();
        assert_eq!(refusal.origin, Source::Local, "Codex can sandbox, as the org asks, but not hide other workspaces");
        assert_eq!(rules.check_agent(&known("codex", "Codex"), SandboxSupport::Isolate), Ok(()));
        let refusal = rules.check_agent(&known("copilot", "GitHub Copilot"), SandboxSupport::None).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "Work needs the agent's own sandbox, which Silt can't turn on for GitHub Copilot (acme's policy on silt.acme.dev)"
        );

        let local = Policy { sandbox: true, ..Policy::default() };
        let org = acme(Policy { isolated: true, ..Policy::default() });
        let rules = Rules { workspace: "Work", local: &local, org: Some(Ok(&org)) };
        let refusal = rules.check_agent(&known("codex", "Codex"), SandboxSupport::Sandbox).unwrap_err();
        assert!(matches!(refusal.origin, Source::Org { .. }));
    }

    #[test]
    fn silt_s_sandbox_is_its_own_rule_and_nothing_starts_where_it_cant_be_made() {
        let why = "no bubblewrap";
        let agents_own = Policy { sandbox: true, isolated: true, ..Policy::default() };
        let rules = Rules { workspace: "Work", local: &agents_own, org: None };
        assert_eq!(rules.check_silt_sandbox(Some(why)), Ok(()), "the agent's own sandbox doesn't ask for Silt's");

        let org = acme(Policy { silt_sandbox: true, ..Policy::default() });
        let rules = Rules { workspace: "Work", local: &agents_own, org: Some(Ok(&org)) };
        assert_eq!(rules.check_silt_sandbox(None), Ok(()));
        let refusal = rules.check_silt_sandbox(Some(why)).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "Work runs its agents in Silt's sandbox, which this machine can't make (acme's policy on silt.acme.dev): no bubblewrap"
        );
        assert!(rules.effective().silt_sandbox);
    }

    #[test]
    fn modes_are_capped_by_the_preset_they_mean() {
        let local = Policy { max_preset: Some(Preset::AskBeforeCommands), ..Policy::default() };
        let rules = Rules { workspace: "Work", local: &local, org: None };
        let cases = [
            ("acceptEdits", ModeMeaning::Preset(Preset::AskBeforeCommands), true),
            ("bypassPermissions", ModeMeaning::Preset(Preset::FullAccess), false),
            ("plan", ModeMeaning::Preset(Preset::Plan), true),
            ("agent", ModeMeaning::NotPermissions, true),
            ("whatever-new", ModeMeaning::Unknown, false),
        ];
        for (mode, meaning, allowed) in cases {
            assert_eq!(rules.check_mode(mode, meaning).is_ok(), allowed, "{mode}");
        }
        let open = Policy::default();
        let rules = Rules { workspace: "Personal", local: &open, org: None };
        assert_eq!(rules.check_mode("whatever-new", ModeMeaning::Unknown), Ok(()));
        assert_eq!(rules.check_flag("--dangerously-skip-permissions", "Claude Code"), Ok(()));
    }

    #[test]
    fn custom_agents_and_the_home() {
        let local = Policy { custom_agents: Some(false), home_locked: true, ..Policy::default() };
        let rules = Rules { workspace: "Work", local: &local, org: None };
        let refusal = rules.check_agent(&AgentChoice::Custom { name: "My agent".into() }, SandboxSupport::Isolate).unwrap_err();
        assert_eq!(refusal.to_string(), "My agent isn't allowed in Work: your rules allow no custom agents");
        assert_eq!(rules.check_home().unwrap_err().to_string(), "Work's projects sync with its server only (your rules)");
        let listed = Policy { agents: keys(&["copilot"]), ..Policy::default() };
        let rules = Rules { workspace: "Work", local: &listed, org: None };
        let refusal = rules.check_agent(&AgentChoice::Custom { name: "My agent".into() }, SandboxSupport::Isolate).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "My agent isn't allowed in Work: your rules allow copilot",
            "a list names no custom agent"
        );
    }

    #[test]
    fn nothing_starts_before_the_org_policy_arrives() {
        let local = Policy::default();
        let waiting = Source::Org { org: "acme".into(), server: "https://silt.acme.dev".into() };
        let rules = Rules { workspace: "Work", local: &local, org: Some(Err(waiting)) };
        assert_eq!(
            rules.check_preset(Preset::AskForEverything).unwrap_err().to_string(),
            "Work is waiting for acme's policy on silt.acme.dev; signing in to it fetches it"
        );
    }

    #[test]
    fn projects_move_only_within_their_server() {
        let acme = Some("https://silt.acme.dev");
        assert_eq!(check_move("silt", &[], "Work", acme), Ok(()));
        assert_eq!(check_move("billing", &["https://silt.acme.dev".into()], "Work", acme), Ok(()));
        assert_eq!(
            check_move("silt", &["https://silt.example.dev".into()], "Work", acme).unwrap_err().to_string(),
            "silt can't move to Work: its threads sync with silt.example.dev, and Work syncs with silt.acme.dev; threads never move between servers"
        );
        assert!(check_move("silt", &["https://silt.example.dev".into()], "Experiments", None).is_err());
    }

    #[test]
    fn cli_and_acp_ids_share_a_key() {
        assert_eq!(agent_key("claude-code"), agent_key("claude-acp"));
        assert_eq!(agent_key("github-copilot-cli").as_str(), "copilot");
        assert_eq!(agent_key("goose").as_str(), "goose");
    }

    #[test]
    fn the_policy_reads_back() {
        let policy = Policy {
            agents: keys(&["Copilot"]),
            max_preset: Some(Preset::AskBeforeCommands),
            isolated: true,
            ..Policy::default()
        };
        let json = serde_json::to_string(&policy).unwrap();
        assert_eq!(json, r#"{"agents":["copilot"],"max_preset":"ask_before_commands","isolated":true}"#);
        assert_eq!(serde_json::from_str::<Policy>(&json).unwrap(), policy);
        assert_eq!(serde_json::from_str::<Policy>("{}").unwrap(), Policy::default());
        assert_eq!(serde_json::from_str::<Policy>(r#"{"agents":["Copilot"]}"#).unwrap().agents, keys(&["copilot"]));
    }
}
