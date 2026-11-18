//! What decides a call: allowed, asked or never, from the person's rules,
//! an org's and the manifest's defaults, with one precedence for every
//! plugin (ADR 0103):
//!
//! 1. Silt's built-in refusals, which nothing lifts;
//! 2. a never from any scope;
//! 3. an ask from a pattern, or from an org;
//! 4. an allow from one of the person's patterns;
//! 5. the person's rule for the action;
//! 6. the action's default, once its publisher is trusted;
//! 7. otherwise, ask.
//!
//! An org's rules only tighten: an allow from one is ignored.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::glob::{Glob, specificity};
use crate::manifest::ActionSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Allow,
    Ask,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "from", content = "name")]
pub enum RuleSource {
    /// Silt's own, which nothing lifts.
    BuiltIn,
    /// An org's policy, locked.
    Org(String),
    Yours,
    /// The manifest's default, its publisher trusted.
    Default,
    /// Asked because its publisher isn't trusted yet.
    Untrusted,
}

/// A rule someone set: for one action or every one (`*`), and for calls
/// whose `field` matches `pattern`, or all of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScopedRule {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<Glob>,
    pub rule: Rule,
    /// Each call approved on its own; no grant covers it.
    #[serde(default)]
    pub once: bool,
    pub source: RuleSource,
}

impl ScopedRule {
    fn covers(&self, action: &str, fields: &BTreeMap<String, String>) -> bool {
        if self.action != "*" && self.action != action {
            return false;
        }
        match (&self.field, &self.pattern) {
            (Some(field), Some(pattern)) => fields.get(field).is_some_and(|value| pattern.matches(value)),
            (None, None) => true,
            // Half a pattern matches nothing rather than everything.
            _ => false,
        }
    }

    fn patterned(&self) -> bool {
        self.pattern.is_some()
    }

    fn by_org(&self) -> bool {
        matches!(self.source, RuleSource::Org(_))
    }
}

/// Every rule in force for one plugin: the person's and the orgs'.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Rules(pub Vec<ScopedRule>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Decision {
    pub rule: Rule,
    /// No grant may cover it.
    pub once: bool,
    pub source: RuleSource,
    /// What decided it, in words, for the card and the Access tab.
    pub why: String,
}

/// How `action` is decided for a call with `fields`. `refusal` is Silt's own
/// reason to refuse it whatever the rules say, such as a folder that holds
/// credentials.
pub fn decide(
    action: &ActionSpec,
    trusted: bool,
    rules: &Rules,
    refusal: Option<&str>,
    fields: &BTreeMap<String, String>,
) -> Decision {
    let covering: Vec<&ScopedRule> = rules.0.iter().filter(|rule| rule.covers(&action.id, fields)).collect();
    let once = action.once || covering.iter().any(|rule| rule.once && rule.rule != Rule::Allow);
    let decided = |rule: Rule, source: RuleSource, why: String| Decision { rule, once, source, why };
    if let Some(reason) = refusal {
        return decided(Rule::Never, RuleSource::BuiltIn, reason.to_owned());
    }
    let pick = |wanted: Rule, keep: &dyn Fn(&ScopedRule) -> bool| -> Option<&ScopedRule> {
        covering
            .iter()
            .copied()
            .filter(|rule| rule.rule == wanted && keep(rule))
            // An org's first, then the most specific.
            .max_by_key(|rule| (rule.by_org(), rule.pattern.as_ref().map(|glob| specificity(&glob.0))))
    };
    let described = |rule: &ScopedRule| -> String {
        let who = match &rule.source {
            RuleSource::Org(org) => format!("{org}'s policy"),
            _ => "your rule".to_owned(),
        };
        match (&rule.field, &rule.pattern) {
            (Some(field), Some(pattern)) => format!("{who} for {} where {field} is {}", action.label, pattern.0),
            _ => format!("{who} for {}", action.label),
        }
    };
    if let Some(rule) = pick(Rule::Never, &|_| true) {
        return decided(Rule::Never, rule.source.clone(), described(rule));
    }
    if let Some(rule) = pick(Rule::Ask, &|rule| rule.patterned() || rule.by_org()) {
        return decided(Rule::Ask, rule.source.clone(), described(rule));
    }
    if let Some(rule) = pick(Rule::Allow, &|rule| rule.patterned() && !rule.by_org()) {
        return decided(Rule::Allow, rule.source.clone(), described(rule));
    }
    let yours = covering.iter().find(|rule| rule.source == RuleSource::Yours && !rule.patterned() && rule.action == action.id);
    if let Some(rule) = yours {
        return decided(rule.rule, RuleSource::Yours, described(rule));
    }
    if trusted {
        return decided(action.default, RuleSource::Default, format!("the default for {}", action.label));
    }
    decided(Rule::Ask, RuleSource::Untrusted, "its publisher isn't trusted yet, so every action asks".to_owned())
}

