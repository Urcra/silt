//! Workspaces (ADR 0065): which projects belong to one, the server their
//! threads sync with, and the rules for agents started in it. Pure: the
//! daemon and the CLI read the files and enforce what this decides.

mod policy;
mod preset;
mod workspace;

pub use policy::{
    AgentChoice, AgentKey, Broken, KNOWN_AGENTS, ModeMeaning, OrgPolicy, Policy, Refusal, Rules, SandboxSupport, Source,
    agent_key, check_move,
};
pub use preset::Preset;
pub use workspace::{
    AgentDefaults, BadWorkspaceId, Defaults, InvalidWorkspaces, MatchRule, Resolved, ServerBinding, Why, Workspace, WorkspaceId,
    Workspaces, is_absolute,
};
