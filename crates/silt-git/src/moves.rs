//! Whether git itself rewrote the worktree while a tool ran: a checkout,
//! pull, merge, rebase, reset or stash moves HEAD or the stash, or leaves a
//! merge in progress. A commit moves HEAD too, but over files the agent
//! already wrote, so it doesn't count (ADR 0066).

use std::fmt;
use std::str::FromStr;

use silt_types::GitOid;

use crate::Repo;

/// The refs that move when git rewrites the worktree, as a tool found them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitState {
    head: Option<GitOid>,
    /// How long HEAD's reflog was, so the entries a tool added can be read.
    reflog_len: Option<u64>,
    stash: Option<GitOid>,
    /// `MERGE_HEAD`, `CHERRY_PICK_HEAD` and `REVERT_HEAD`: an operation
    /// stopped on conflicts has rewritten files without moving HEAD.
    pending: [Option<GitOid>; 3],
}

const PENDING: [&str; 3] = ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"];

impl GitState {
    pub fn has_stash(&self) -> bool {
        self.stash.is_some()
    }
}

impl Repo {
    pub fn git_state(&self) -> GitState {
        let read_oid = |path: std::path::PathBuf| -> Option<GitOid> {
            std::fs::read_to_string(path).ok()?.lines().next()?.trim().parse().ok()
        };
        let stash = self
            .repo
            .try_find_reference("refs/stash")
            .ok()
            .flatten()
            .and_then(|mut reference| reference.peel_to_id().ok())
            .map(|id| crate::oid(id.detach()));
        GitState {
            head: self.head_commit().ok().flatten(),
            reflog_len: std::fs::metadata(self.head_reflog()).ok().map(|metadata| metadata.len()),
            stash,
            pending: PENDING.map(|name| read_oid(self.git_dir().join(name))),
        }
    }

    /// Whether git rewrote the worktree since `before` was taken.
    pub fn rewritten_since(&self, before: &GitState) -> bool {
        let after = self.git_state();
        let appended = match (before.reflog_len, after.reflog_len) {
            (Some(from), Some(to)) if to >= from => std::fs::read(self.head_reflog())
                .ok()
                .and_then(|bytes| usize::try_from(from).ok().and_then(|from| bytes.get(from..).map(<[u8]>::to_vec)))
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            _ => None,
        };
        let committed_onto = |head: GitOid| self.first_parent(head);
        rewritten(before, &after, appended.as_deref(), committed_onto)
    }

    /// Whether `paths` (relative, `/`-separated), as the worktree has them
    /// now, are as a stash entry has them and not as HEAD does: a `git stash
    /// apply`, which moves no ref, wrote them.
    pub fn applied_from_stash(&self, paths: &[String]) -> bool {
        let Ok(listed) = self.git(&["stash", "list", "--format=%H"], None) else {
            return false;
        };
        let stashes: Vec<GitOid> = String::from_utf8_lossy(&listed).lines().filter_map(|line| line.trim().parse().ok()).collect();
        let present: Vec<&String> = paths.iter().filter(|path| self.root().join(path).is_file()).collect();
        if stashes.is_empty() || present.is_empty() {
            return false;
        }
        let input: String = present.iter().map(|path| format!("{path}\n")).collect();
        let Ok(hashed) = self.git(&["hash-object", "--stdin-paths"], Some(input.as_bytes())) else {
            return false;
        };
        let now: Vec<(&str, GitOid)> = present
            .iter()
            .zip(String::from_utf8_lossy(&hashed).lines())
            .filter_map(|(path, oid)| Some((path.as_str(), oid.trim().parse().ok()?)))
            .collect();
        let in_tree = |commit: GitOid| -> std::collections::HashMap<String, GitOid> {
            self.files_at(commit).map(|files| files.into_iter().map(|file| (file.path, file.oid)).collect()).unwrap_or_default()
        };
        let head = self.head_commit().ok().flatten().map(in_tree).unwrap_or_default();
        let stashed: Vec<_> = stashes.into_iter().map(in_tree).collect();
        applied(&now, &head, &stashed)
    }

    fn head_reflog(&self) -> std::path::PathBuf {
        self.git_dir().join("logs").join("HEAD")
    }

    fn first_parent(&self, commit: GitOid) -> Option<GitOid> {
        let out = self.git(&["rev-list", "--parents", "-n", "1", &commit.to_string()], None).ok()?;
        String::from_utf8_lossy(&out).split_whitespace().nth(1)?.parse().ok()
    }
}

/// The functional core of [`Repo::rewritten_since`]: `appended` is what
/// HEAD's reflog gained meanwhile, when it can be read, and `parent_of`
/// names a commit's first parent.
fn rewritten(before: &GitState, after: &GitState, appended: Option<&str>, parent_of: impl Fn(GitOid) -> Option<GitOid>) -> bool {
    if before.stash != after.stash {
        return true;
    }
    if before.pending.iter().zip(&after.pending).any(|(was, now)| now.is_some() && was != now) {
        return true;
    }
    if before.head == after.head {
        return false;
    }
    match appended {
        Some(entries) if !entries.trim().is_empty() => {
            entries.lines().filter_map(reflog_message).any(|message| !message.starts_with("commit"))
        }
        // No reflog to say what moved HEAD: a commit has the old HEAD as its parent.
        _ => after.head.is_none_or(|head| before.head.is_none() || parent_of(head) != before.head),
    }
}