/// The environment whose patterns match `value` most closely, if any.
pub fn environment_for<'a>(value: &str, environments: &'a [(String, Vec<Glob>)]) -> Option<&'a str> {
    environments
        .iter()
        .filter_map(|(name, patterns)| {
            patterns
                .iter()
                .filter(|pattern| pattern.matches(value))
                .map(|pattern| specificity(&pattern.0))
                .max()
                .map(|best| (best, name))
        })
        .max_by(|one, other| one.0.cmp(&other.0))
        .map(|(_, name)| name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ActionKind;

    fn push() -> ActionSpec {
        ActionSpec {
            id: "push".into(),
            label: "Push a branch".into(),
            description: "d".into(),
            kind: ActionKind::Change,
            default: Rule::Ask,
            once: false,
            fields: vec!["repo".into(), "branch".into()],
        }
    }

    fn rule(rule: Rule, source: RuleSource, pattern: Option<(&str, &str)>) -> ScopedRule {
        ScopedRule {
            action: "push".into(),
            field: pattern.map(|(field, _)| field.to_owned()),
            pattern: pattern.map(|(_, glob)| Glob::new(glob)),
            rule,
            once: false,
            source,
        }
    }

    fn call(branch: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("repo".into(), "acme/api".into()), ("branch".into(), branch.into())])
    }

    #[test]
    fn the_precedence_holds() {
        let acme = || RuleSource::Org("acme".into());
        let cases: Vec<(&str, Vec<ScopedRule>, bool, Rule, RuleSource)> = vec![
            ("untrusted asks", vec![], false, Rule::Ask, RuleSource::Untrusted),
            ("trusted takes the default", vec![], true, Rule::Ask, RuleSource::Default),
            (
                "your action rule over the default",
                vec![rule(Rule::Allow, RuleSource::Yours, None)],
                false,
                Rule::Allow,
                RuleSource::Yours,
            ),
            (
                "your pattern allow over your action ask",
                vec![rule(Rule::Ask, RuleSource::Yours, None), rule(Rule::Allow, RuleSource::Yours, Some(("branch", "ada/*")))],
                true,
                Rule::Allow,
                RuleSource::Yours,
            ),
            (
                "an org's ask over your pattern allow",
                vec![rule(Rule::Ask, acme(), None), rule(Rule::Allow, RuleSource::Yours, Some(("branch", "ada/*")))],
                true,
                Rule::Ask,
                acme(),
            ),
            ("an org's allow is ignored", vec![rule(Rule::Allow, acme(), None)], false, Rule::Ask, RuleSource::Untrusted),
            (
                "your never over an org's ask",
                vec![rule(Rule::Ask, acme(), None), rule(Rule::Never, RuleSource::Yours, None)],
                true,
                Rule::Never,
                RuleSource::Yours,
            ),
            (
                "an org's never over everything you allow",
                vec![rule(Rule::Allow, RuleSource::Yours, None), rule(Rule::Never, acme(), Some(("branch", "main")))],
                true,
                Rule::Never,
                acme(),
            ),
        ];
        for (name, rules, trusted, expected, source) in cases {
            let branch = if name.contains("everything") { "main" } else { "ada/retry" };
            let decision = decide(&push(), trusted, &Rules(rules), None, &call(branch));
            assert_eq!((decision.rule, decision.source), (expected, source), "{name}: {}", decision.why);
        }
    }

    #[test]
    fn a_pattern_covers_only_what_it_matches() {
        let rules = Rules(vec![rule(Rule::Allow, RuleSource::Yours, Some(("branch", "ada/*")))]);
        assert_eq!(decide(&push(), true, &rules, None, &call("ada/x")).rule, Rule::Allow);
        assert_eq!(decide(&push(), true, &rules, None, &call("main")).rule, Rule::Ask);
        assert_eq!(decide(&push(), true, &rules, None, &BTreeMap::new()).rule, Rule::Ask, "no field, no match");
        let decision = decide(&push(), true, &rules, None, &call("ada/x"));
        assert_eq!(decision.why, "your rule for Push a branch where branch is ada/*");
    }

    #[test]
    fn built_in_refusals_beat_everything_and_once_sticks() {
        let rules = Rules(vec![rule(Rule::Allow, RuleSource::Yours, None)]);
        let refused = decide(&push(), true, &rules, Some("~/.ssh holds credentials"), &call("x"));
        assert_eq!((refused.rule, refused.source), (Rule::Never, RuleSource::BuiltIn));
        let once = ScopedRule { once: true, ..rule(Rule::Ask, RuleSource::Org("acme".into()), None) };
        assert!(decide(&push(), true, &Rules(vec![once]), None, &call("x")).once);
        assert!(decide(&ActionSpec { once: true, ..push() }, true, &Rules::default(), None, &call("x")).once);
    }

    #[test]
    fn the_most_specific_environment_wins() {
        let environments = vec![
            ("acme".to_owned(), vec![Glob::new("acme/*"), Glob::new("acme-labs/*")]),
            ("billing".to_owned(), vec![Glob::new("acme/billing-*")]),
            ("personal".to_owned(), vec![Glob::new("ada/*")]),
        ];
        assert_eq!(environment_for("acme/api", &environments), Some("acme"));
        assert_eq!(environment_for("acme/billing-api", &environments), Some("billing"));
        assert_eq!(environment_for("ada/dotfiles", &environments), Some("personal"));
        assert_eq!(environment_for("other/repo", &environments), None);
    }
}
