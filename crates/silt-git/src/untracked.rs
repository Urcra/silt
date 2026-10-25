//! Untracked files, as `git ls-files --others --exclude-standard` lists them,
//! found by walking the worktree in this process with git's ignore rules.
//! A long-lived [`crate::Repo`] keeps what each directory held and reads a
//! directory again only when it, or a rule that applies to it, changed:
//! comparing a big checkout was mostly this walk (ADR 0034).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice;
use gix::index::entry::Mode;

use crate::GitError;

/// A directory changed this recently is read again rather than kept: a
/// change in the same tick of its mtime wouldn't show.
const SETTLED_NS: i64 = 1_000_000_000;

/// What each directory held, as last read.
#[derive(Debug, Default)]
pub(crate) struct Cache {
    /// What every directory's listing depends on besides itself: the ignore
    /// files outside the worktree and the index, when they were read.
    globals: Option<Vec<Option<i64>>>,
    dirs: HashMap<String, Dir>,
}

#[derive(Debug, Clone)]
struct Dir {
    mtime_ns: i64,
    ignore_ns: Option<i64>,
    /// What isn't ignored or tracked in it, by name.
    entries: Vec<(String, Kind)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
    /// Another repository, which git lists as a whole, as `name/`.
    Repository,
}

/// What a walk needs of the repository.
pub(crate) struct Walk<'a> {
    pub(crate) repo: &'a gix::Repository,
    pub(crate) root: &'a Path,
    pub(crate) index: &'a gix::index::File,
}

impl Walk<'_> {
    /// The untracked files under `start`, sorted. With `cache`, directories
    /// that didn't change since it was filled aren't read again.
    pub(crate) fn untracked(&self, start: &str, mut cache: Option<&mut Cache>, now_ns: i64) -> Result<Vec<String>, GitError> {
        let mut excludes = self
            .repo
            .excludes(self.index, None, gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped)
            .map_err(GitError::gix)?;
        if let Some(cache) = cache.as_deref_mut() {
            let globals = self.globals();
            if cache.globals.as_ref() != Some(&globals) {
                cache.dirs.clear();
                cache.globals = Some(globals);
            }
        }
        let mut untracked = Vec::new();
        // Each directory, with whether a rule over it may have changed.
        let mut pending = vec![(start.to_owned(), false)];
        while let Some((dir, rules_changed)) = pending.pop() {
            let at = self.root.join(&dir);
            let Ok(metadata) = fs::metadata(&at) else {
                continue;
            };
            let mtime_ns = crate::mtime_ns(&metadata);
            let ignore_ns = fs::metadata(at.join(".gitignore")).ok().map(|ignore| crate::mtime_ns(&ignore));
            let kept = cache.as_deref().and_then(|cache| cache.dirs.get(&dir));
            let (entries, rules_changed) = match kept {
                Some(kept) if !rules_changed && kept.mtime_ns == mtime_ns && kept.ignore_ns == ignore_ns => {
                    (kept.entries.clone(), false)
                }
                _ => {
                    let rules_changed = rules_changed || kept.is_none_or(|kept| kept.ignore_ns != ignore_ns);
                    let entries = self.read(&at, &dir, &mut excludes)?;
                    if let Some(cache) = cache.as_deref_mut() {
                        let settled = |ns: i64| now_ns - ns >= SETTLED_NS;
                        if settled(mtime_ns) && ignore_ns.is_none_or(settled) {
                            cache.dirs.insert(dir.clone(), Dir { mtime_ns, ignore_ns, entries: entries.clone() });
                        } else {
                            cache.dirs.remove(&dir);
                        }
                    }
                    (entries, rules_changed)
                }
            };
            for (name, kind) in entries {
                let path = if dir.is_empty() { name } else { format!("{dir}/{name}") };
                match kind {
                    Kind::Dir => pending.push((path, rules_changed)),
                    Kind::Repository => untracked.push(format!("{path}/")),
                    Kind::File => untracked.push(path),
                }
            }
        }
        untracked.sort_unstable();
        Ok(untracked)
    }

    /// What `at`, the directory `dir`, holds that isn't ignored or tracked.
    fn read(&self, at: &Path, dir: &str, excludes: &mut gix::AttributeStack<'_>) -> Result<Vec<(String, Kind)>, GitError> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(at)? {
            let entry = entry?;
            // As git's paths reach the rest of Silt only when they're UTF-8.
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name == ".git" {
                continue;
            }
            let path = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
            let is_dir = entry.file_type()?.is_dir();
            // Tracked files are most of a worktree, and never untracked
            // whatever the rules say, so they're not looked up in them. A
            // tracked directory is a submodule.
            let tracked = self.index.entry_by_path(path.as_bytes().as_bstr()).is_some();
            let kind = if !is_dir {
                Kind::File
            } else if entry.path().join(".git").exists() {
                Kind::Repository
            } else {
                Kind::Dir
            };
            if tracked && kind != Kind::Dir {
                continue;
            }
            let mode = if is_dir { Mode::DIR } else { Mode::FILE };
            if excludes.at_path(Path::new(&path), Some(mode)).map_err(GitError::gix)?.is_excluded() {
                continue;
            }
            entries.push((name, kind));
        }
        Ok(entries)
    }

    /// The modification times of what decides what is ignored or tracked
    /// everywhere: the repository's own exclude file, the user's, and the
    /// index.
    fn globals(&self) -> Vec<Option<i64>> {
        let configured = self.repo.config_snapshot().trusted_path("core.excludesFile").ok().flatten();
        let user = configured.or_else(default_excludes_file);
        let paths = [Some(self.repo.git_dir().join("info").join("exclude")), user, Some(self.index.path().to_owned())];
        paths
            .iter()
            .map(|path| path.as_ref().and_then(|path| fs::metadata(path).ok()).map(|metadata| crate::mtime_ns(&metadata)))
            .collect()
    }
}

/// Where git looks for the user's excludes when no config names a file.
fn default_excludes_file() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| silt_os::home_dir().map(|home| home.join(".config")))
        .map(|config| config.join("git").join("ignore"))
}