/// The functional core of [`Repo::applied_from_stash`]: every file in
/// `now` has a stash's content for its path, which HEAD's isn't.
fn applied(
    now: &[(&str, GitOid)],
    head: &std::collections::HashMap<String, GitOid>,
    stashed: &[std::collections::HashMap<String, GitOid>],
) -> bool {
    !now.is_empty()
        && now.iter().all(|(path, oid)| head.get(*path) != Some(oid) && stashed.iter().any(|stash| stash.get(*path) == Some(oid)))
}

/// The message of a reflog line: `<old> <new> <who> <time> <tz>\t<message>`.
fn reflog_message(line: &str) -> Option<&str> {
    line.split_once('\t').map(|(_, message)| message)
}

impl fmt::Display for GitState {
    /// One `key value` per line, as the store keeps it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(head) = self.head {
            writeln!(f, "head {head}")?;
        }
        if let Some(len) = self.reflog_len {
            writeln!(f, "reflog {len}")?;
        }
        if let Some(stash) = self.stash {
            writeln!(f, "stash {stash}")?;
        }
        for (name, oid) in PENDING.iter().zip(&self.pending) {
            if let Some(oid) = oid {
                writeln!(f, "{name} {oid}")?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("not a git state: {0:?}")]
pub struct BadGitState(String);

impl FromStr for GitState {
    type Err = BadGitState;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || BadGitState(text.to_owned());
        let mut state = Self::default();
        for line in text.lines().filter(|line| !line.is_empty()) {
            let (key, value) = line.split_once(' ').ok_or_else(bad)?;
            let oid = || value.parse::<GitOid>().map_err(|_| bad());
            match key {
                "head" => state.head = Some(oid()?),
                "reflog" => state.reflog_len = Some(value.parse().map_err(|_| bad())?),
                "stash" => state.stash = Some(oid()?),
                other => {
                    let index = PENDING.iter().position(|name| *name == other).ok_or_else(bad)?;
                    state.pending[index] = Some(oid()?);
                }
            }
        }
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(n: u8) -> GitOid {
        format!("{n:02x}").repeat(20).parse().expect("an oid")
    }

    fn at(head: u8) -> GitState {
        GitState { head: Some(oid(head)), reflog_len: Some(100), ..GitState::default() }
    }

    fn entry(message: &str) -> String {
        format!("{} {} A <a@example.com> 1700000000 +0000\t{message}\n", oid(1), oid(2))
    }

    const NO_PARENT: fn(GitOid) -> Option<GitOid> = |_| None;

    #[test]
    fn a_commit_is_the_agents_own_and_a_checkout_or_pull_is_gits() {
        assert!(!rewritten(&at(1), &at(2), Some(&entry("commit: fix the parser")), NO_PARENT));
        assert!(!rewritten(&at(1), &at(2), Some(&entry("commit (amend): fix")), NO_PARENT));
        assert!(rewritten(&at(1), &at(2), Some(&entry("checkout: moving from main to topic")), NO_PARENT));
        assert!(rewritten(&at(1), &at(2), Some(&entry("pull: Fast-forward")), NO_PARENT));
        let both = entry("commit: wip") + &entry("reset: moving to HEAD~1");
        assert!(rewritten(&at(1), &at(2), Some(&both), NO_PARENT));
    }

    #[test]
    fn nothing_moved_is_nothing_rewritten() {
        assert!(!rewritten(&at(1), &at(1), None, NO_PARENT));
    }

    #[test]
    fn a_stash_or_a_merge_stopped_on_conflicts_is_gits() {
        let stashed = GitState { stash: Some(oid(9)), ..at(1) };
        assert!(rewritten(&at(1), &stashed, None, NO_PARENT));
        assert!(rewritten(&stashed, &at(1), None, NO_PARENT));
        let merging = GitState { pending: [Some(oid(7)), None, None], ..at(1) };
        assert!(rewritten(&at(1), &merging, None, NO_PARENT));
        // Finishing that merge is a commit.
        assert!(!rewritten(&merging, &at(2), Some(&entry("commit (merge): Merge branch 'x'")), NO_PARENT));
    }

    #[test]
    fn without_a_reflog_a_commit_is_one_on_top_of_the_old_head() {
        let (before, after) = (GitState { reflog_len: None, ..at(1) }, GitState { reflog_len: None, ..at(2) });
        assert!(!rewritten(&before, &after, None, |_| Some(oid(1))));
        assert!(rewritten(&before, &after, None, |_| Some(oid(5))));
    }

    #[test]
    fn files_as_a_stash_has_them_and_head_doesnt_were_applied_from_it() {
        let map = |pairs: &[(&str, u8)]| pairs.iter().map(|(path, n)| ((*path).to_owned(), oid(*n))).collect();
        let head = map(&[("a.rs", 1), ("b.rs", 2)]);
        let stashed = vec![map(&[("a.rs", 3), ("b.rs", 2)]), map(&[("c.rs", 4)])];
        assert!(applied(&[("a.rs", oid(3))], &head, &stashed));
        assert!(applied(&[("a.rs", oid(3)), ("c.rs", oid(4))], &head, &stashed));
        assert!(!applied(&[("a.rs", oid(3)), ("d.rs", oid(5))], &head, &stashed), "d.rs is an edit of the agent's own");
        assert!(!applied(&[("b.rs", oid(2))], &head, &stashed), "b.rs is as HEAD has it");
        assert!(!applied(&[], &head, &stashed));
    }

    #[test]
    fn a_state_reads_back_as_kept() {
        let state = GitState { stash: Some(oid(3)), pending: [None, Some(oid(4)), None], ..at(1) };
        assert_eq!(state.to_string().parse::<GitState>().expect("parses"), state);
        assert_eq!("".parse::<GitState>().expect("parses"), GitState::default());
        assert!("head nope".parse::<GitState>().is_err());
    }
}
