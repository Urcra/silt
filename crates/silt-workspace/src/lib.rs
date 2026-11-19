//! Workspaces (ADR 0065): which projects belong to one, the server their
//! threads sync with, and the rules for agents started in it. Pure: the
//! daemon and the CLI read the files and enforce what this decides.

mod folders;
mod plugins;
mod policy;
mod preset;
mod reach;
mod sandbox;
mod seatbelt;
mod workspace;

pub use folders::{FolderAnswer, FolderRule, FolderVerdict};
pub use plugins::{OrgPlugin, OrgPluginRule, PluginPolicy};
pub use policy::{
    AgentChoice, AgentKey, Broken, KNOWN_AGENTS, ModeMeaning, OrgPolicy, Policy, Refusal, Rules, SandboxSupport, Source,
    agent_key, check_move,
};
pub use preset::Preset;
pub use reach::{Backend, Devices, Extra, Network, Reach};
pub use sandbox::{
    ALWAYS_BLOCKED, Cidr, ExtraDir, ExtraRepo, Mount, MountKind, PathDir, Relay, RunPaths, SandboxPlan, expand_home, plan,
    subtract,
};
pub use seatbelt::{SeatbeltPaths, profile as seatbelt_profile};
pub use workspace::{
    AgentDefaults, BadWorkspaceId, Defaults, InvalidWorkspaces, MatchRule, Resolved, ServerBinding, Why, Workspace, WorkspaceId,
    Workspaces, is_absolute,
};
