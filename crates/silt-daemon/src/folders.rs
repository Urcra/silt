//! Folders granted to a thread's agents (ADR 0101), kept in
//! `reach/<thread>.json`: each for the run that asked or for the thread on
//! this machine. A grant reaches the agent when its turn ends: Silt starts
//! its sandbox again with the folder and resumes its session.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use silt_workspace::Reach;

use crate::CoreError;

/// Who a folder is granted to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "for")]
pub enum FolderScope {
    /// The run whose token hashes to `run`, in hex.
    Run { run: String },
    /// The thread on this machine, until `until_ms` if set.
    Thread {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until_ms: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderGrant {
    /// Where it really is, every link followed.
    pub path: String,
    pub writable: bool,
    pub scope: FolderScope,
    pub granted_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    grants: Vec<FolderGrant>,
    /// What the thread's agent reaches changed since it started.
    #[serde(default)]
    pending: bool,
    /// When a terminal's agent's turn ended with a change pending: the
    /// terminal restarts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restart_ms: Option<i64>,
}

fn file(data_dir: &Path, thread: &str) -> Result<PathBuf, CoreError> {
    if thread.is_empty() || !thread.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(CoreError::Invalid(format!("no thread {thread}")));
    }
    Ok(data_dir.join("reach").join(format!("{thread}.json")))
}

fn read(data_dir: &Path, thread: &str) -> Kept {
    file(data_dir, thread)
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write(data_dir: &Path, thread: &str, kept: &Kept) -> Result<(), CoreError> {
    let path = file(data_dir, thread)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, serde_json::to_vec_pretty(kept).map_err(std::io::Error::other)?)?;
    std::fs::rename(partial, path)?;
    Ok(())
}

fn change(data_dir: &Path, thread: &str, changed: impl FnOnce(&mut Kept)) -> Result<(), CoreError> {
    let mut kept = read(data_dir, thread);
    changed(&mut kept);
    write(data_dir, thread, &kept)
}

/// How a run is named in its grants: its token's hash, in hex.
pub fn run_id(hash: &[u8]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The run id of the run holding `token`.
pub fn run_of_token(token: &str) -> String {
    run_id(blake3::hash(token.as_bytes()).as_bytes())
}

/// Keeps `grant`; the thread's agent gets it at the end of its turn.
pub fn record(data_dir: &Path, thread: &str, grant: FolderGrant) -> Result<(), CoreError> {
    change(data_dir, thread, |kept| {
        kept.grants.retain(|other| !(other.path == grant.path && other.scope == grant.scope));
        kept.grants.push(grant);
        kept.pending = true;
    })
}

/// Ends the thread's grants of `path`; its agent loses it at the end of
/// its turn. Answers whether there was one.
pub fn revoke(data_dir: &Path, thread: &str, path: &str) -> Result<bool, CoreError> {
    let mut found = false;
    change(data_dir, thread, |kept| {
        let before = kept.grants.len();
        kept.grants.retain(|grant| grant.path != path);
        found = kept.grants.len() != before;
        kept.pending |= found;
    })?;
    Ok(found)
}

/// The folders granted to the run whose token hashes to `run`, in force at
/// `now_ms`, each writable if any of its grants is.
pub fn granted(data_dir: &Path, thread: &str, run: Option<&str>, now_ms: i64) -> BTreeMap<String, bool> {
    let mut folders = BTreeMap::new();
    for grant in read(data_dir, thread).grants {
        let in_force = match &grant.scope {
            FolderScope::Run { run: of } => run == Some(of.as_str()),
            FolderScope::Thread { until_ms } => until_ms.is_none_or(|until| until > now_ms),
        };
        if in_force {
            *folders.entry(grant.path).or_insert(false) |= grant.writable;
        }
    }
    folders
}

/// `granted` less what the rules in force refuse now: a folder an org's
/// rule made never since it was granted, or one Silt won't hand over.
pub fn still_allowed(
    granted: BTreeMap<String, bool>,
    policy: Option<&silt_workspace::Policy>,
    data_dir: &Path,
) -> BTreeMap<String, bool> {
    let home = silt_os::home_dir();
    granted
        .into_iter()
        .filter(|(path, writable)| {
            let dir = crate::sandbox::extra_dir(path, *writable, home.as_deref());
            if crate::sandbox::folder_refusal(data_dir, &dir).is_some() {
                return false;
            }
            policy.is_none_or(|policy| {
                let resolve = |root: &str| std::fs::canonicalize(silt_workspace::expand_home(root, home.as_deref())).ok();
                policy.folder_verdict(Path::new(path), *writable, resolve).answer != silt_workspace::FolderAnswer::Never
            })
        })
        .collect()
}

/// Every grant the thread has, for its Access tab.
pub fn grants(data_dir: &Path, thread: &str) -> Vec<FolderGrant> {
    read(data_dir, thread).grants
}

/// `reach` with the folders granted to the run added: what its sandbox is
/// made with at a start or a restart.
pub fn widened(mut reach: Reach, granted: &BTreeMap<String, bool>) -> Reach {
    for (path, writable) in granted {
        *reach.paths.entry(path.clone()).or_insert(false) |= *writable;
    }
    reach
}

/// Whether what the thread's agent reaches changed since it started, and
/// takes the change as seen: the caller restarts it.
pub fn take_pending(data_dir: &Path, thread: &str) -> bool {
    let kept = read(data_dir, thread);
    if !kept.pending {
        return false;
    }
    change(data_dir, thread, |kept| kept.pending = false).is_ok()
}

/// The thread's agent's turn ended: one a terminal runs is restarted there
/// if what it reaches changed.
pub fn turn_ended(data_dir: &Path, thread: &str, now_ms: i64) -> Result<(), CoreError> {
    if !read(data_dir, thread).pending {
        return Ok(());
    }
    change(data_dir, thread, |kept| {
        kept.pending = false;
        kept.restart_ms = Some(now_ms);
    })
}

/// Whether a terminal that started its agent at `started_ms` should now
/// restart it, taking the request.
pub fn take_restart(data_dir: &Path, thread: &str, started_ms: i64) -> bool {
    let kept = read(data_dir, thread);
    match kept.restart_ms {
        Some(asked) if asked >= started_ms => change(data_dir, thread, |kept| kept.restart_ms = None).is_ok(),
        _ => false,
    }
}

/// What the agent is told when its session resumes with `added` and
/// without `removed`.
pub fn resume_note(added: &BTreeMap<String, bool>, removed: &[String]) -> String {
    let mut said = Vec::new();
    for (path, writable) in added {
        let access = if *writable { "read and write" } else { "read-only" };
        said.push(format!("{path} is now in your sandbox, {access}, at that path."));
    }
    for path in removed {
        said.push(format!("{path} was taken back and isn't in your sandbox any more."));
    }
    said.push("You were restarted for this; carry on where you left off.".to_owned());
    said.join(" ")
}

/// What `after` has that `before` hasn't, and what it lost.
pub fn difference(before: &BTreeMap<String, bool>, after: &BTreeMap<String, bool>) -> (BTreeMap<String, bool>, Vec<String>) {
    let added = after
        .iter()
        .filter(|(path, writable)| before.get(*path) != Some(writable))
        .map(|(path, writable)| (path.clone(), *writable))
        .collect();
    let removed = before.keys().filter(|path| !after.contains_key(*path)).cloned().collect();
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(path: &str, writable: bool, scope: FolderScope) -> FolderGrant {
        FolderGrant { path: path.into(), writable, scope, granted_ms: 1 }
    }

    #[test]
    fn a_run_s_grant_is_its_own_and_a_thread_s_lasts() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        record(data, "t1", grant("/src/lib", false, FolderScope::Run { run: "aa".into() })).unwrap();
        record(data, "t1", grant("/src/data", true, FolderScope::Thread { until_ms: Some(100) })).unwrap();
        assert_eq!(granted(data, "t1", Some("aa"), 10).len(), 2);
        assert_eq!(granted(data, "t1", Some("bb"), 10), BTreeMap::from([("/src/data".into(), true)]));
        assert!(granted(data, "t1", Some("bb"), 100).is_empty(), "expired");
        assert!(granted(data, "t2", Some("aa"), 10).is_empty());
        assert!(file(data, "../x").is_err());
    }

    #[test]
    fn a_terminal_restarts_once_its_turn_ends_with_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        turn_ended(data, "t1", 5).unwrap();
        assert!(!take_restart(data, "t1", 0), "nothing changed");
        record(data, "t1", grant("/src/lib", false, FolderScope::Thread { until_ms: None })).unwrap();
        assert!(!take_restart(data, "t1", 0), "not before the turn ends");
        turn_ended(data, "t1", 5).unwrap();
        assert!(!take_restart(data, "t1", 6), "asked of an earlier start");
        turn_ended(data, "t1", 5).unwrap();
        assert!(revoke(data, "t1", "/src/lib").unwrap());
        turn_ended(data, "t1", 7).unwrap();
        assert!(take_restart(data, "t1", 6));
        assert!(!take_restart(data, "t1", 6), "taken");
    }

