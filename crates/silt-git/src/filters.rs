use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice;
use gix::filter::plumbing::driver::apply::Delay;
use gix::filter::plumbing::pipeline::convert::to_worktree;

use crate::{GitError, run_git};

/// Converts between the content git stores ("clean") and what belongs on
/// disk ("smudged"): line endings, `.gitattributes` and filter drivers.
///
/// gix matches git except for non-UTF-8 `working-tree-encoding` and `ident`
/// (docs/research/probes.md), so repositories using either go through the git CLI instead.
pub struct Filters<'repo> {
    pipeline: gix::filter::Pipeline<'repo>,
    index: gix::worktree::IndexPersistedOrInMemory,
    root: PathBuf,
    use_git_cli: bool,
}

impl<'repo> Filters<'repo> {
    pub(crate) fn new(repo: &'repo gix::Repository, root: &Path) -> Result<Self, GitError> {
        let (pipeline, index) = repo.filter_pipeline(None).map_err(GitError::gix)?;
        Ok(Self {
            pipeline,
            index,
            root: root.to_owned(),
            use_git_cli: uses_attributes_gix_gets_wrong(root, repo.git_dir()),
        })
    }

    /// Disk content of `path` (relative to the worktree) as git stores it.
    pub fn to_git(&mut self, path: &str, worktree: &[u8]) -> Result<Vec<u8>, GitError> {
        if self.use_git_cli {
            let oid = run_git(&self.root, &["hash-object", "-w", "--path", path, "--stdin"], Some(worktree))?;
            let oid = String::from_utf8_lossy(&oid).trim().to_owned();
            return run_git(&self.root, &["cat-file", "blob", &oid], None);
        }
        let mut outcome = self
            .pipeline
            .convert_to_git(worktree, Path::new(path), &self.index)
            .map_err(GitError::gix)?;
        let mut clean = Vec::with_capacity(worktree.len());
        outcome.read_to_end(&mut clean)?;
        Ok(clean)
    }

    /// Stored content of `path` as it belongs on disk.
    pub fn to_worktree(&mut self, path: &str, clean: &[u8]) -> Result<Vec<u8>, GitError> {
        if self.use_git_cli {
            let oid = run_git(&self.root, &["hash-object", "-w", "--no-filters", "--stdin"], Some(clean))?;
            let oid = String::from_utf8_lossy(&oid).trim().to_owned();
            return run_git(&self.root, &["cat-file", "--filters", &format!("--path={path}"), &oid], None);
        }
        let options = to_worktree::Options {
            can_delay: Delay::Forbid,
            unknown_encoding: to_worktree::UnknownEncoding::Fail,
        };
        let mut outcome = self
            .pipeline
            .convert_to_worktree(clean, path.as_bytes().as_bstr(), options)
            .map_err(GitError::gix)?;
        let mut smudged = Vec::with_capacity(clean.len());
        outcome.read_to_end(&mut smudged)?;
        Ok(smudged)
    }
}

fn uses_attributes_gix_gets_wrong(root: &Path, git_dir: &Path) -> bool {
    let mut candidates = vec![git_dir.join("info").join("attributes")];
    candidates.extend(gitattributes_files(root));
    candidates.into_iter().any(|path| {
        fs::read_to_string(path).is_ok_and(|text| text.contains("working-tree-encoding") || text.contains("ident"))
    })
}

/// Every `.gitattributes` file in the worktree, skipping `.git`.
fn gitattributes_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && entry.file_name() != ".git" {
                stack.push(path);
            } else if kind.is_file() && entry.file_name() == ".gitattributes" {
                found.push(path);
            }
        }
    }
    found
}
