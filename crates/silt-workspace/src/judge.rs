//! Who checks the commands of an agent on Auto that has no auto mode of its
//! own (ADR 0132).

use serde::{Deserialize, Serialize};

use crate::{Rules, Source};

/// The agent and model that judge a thread's commands on Auto.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Judge {
    /// The thread's own agent, on the model it runs.
    #[default]
    Thread,
    /// An agent by its registry id, on one of its models, or the one it
    /// starts with when unset.
    Agent {
        agent: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
}

impl Judge {
    /// The agent that judges and the model asked of it, for a thread whose
    /// agent is `own` on `own_model`.
    pub fn agent_for<'a>(&'a self, own: &'a str, own_model: Option<&'a str>) -> (&'a str, Option<&'a str>) {
        match self {
            Self::Thread => (own, own_model),
            Self::Agent { agent, model } => (agent, model.as_deref()),
        }
    }
}

impl Rules<'_> {
    /// The judge the rules fix, and whose rule it is: an org's over the
    /// person's own.
    pub fn fixed_judge(&self) -> Option<(Judge, Source)> {
        if let Some(Ok(org)) = &self.org
            && let Some(judge) = &org.policy.auto_judge
        {
            return Some((judge.clone(), Source::Org { org: org.org.clone(), server: org.server.clone() }));
        }
        self.local.auto_judge.clone().map(|judge| (judge, Source::Local))
    }

    /// The judge that holds for a thread: the one the rules fix, else the
    /// thread's own choice, else the workspace's default.
    pub fn judge(&self, chosen: Option<&Judge>, default: Option<&Judge>) -> Judge {
        self.fixed_judge().map(|(judge, _)| judge).or_else(|| chosen.or(default).cloned()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OrgPolicy, Policy};

    fn copilot(model: &str) -> Judge {
        Judge::Agent { agent: "github-copilot-cli".into(), model: Some(model.into()) }
    }

    #[test]
    fn the_thread_s_own_agent_judges_unless_something_says_otherwise() {
        let local = Policy::default();
        let rules = Rules { workspace: "Default", local: &local, org: None };
        assert_eq!(rules.judge(None, None), Judge::Thread);
        assert_eq!(rules.judge(None, Some(&copilot("haiku"))), copilot("haiku"), "the workspace's default");
        assert_eq!(rules.judge(Some(&copilot("opus")), Some(&copilot("haiku"))), copilot("opus"), "the thread's choice");
        assert_eq!(Judge::Thread.agent_for("codex", Some("gpt")), ("codex", Some("gpt")));
        assert_eq!(copilot("haiku").agent_for("codex", Some("gpt")), ("github-copilot-cli", Some("haiku")));
    }

    #[test]
    fn an_org_s_judge_holds_over_everyone_else_s() {
        let local = Policy { auto_judge: Some(copilot("opus")), ..Policy::default() };
        let org = OrgPolicy {
            org: "ledger".into(),
            server: "https://silt.ledger.dev".into(),
            policy: Policy { auto_judge: Some(copilot("haiku")), ..Policy::default() },
        };
        let rules = Rules { workspace: "Bank", local: &local, org: Some(Ok(&org)) };
        assert_eq!(rules.judge(Some(&Judge::Thread), None), copilot("haiku"));
        assert!(matches!(rules.fixed_judge(), Some((_, Source::Org { org, .. })) if org == "ledger"));
        assert_eq!(rules.effective().auto_judge, Some(copilot("haiku")));
        let mine = Rules { workspace: "Bank", local: &local, org: None };
        assert_eq!(mine.fixed_judge(), Some((copilot("opus"), Source::Local)));
    }

    #[test]
    fn a_judge_reads_as_its_kind() {
        let read: Judge = serde_json::from_str(r#"{"kind":"agent","agent":"codex"}"#).expect("an agent");
        assert_eq!(read, Judge::Agent { agent: "codex".into(), model: None });
        assert_eq!(serde_json::to_string(&Judge::Thread).expect("json"), r#"{"kind":"thread"}"#);
    }
}
