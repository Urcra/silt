//! Rules for agents asking other agents (ADR 0120).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{AgentKey, Preset};

/// How many asks a thread may have running at once when no rule says.
pub const ASKS_AT_ONCE: u8 = 2;

/// What a workspace or an org lets agents ask of other agents. Every rule
/// is unset by default: asking is allowed, any agent may be asked, an asked
/// agent may do what its caller may, and only one level deep.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AskPolicy {
    /// Agents may ask other agents; they may when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed: Option<bool>,
    /// The agents that may be asked; any the workspace allows when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<BTreeSet<AgentKey>>,
    /// The most an asked agent may do without asking, below its caller's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_preset: Option<Preset>,
    /// Asks running at once per thread; [`ASKS_AT_ONCE`] when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_once: Option<u8>,
    /// An asked agent may ask further; it may not when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub further: Option<bool>,
}

/// Why an ask isn't allowed, in words the asking agent can act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AskRefusal {
    #[error("this workspace's rules don't let agents ask other agents")]
    Never,
    #[error("this workspace's rules let agents ask only {0}")]
    NotThisAgent(String),
    #[error("an asked agent may not ask another agent in this workspace")]
    NoFurther,
    #[error("this thread already has {0} asks running, the most its workspace allows at once; wait for one, or stop one")]
    TooMany(u8),
}

impl AskPolicy {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Both at once, each rule the stricter.
    pub fn stricter(&self, other: &Self) -> Self {
        Self {
            allowed: match (self.allowed, other.allowed) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (one, other) => one.or(other),
            },
            agents: match (&self.agents, &other.agents) {
                (Some(mine), Some(theirs)) => Some(mine.intersection(theirs).cloned().collect()),
                (one, other) => one.clone().or_else(|| other.clone()),
            },
            max_preset: match (self.max_preset, other.max_preset) {
                (Some(mine), Some(theirs)) => Some(mine.stricter(theirs)),
                (one, other) => one.or(other),
            },
            at_once: match (self.at_once, other.at_once) {
                (Some(mine), Some(theirs)) => Some(mine.min(theirs)),
                (one, other) => one.or(other),
            },
            further: match (self.further, other.further) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (one, other) => one.or(other),
            },
        }
    }

    pub fn at_once(&self) -> u8 {
        self.at_once.unwrap_or(ASKS_AT_ONCE)
    }

    pub fn allows_agent(&self, agent: &AgentKey) -> bool {
        self.agents.as_ref().is_none_or(|agents| agents.contains(agent))
    }

    /// Whether an agent may ask `agent`, being asked itself or not, with
    /// `running` asks of its thread already running.
    pub fn check(&self, agent: &AgentKey, asked_itself: bool, running: usize) -> Result<(), AskRefusal> {
        if self.allowed == Some(false) {
            return Err(AskRefusal::Never);
        }
        if asked_itself && self.further != Some(true) {
            return Err(AskRefusal::NoFurther);
        }
        if !self.allows_agent(agent) {
            let named: Vec<&str> = self.agents.iter().flatten().map(AgentKey::as_str).collect();
            return Err(AskRefusal::NotThisAgent(if named.is_empty() { "none".to_owned() } else { named.join(", ") }));
        }
        if running >= usize::from(self.at_once()) {
            return Err(AskRefusal::TooMany(self.at_once()));
        }
        Ok(())
    }

    /// The preset an asked agent runs at: its caller's, held to the cap.
    pub fn preset_for(&self, caller: Preset) -> Preset {
        self.max_preset.map_or(caller, |cap| caller.stricter(cap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(names: &[&str]) -> Option<BTreeSet<AgentKey>> {
        Some(names.iter().map(|name| AgentKey::new(*name)).collect())
    }

    #[test]
    fn asking_is_allowed_by_default_one_level_deep() {
        let rules = AskPolicy::default();
        assert_eq!(rules.check(&AgentKey::new("agy"), false, 0), Ok(()));
        assert_eq!(rules.check(&AgentKey::new("agy"), true, 0), Err(AskRefusal::NoFurther));
        assert_eq!(rules.check(&AgentKey::new("agy"), false, 2), Err(AskRefusal::TooMany(2)));
        assert_eq!(rules.preset_for(Preset::FullAccess), Preset::FullAccess, "no cap beyond the caller's");
    }

    #[test]
    fn rules_refuse_with_what_they_allow() {
        let rules = AskPolicy { agents: keys(&["codex"]), ..AskPolicy::default() };
        assert_eq!(rules.check(&AgentKey::new("agy"), false, 0), Err(AskRefusal::NotThisAgent("codex".into())));
        let never = AskPolicy { allowed: Some(false), ..AskPolicy::default() };
        assert_eq!(never.check(&AgentKey::new("codex"), false, 0), Err(AskRefusal::Never));
        let capped = AskPolicy { max_preset: Some(Preset::AskBeforeCommands), ..AskPolicy::default() };
        assert_eq!(capped.preset_for(Preset::Auto), Preset::AskBeforeCommands);
        assert_eq!(capped.preset_for(Preset::Plan), Preset::Plan, "never more than its caller");
    }

    #[test]
    fn the_stricter_of_two_holds() {
        let mine = AskPolicy { agents: keys(&["codex", "agy"]), at_once: Some(4), further: Some(true), ..AskPolicy::default() };
        let org = AskPolicy {
            agents: keys(&["agy", "claude"]),
            at_once: Some(1),
            max_preset: Some(Preset::AskBeforeCommands),
            ..AskPolicy::default()
        };
        let both = mine.stricter(&org);
        assert_eq!(both.agents, keys(&["agy"]));
        assert_eq!(both.at_once(), 1);
        assert_eq!(both.max_preset, Some(Preset::AskBeforeCommands));
        assert_eq!(both.further, Some(true), "a rule the org leaves unset is the person's");
        let refusing = AskPolicy { allowed: Some(false), further: Some(false), ..AskPolicy::default() };
        let both = mine.stricter(&refusing);
        assert_eq!((both.allowed, both.further), (Some(false), Some(false)));
    }
}
