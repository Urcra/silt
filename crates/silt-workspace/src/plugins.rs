//! What an org says of plugins in its workspaces (ADR 0104): the ones it
//! installs and locks on, which may be on at all, the versions it pins,
//! what kinds of effect it refuses, and its own rules on their actions.
//! Like every org rule, these only tighten what its members choose.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// A plugin an org installs from git, on and locked in its workspaces.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OrgPlugin {
    pub id: String,
    pub git: String,
    /// The commit it's built from, and no other.
    pub commit: String,
}

/// An org's rule on one of a plugin's actions: asked or never, for every
/// call or those whose `field` matches `pattern`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OrgPluginRule {
    pub plugin: String,
    /// An action, or `*` for every one.
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// `ask` or `never`: an org's allow would loosen its members' rules.
    pub rule: String,
}

impl OrgPluginRule {
    /// A rule as `silt org policy --plugin-rule` takes it:
    /// `plugin:action[:field=pattern]=ask|never`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let shape = || format!("{text}: give a plugin rule as plugin:action[:field=pattern]=ask|never");
        let (target, rule) = text.rsplit_once('=').ok_or_else(shape)?;
        if !matches!(rule, "ask" | "never") {
            return Err(format!("{text}: an org's plugin rule asks or never; it can't allow"));
        }
        let mut parts = target.splitn(3, ':');
        let (plugin, action) = (parts.next().ok_or_else(shape)?, parts.next().ok_or_else(shape)?);
        let (field, pattern) = match parts.next() {
            Some(matching) => {
                let (field, pattern) = matching.split_once('=').ok_or_else(shape)?;
                (Some(field.to_owned()), Some(pattern.to_owned()))
            }
            None => (None, None),
        };
        if plugin.is_empty() || action.is_empty() {
            return Err(shape());
        }
        Ok(Self { plugin: plugin.to_owned(), action: action.to_owned(), field, pattern, rule: rule.to_owned() })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginPolicy {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub install: Vec<OrgPlugin>,
    /// Only these may be on: `publisher:<name>`, `git:<url>`, or a plugin's
    /// id; any when unset. Silt's own plugins are named by id like others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<BTreeSet<String>>,
    /// The one version each named plugin may run at.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pinned: BTreeMap<String, String>,
    /// No call may have an effect Silt can't check (TCP, SSH, a local MCP
    /// server's).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refuse_opaque: bool,
    /// No plugin may widen an agent's sandbox.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refuse_reach: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<OrgPluginRule>,
}

impl PluginPolicy {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Both at once, each rule the stricter.
    pub fn stricter(&self, other: &Self) -> Self {
        let both = |one: &[OrgPlugin], other: &[OrgPlugin]| -> Vec<OrgPlugin> {
            let mut all: Vec<OrgPlugin> = one.iter().chain(other).cloned().collect();
            all.sort();
            all.dedup();
            all
        };
        let mut rules: Vec<OrgPluginRule> = self.rules.iter().chain(&other.rules).cloned().collect();
        rules.sort();
        rules.dedup();
        let mut pinned = self.pinned.clone();
        for (id, version) in &other.pinned {
            // Two pins of one plugin: the lower holds, whichever was first.
            pinned
                .entry(id.clone())
                .and_modify(|kept| *kept = kept.clone().min(version.clone()))
                .or_insert_with(|| version.clone());
        }
        Self {
            install: both(&self.install, &other.install),
            allow: match (&self.allow, &other.allow) {
                (Some(mine), Some(theirs)) => Some(mine.intersection(theirs).cloned().collect()),
                (one, other) => one.clone().or_else(|| other.clone()),
            },
            pinned,
            refuse_opaque: self.refuse_opaque || other.refuse_opaque,
            refuse_reach: self.refuse_reach || other.refuse_reach,
            rules,
        }
    }

    /// Why the plugin `id` by `publisher`, from `git` if it came from one,
    /// may not be on at `version`, if it may not.
    pub fn refusal(&self, id: &str, publisher: &str, git: Option<&str>, version: &str) -> Option<String> {
        if let Some(allowed) = &self.allow {
            let named = allowed.contains(id)
                || allowed.contains(&format!("publisher:{publisher}"))
                || git.is_some_and(|git| allowed.contains(&format!("git:{git}")));
            let installed = self.install.iter().any(|plugin| plugin.id == id);
            if !named && !installed {
                return Some(format!("{id} isn't among the plugins your org allows"));
            }
        }
        match self.pinned.get(id) {
            Some(pinned) if pinned != version => Some(format!("your org pins {id} to {pinned}; this is {version}")),
            _ => None,
        }
    }

    /// The org's rules for plugin `id`.
    pub fn rules_for<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a OrgPluginRule> + 'a {
        self.rules.iter().filter(move |rule| rule.plugin == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_parse_and_an_org_cant_allow() {
        let rule = OrgPluginRule::parse("silt.github:push:branch=main=never").unwrap();
        assert_eq!(
            (rule.plugin.as_str(), rule.action.as_str(), rule.field.as_deref(), rule.pattern.as_deref(), rule.rule.as_str()),
            ("silt.github", "push", Some("branch"), Some("main"), "never")
        );
        assert_eq!(OrgPluginRule::parse("acme.deploys:*=ask").unwrap().action, "*");
        assert!(OrgPluginRule::parse("silt.github:push=allow").is_err());
        assert!(OrgPluginRule::parse("silt.github=never").is_err());
    }

    #[test]
    fn only_allowed_plugins_at_pinned_versions_may_be_on() {
        let policy = PluginPolicy {
            allow: Some(["publisher:silt".to_owned(), "git:https://git.acme.dev/deploys".to_owned()].into()),
            pinned: [("acme.deploys".to_owned(), "2.1.0".to_owned())].into(),
            ..PluginPolicy::default()
        };
        assert_eq!(policy.refusal("silt.github", "silt", None, "1.0.0"), None);
        assert_eq!(policy.refusal("acme.deploys", "acme", Some("https://git.acme.dev/deploys"), "2.1.0"), None);
        assert!(policy.refusal("acme.deploys", "acme", Some("https://git.acme.dev/deploys"), "2.2.0").unwrap().contains("pins"));
        assert!(policy.refusal("tidewater.linear", "tidewater", None, "1.0.0").is_some());
    }

    #[test]
    fn two_policies_take_the_stricter_of_each() {
        let mine = PluginPolicy { allow: Some(["a".to_owned(), "b".to_owned()].into()), ..PluginPolicy::default() };
        let org = PluginPolicy {
            allow: Some(["b".to_owned(), "c".to_owned()].into()),
            refuse_opaque: true,
            rules: vec![OrgPluginRule::parse("b:push=never").unwrap()],
            ..PluginPolicy::default()
        };
        let both = mine.stricter(&org);
        assert_eq!(both, org.stricter(&mine));
        assert_eq!(both.allow, Some(["b".to_owned()].into()));
        assert!(both.refuse_opaque && both.rules.len() == 1);
        assert_eq!(PluginPolicy::default().stricter(&org), org);
    }
}
