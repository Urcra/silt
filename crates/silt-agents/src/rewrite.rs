use std::sync::OnceLock;

use aho_corasick::{AhoCorasick, MatchKind};

use crate::AgentError;

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
    /// The session, once its id, subagent names and side files are known
    /// to stay inside its agent's session store; only such a session is
    /// restored.
    pub fn checked(self) -> Result<CheckedSession, AgentError> {
        if !is_safe_id(&self.id) {
            return Err(AgentError::Unsafe("id"));
        }
        if self.subagents.iter().any(|(name, _)| !is_safe_id(name)) {
            return Err(AgentError::Unsafe("subagent name"));
        }
        if self.attachments.iter().any(|(name, _)| !is_side_file_name(name)) {
            return Err(AgentError::Unsafe("side file"));
        }
        Ok(CheckedSession(self))
    }
}

/// A [`RecordedSession`] whose names are safe to write files under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedSession(RecordedSession);

impl CheckedSession {
    pub fn into_inner(self) -> RecordedSession {
        self.0
    }
}

impl std::ops::Deref for CheckedSession {
    type Target = RecordedSession;

    fn deref(&self) -> &RecordedSession {
        &self.0
    }
}

/// Whether `name` names a file inside a session directory: plain names
/// between slashes only. Checked by segment, as `Path` ignores a trailing
/// `/` or `/.`, which would name the directory instead.
pub fn is_side_file_name(name: &str) -> bool {
    name.split('/').all(|segment| !segment.is_empty() && segment != "." && segment != "..")
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

/// Below this many bytes of lines, rewriting them on one thread is faster
/// than starting others.
const PARALLEL_BYTES: usize = 4 << 20;

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
        Self { pairs, matcher: OnceLock::new() }
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
        let mut out = String::with_capacity(json.len());
        self.apply_into(json, &mut out);
        out
    }

    fn apply_into(&self, json: &str, out: &mut String) {
        let matcher = self.matcher.get_or_init(|| Matcher::new(&self.pairs));
        let mut matches: Vec<(usize, usize, usize)> = matcher
            .automaton
            .find_overlapping_iter(json)
            .filter(|found| json[..found.end()].ends_with('/') || ends_a_value(&json[found.end()..]))
            .map(|found| (found.start(), found.end(), found.pattern().as_usize()))
            .collect();
        matches.sort_by_key(|(start, end, pattern)| (*start, std::cmp::Reverse(*end), *pattern));
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
    }

    /// `lines` rewritten, one to a line. A long session's are rewritten on
    /// every core.
    pub fn lines(&self, lines: &[String]) -> String {
        let bytes: usize = lines.iter().map(|line| line.len() + 1).sum();
        let threads = if bytes < PARALLEL_BYTES { 1 } else { std::thread::available_parallelism().map_or(1, usize::from) };
        if threads == 1 {
            return self.joined(lines);
        }
        std::thread::scope(|scope| {
            let parts: Vec<_> =
                lines.chunks(lines.len().div_ceil(threads)).map(|part| scope.spawn(|| self.joined(part))).collect();
            parts.into_iter().map(|part| part.join().expect("rewriting doesn't panic")).collect()
        })
    }

    fn joined(&self, lines: &[String]) -> String {
        let mut out = String::with_capacity(lines.iter().map(|line| line.len() + 1).sum());
        for line in lines {
            self.apply_into(line, &mut out);
            out.push('\n');
        }
        out
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
        Self { automaton, replacements: pairs.into_iter().map(|(_, new)| json_escaped(new)).collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_file_names_name_files_inside_the_session() {
        for name in ["notes.txt", "tool-results/t1.txt", "a/b/c"] {
            assert!(is_side_file_name(name), "{name}");
        }
        for name in ["", "/etc/passwd", "a/.", "a/..", "../a", "a//b", "a/", "./a"] {
            assert!(!is_side_file_name(name), "{name}");
        }
    }

    #[test]
    fn a_long_session_rewrites_as_its_lines_do_one_by_one() {
        let rewrites = Rewrites::default().with("/a/repo", "/b/repo");
        let lines: Vec<String> = (0..PARALLEL_BYTES / 500)
            .map(|index| format!(r#"{{"cwd":"/a/repo","n":{index},"pad":"{}"}}"#, "x".repeat(500)))
            .collect();
        let one_by_one: String = lines.iter().map(|line| rewrites.apply(line) + "\n").collect();
        assert_eq!(rewrites.lines(&lines), one_by_one);
        assert!(!one_by_one.contains("/a/repo"));
    }

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
        assert!(session("s1", "agent-1", "tool-results/x.txt").checked().is_ok());
        assert!(session("../..", "agent-1", "x").checked().is_err());
        assert!(session("s1", "../x", "x").checked().is_err());
        assert!(session("s1", "agent-1", "../../.bashrc").checked().is_err());
    }
}