    #[test]
    fn a_grant_the_rules_now_refuse_is_dropped_at_the_next_start() {
        let data = tempfile::tempdir().unwrap();
        let mine = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(mine.path()).unwrap();
        let (kept, refused) = (root.join("kept"), root.join("refused"));
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::create_dir_all(&refused).unwrap();
        let granted =
            BTreeMap::from([(kept.to_string_lossy().into_owned(), false), (refused.to_string_lossy().into_owned(), false)]);
        let org = silt_workspace::Policy {
            folders: vec![silt_workspace::FolderRule::parse(&format!("{}:never", refused.display())).unwrap()],
            ..silt_workspace::Policy::default()
        };
        let mine = silt_workspace::Policy::default().stricter(&org);
        let left = still_allowed(granted.clone(), Some(&mine), data.path());
        assert_eq!(left.keys().cloned().collect::<Vec<_>>(), [kept.to_string_lossy().into_owned()]);
        let home = silt_os::home_dir().unwrap().to_string_lossy().into_owned();
        assert!(still_allowed(BTreeMap::from([(home, false)]), None, data.path()).is_empty(), "never the home folder");
        assert_eq!(still_allowed(granted, None, data.path()).len(), 2);
    }

    #[test]
    fn the_agent_is_told_what_changed() {
        let before = BTreeMap::from([("/a".to_owned(), false), ("/b".to_owned(), false)]);
        let after = BTreeMap::from([("/a".to_owned(), true), ("/c".to_owned(), false)]);
        let (added, removed) = difference(&before, &after);
        assert_eq!(added.keys().collect::<Vec<_>>(), ["/a", "/c"]);
        assert_eq!(removed, ["/b"]);
        let note = resume_note(&added, &removed);
        assert!(note.contains("/a is now in your sandbox, read and write") && note.contains("/b was taken back"), "{note}");
    }
}
