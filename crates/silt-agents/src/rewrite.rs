use std::sync::OnceLock;

use aho_corasick::{AhoCorasick, MatchKind};

/// A session as recorded elsewhere, possibly continued across machines
/// and forks, to be written back under one id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordedSession {
    /// The id to restore it as.
    pub id: String,
    /// Ids, working directories and session directories it was recorded
    /// under; each is rewritten to the new one.
    pub former_ids: Vec<String>,
    pub former_cwds: Vec<String>,
    /// Checkout roots, rewritten to the new checkout root after the cwds.
    pub former_roots: Vec<String>,
    pub former_dirs: Vec<String>,
    pub main: Vec<String>,
    pub subagents: Vec<(String, Vec<String>)>,
    /// Side files by path relative to the session directory.
    pub attachments: Vec<(String, Vec<u8>)>,
}

/// Longest session id Silt accepts.
const MAX_ID_LEN: usize = 128;

/// Whether `id` may name a session's files and be passed to an agent CLI.
/// Session ids and subagent names come from the log, so from other people
/// (ADR 0016): letters, digits, `_` and `-` only, not starting with `-`.
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && !id.starts_with('-')
        && id.chars().all(|char| char.is_ascii_alphanumeric() || char == '_' || char == '-')
}

impl RecordedSession {
    /// Refuses a session whose id, subagent names or side files would lead
    /// outside its agent's session store.
    pub fn check(&self) -> std::io::Result<()> {
        let refuse = |what: &str| Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("the recorded session has an unsafe {what}")));
        if !is_safe_id(&self.id) {
            return refuse("id");
        }
        if self.subagents.iter().any(|(name, _)| !is_safe_id(name)) {
            return refuse("subagent name");
        }
        if self.attachments.iter().any(|(name, _)| !is_side_file_name(name)) {
            return refuse("side file");
        }
        Ok(())
    }
}

/// Whether `name` names a file inside a session directory: plain names
/// only, and not one ending in `/`, which `Path` would ignore.
pub fn is_side_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.ends_with('/')
        && std::path::Path::new(name)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

/// Replaces the paths and ids a session was recorded under with new ones.
#[derive(Debug, Clone, Default)]
pub struct Rewrites {
    pairs: Vec<(String, String)>,
    /// Built on first use from `pairs`, longest first.
    matcher: OnceLock<Matcher>,
}

#[derive(Debug, Clone)]
struct Matcher {
    automaton: AhoCorasick,
    replacements: Vec<String>,
}

impl Rewrites {
    /// Former session directories, cwds, roots and ids, in that order of
    /// precedence, mapped to the new ones.
    pub fn new(session: &RecordedSession, new_dir: Option<&str>, new_cwd: &str, new_root: &str) -> Self {
        let mut pairs = Vec::new();
        let mut add = |former: &[String], new: &str| {
            pairs.extend(former.iter().filter(|old| !old.is_empty()).map(|old| (old.clone(), new.to_owned())));
        };
        if let Some(new_dir) = new_dir {
            add(&session.former_dirs, new_dir);
        }
        add(&session.former_cwds, new_cwd);
        add(&session.former_roots, new_root);
        add(&session.former_ids, &session.id);
        Self {
            pairs,
            matcher: OnceLock::new(),
        }
    }

    pub fn with(mut self, old: &str, new: &str) -> Self {
        self.pairs.push((old.to_owned(), new.to_owned()));
        self.matcher = OnceLock::new();
        self
    }

    /// `json`, a transcript line, with every old value replaced by its new
    /// one. Both are matched and written as they appear inside JSON strings,
    /// and an old value counts only where it ends: `/a/repo` is not rewritten
    /// in `/a/repo-old`.
    pub fn apply(&self, json: &str) -> String {
        let matcher = self.matcher.get_or_init(|| Matcher::new(&self.pairs));
        let mut matches: Vec<(usize, usize, usize)> = matcher
            .automaton
            .find_overlapping_iter(json)
            .filter(|found| json[..found.end()].ends_with('/') || ends_a_value(&json[found.end()..]))
            .map(|found| (found.start(), found.end(), found.pattern().as_usize()))
            .collect();
        matches.sort_by_key(|(start, end, pattern)| (*start, std::cmp::Reverse(*end), *pattern));
        let mut out = String::with_capacity(json.len());
        let mut done = 0;
        for (start, end, pattern) in matches {
            if start < done {
                continue;
            }
            out.push_str(&json[done..start]);
            out.push_str(&matcher.replacements[pattern]);
            done = end;
        }
        out.push_str(&json[done..]);
        out
    }

