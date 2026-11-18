//! The folders an agent may ask for (ADR 0098): rules naming roots, each
//! allowed, asked or never, and whether writing may be given there. In one
//! set of rules the most specific root wins; sets combined from more than
//! one policy are each judged alone, and the strictest answer holds, so an
//! org's rules only tighten the person's.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Policy;
use crate::reach::Extra;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum FolderAnswer {
    Allow,
    Ask,
    Never,
}

impl FolderAnswer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Never => "never",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [Self::Allow, Self::Ask, Self::Never].into_iter().find(|answer| answer.as_str() == name)
    }
}

/// A root and how a folder in it is answered.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FolderRule {
    /// A folder, `~/` for the home folder's.
    pub root: String,
    pub answer: FolderAnswer,
    /// Writing may be given in it.
    #[serde(default)]
    pub writable: bool,
}

impl FolderRule {
    /// A rule as `silt workspace policy --ask-folders` takes it:
    /// `root:allow|ask|never[:rw]`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut parts: Vec<&str> = text.rsplitn(3, ':').collect();
        parts.reverse();
        let (root, answer, writable) = match parts.as_slice() {
            [root, answer] => (*root, *answer, false),
            [root, answer, "rw"] => (*root, *answer, true),
            [root, answer, "ro"] => (*root, *answer, false),
            [first, second, third] if FolderAnswer::parse(third).is_some() => {
                // A root with a `:` in it, without a mode.
                return Ok(Self {
                    root: format!("{first}:{second}"),
                    answer: FolderAnswer::parse(third).expect("checked"),
                    writable: false,
                });
            }
            _ => return Err(format!("{text}: give a folder rule as root:allow|ask|never[:rw]")),
        };
        let answer = FolderAnswer::parse(answer).ok_or_else(|| format!("{text}: allow, ask or never, not {answer}"))?;
        if root.is_empty() {
            return Err(format!("{text}: name a folder"));
        }
        Ok(Self { root: root.to_owned(), answer, writable: writable && answer != FolderAnswer::Never })
    }

    fn shown(&self) -> String {
        let mode = if self.writable { ":rw" } else { "" };
        format!("{}:{}{mode}", self.root, self.answer.as_str())
    }
}

/// How a folder asked for is answered, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FolderVerdict {
    pub answer: FolderAnswer,
    pub why: String,
}

impl Policy {
    /// Every set of folder rules in force, each judged on its own.
    pub fn folder_rule_sets(&self) -> impl Iterator<Item = &[FolderRule]> {
        std::iter::once(self.folders.as_slice()).chain(self.folder_layers.iter().map(Vec::as_slice)).filter(|set| !set.is_empty())
    }

    /// How `real`, a folder with every link followed, is answered when an
    /// agent asks for it. `resolve` says where a rule's root really is, `~/`
    /// expanded and links followed, or `None` when it isn't there.
    pub fn folder_verdict(&self, real: &Path, writable: bool, resolve: impl Fn(&str) -> Option<PathBuf>) -> FolderVerdict {
        if self.refuse.contains(&Extra::Paths) {
            return FolderVerdict { answer: FolderAnswer::Never, why: "a rule in force refuses more folders".to_owned() };
        }
        let unnamed = || FolderVerdict { answer: FolderAnswer::Ask, why: "no folder rule names it".to_owned() };
        let mut strictest: Option<FolderVerdict> = None;
        for set in self.folder_rule_sets() {
            let closest = set
                .iter()
                .filter_map(|rule| resolve(&rule.root).filter(|root| real.starts_with(root)).map(|root| (root, rule)))
                .max_by_key(|(root, _)| root.components().count());
            let this = match closest {
                None => unnamed(),
                Some((_, rule)) if writable && rule.answer != FolderAnswer::Never && !rule.writable => FolderVerdict {
                    answer: FolderAnswer::Never,
                    why: format!("the folder rule {} doesn't give writing", rule.shown()),
                },
                Some((_, rule)) => FolderVerdict { answer: rule.answer, why: format!("the folder rule {}", rule.shown()) },
            };
            if strictest.as_ref().is_none_or(|kept| this.answer > kept.answer) {
                strictest = Some(this);
            }
        }
        strictest.unwrap_or_else(unnamed)
    }

    /// The folder rules of both policies, as [`Policy::stricter`] keeps them:
    /// each policy's own set apart, in an order that doesn't depend on which
    /// is which.
    pub(crate) fn both_folder_rules(&self, other: &Self) -> (Vec<FolderRule>, Vec<Vec<FolderRule>>) {
        let mut sets: Vec<Vec<FolderRule>> = self.folder_rule_sets().chain(other.folder_rule_sets()).map(<[_]>::to_vec).collect();
        sets.sort();
        sets.dedup();
        match sets.len() {
            0 => (Vec::new(), Vec::new()),
            1 => (sets.pop().expect("one"), Vec::new()),
            _ => (Vec::new(), sets),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(text: &str) -> FolderRule {
        FolderRule::parse(text).unwrap()
    }

    fn resolve(root: &str) -> Option<PathBuf> {
        Some(crate::expand_home(root, Some(Path::new("/home/ada"))))
    }

    fn verdict(policy: &Policy, path: &str, writable: bool) -> FolderAnswer {
        policy.folder_verdict(Path::new(path), writable, resolve).answer
    }

    #[test]
    fn rules_parse_with_their_mode() {
        assert_eq!(rule("~/code:allow:rw"), FolderRule { root: "~/code".into(), answer: FolderAnswer::Allow, writable: true });
        assert!(!rule("~/work:ask").writable);
        assert!(!rule("~/secrets:never:rw").writable, "never gives nothing");
        assert_eq!(rule("C:/code:allow").root, "C:/code");
        assert!(FolderRule::parse("~/code:maybe").is_err());
        assert!(FolderRule::parse("~/code").is_err());
    }

    #[test]
    fn the_most_specific_root_wins_and_unnamed_folders_ask() {
        let policy = Policy {
            folders: vec![rule("~/code:ask"), rule("~/code/silt:allow:rw"), rule("~/code/silt/vendor:never")],
            ..Policy::default()
        };
        assert_eq!(verdict(&policy, "/home/ada/code/silt/crates", true), FolderAnswer::Allow);
        assert_eq!(verdict(&policy, "/home/ada/code/other", false), FolderAnswer::Ask);
        assert_eq!(verdict(&policy, "/home/ada/code/silt/vendor/x", false), FolderAnswer::Never);
        assert_eq!(verdict(&policy, "/home/ada/music", false), FolderAnswer::Ask);
        assert_eq!(verdict(&policy, "/home/ada/code/other", true), FolderAnswer::Never, "the rule doesn't give writing");
        assert_eq!(verdict(&policy, "/home/ada/code-old", false), FolderAnswer::Ask, "a root holds whole names only");
    }

    #[test]
    fn an_orgs_rules_only_tighten() {
        let mine = Policy { folders: vec![rule("~/work/api:allow:rw")], ..Policy::default() };
        let org = Policy { folders: vec![rule("~/work:ask")], ..Policy::default() };
        let both = mine.stricter(&org);
        assert_eq!(both, org.stricter(&mine));
        assert_eq!(verdict(&both, "/home/ada/work/api", false), FolderAnswer::Ask);
        let refusing = Policy { refuse: [Extra::Paths].into(), ..Policy::default() };
        assert_eq!(verdict(&mine.stricter(&refusing), "/home/ada/work/api", false), FolderAnswer::Never);
        assert_eq!(Policy::default().stricter(&mine), mine);
        let reason = both.folder_verdict(Path::new("/home/ada/work/api"), false, resolve).why;
        assert!(reason.contains("~/work:ask"), "{reason}");
    }
}