    pub fn lines(&self, lines: &[String]) -> String {
        lines.iter().map(|line| self.apply(line) + "\n").collect()
    }
}

/// Whether a value ends right before `rest`: paths and ids go on with
/// letters, digits, `-` and `_`.
fn ends_a_value(rest: &str) -> bool {
    !rest.chars().next().is_some_and(|next| next.is_ascii_alphanumeric() || matches!(next, '-' | '_'))
}

/// `text` as it appears inside a JSON string.
fn json_escaped(text: &str) -> String {
    let quoted = serde_json::to_string(text).expect("strings serialize");
    quoted[1..quoted.len() - 1].to_owned()
}

impl Matcher {
    /// Every occurrence of each `old` is replaced by its `new` in one pass,
    /// so replacements are never themselves rewritten. Where several match
    /// at one place the longest wins, so a path is never rewritten through
    /// a shorter path it starts with; between equally long ones, the
    /// earlier pair.
    fn new(pairs: &[(String, String)]) -> Self {
        let pairs: Vec<&(String, String)> = pairs.iter().filter(|(old, _)| !old.is_empty()).collect();
        let automaton = AhoCorasick::builder()
            .match_kind(MatchKind::Standard)
            .build(pairs.iter().map(|(old, _)| json_escaped(old)))
            .expect("rewrites are few and short enough for an automaton");
        Self {
            automaton,
            replacements: pairs.into_iter().map(|(_, new)| json_escaped(new)).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_never_apply_to_their_own_output() {
        let rewrites = Rewrites::default().with("/a/repo", "/a/repo/new").with("/a/repo/old", "/a/repo/new");
        assert_eq!(rewrites.apply("cd /a/repo/old && cd /a/repo"), "cd /a/repo/new && cd /a/repo/new");
        assert_eq!(rewrites.with("", "x").apply("ab"), "ab", "an empty old matches nothing");
    }

    #[test]
    fn rewrites_match_whole_values_as_json_holds_them() {
        let rewrites = Rewrites::default().with("/a/repo", "/b/new \"repo\"").with("/a", "/c");
        let line = r#"{"cwd":"/a/repo","old":"/a/repo-old/x","file":"/a/repo/src/main.rs","other":"/a/b"}"#;
        assert_eq!(
            rewrites.apply(line),
            r#"{"cwd":"/b/new \"repo\"","old":"/c/repo-old/x","file":"/b/new \"repo\"/src/main.rs","other":"/c/b"}"#
        );
        assert_eq!(Rewrites::default().with("/a/", "/z/").apply(r#""/a/b""#), r#""/z/b""#);
        let quoted = Rewrites::default().with(r#"/a/"q""#, "/b");
        assert_eq!(quoted.apply(r#"{"cwd":"/a/\"q\""}"#), r#"{"cwd":"/b"}"#);
        let parsed: serde_json::Value = serde_json::from_str(&rewrites.apply(line)).expect("still JSON");
        assert_eq!(parsed["cwd"], "/b/new \"repo\"");
    }

    #[test]
    fn sessions_with_unsafe_names_are_refused() {
        assert!(is_safe_id("0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b") && is_safe_id("ses_c4949c2fd001MQcUSXA3sXpVlA"));
        for id in ["", "../../..", "/home/alice", "-rf", "a/b", "a\nb", "é"] {
            assert!(!is_safe_id(id), "{id:?}");
        }
        let session = |id: &str, subagent: &str, attachment: &str| RecordedSession {
            id: id.to_owned(),
            subagents: vec![(subagent.to_owned(), Vec::new())],
            attachments: vec![(attachment.to_owned(), Vec::new())],
            ..Default::default()
        };
        assert!(session("s1", "agent-1", "tool-results/x.txt").check().is_ok());
        assert!(session("../..", "agent-1", "x").check().is_err());
        assert!(session("s1", "../x", "x").check().is_err());
        assert!(session("s1", "agent-1", "../../.bashrc").check().is_err());
    }
}
