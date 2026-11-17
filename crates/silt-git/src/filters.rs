use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice;
use gix::filter::plumbing::driver::apply::Delay;
use gix::filter::plumbing::pipeline::convert::to_worktree;

use crate::{GitError, run_git, run_git_with};

/// Converts between the content git stores ("clean") and what belongs on
/// disk ("smudged"): line endings, `.gitattributes` and filter drivers.
///
/// gix matches git except for non-UTF-8 `working-tree-encoding` and `ident`
/// (docs/research/probes.md), so repositories using either go through the git CLI instead.
pub struct Filters<'repo> {
    pipeline: gix::filter::Pipeline<'repo>,
    index: gix::worktree::IndexPersistedOrInMemory,
    root: PathBuf,
    objects: PathBuf,
    use_git_cli: bool,
}

impl<'repo> Filters<'repo> {
    pub(crate) fn new(
        pipeline: gix::filter::Pipeline<'repo>,
        index: gix::worktree::IndexPersistedOrInMemory,
        root: &Path,
        common_dir: &Path,
        use_git_cli: bool,
    ) -> Self {
        Self { pipeline, index, root: root.to_owned(), objects: common_dir.join("objects"), use_git_cli }
    }

    /// Runs git with the objects it writes going to a throwaway directory:
    /// the git CLI only converts content by way of an object, which would
    /// otherwise stay in the repository after every save.
    fn git_scratch(&self, scratch: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, GitError> {
        let envs = [
            ("GIT_OBJECT_DIRECTORY".to_owned(), scratch.to_string_lossy().into_owned()),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES".to_owned(), self.objects.to_string_lossy().into_owned()),
        ];
        run_git_with(&self.root, args, stdin, &envs)
    }

    /// Disk content of `path` (relative to the worktree) as git stores it.
    pub fn to_git(&mut self, path: &str, worktree: &[u8]) -> Result<Vec<u8>, GitError> {
        if self.use_git_cli {
            let scratch = tempfile::tempdir()?;
            let oid = self.git_scratch(scratch.path(), &["hash-object", "-w", "--path", path, "--stdin"], Some(worktree))?;
            let oid = String::from_utf8_lossy(&oid).trim().to_owned();
            return self.git_scratch(scratch.path(), &["cat-file", "blob", &oid], None);
        }
        let mut outcome = self.pipeline.convert_to_git(worktree, Path::new(path), &self.index).map_err(GitError::gix)?;
        let mut clean = Vec::with_capacity(worktree.len());
        outcome.read_to_end(&mut clean)?;
        Ok(clean)
    }

    /// Stored content of `path` as it belongs on disk.
    pub fn to_worktree(&mut self, path: &str, clean: &[u8]) -> Result<Vec<u8>, GitError> {
        if self.use_git_cli {
            let scratch = tempfile::tempdir()?;
            let oid = self.git_scratch(scratch.path(), &["hash-object", "-w", "--no-filters", "--stdin"], Some(clean))?;
            let oid = String::from_utf8_lossy(&oid).trim().to_owned();
            return self.git_scratch(scratch.path(), &["cat-file", "--filters", &format!("--path={path}"), &oid], None);
        }
        let options = to_worktree::Options { can_delay: Delay::Forbid, unknown_encoding: to_worktree::UnknownEncoding::Fail };
        let mut outcome = self.pipeline.convert_to_worktree(clean, path.as_bytes().as_bstr(), options).map_err(GitError::gix)?;
        let mut smudged = Vec::with_capacity(clean.len());
        outcome.read_to_end(&mut smudged)?;
        Ok(smudged)
    }
}

/// Whether any attributes file of the repository sets what gix converts
/// differently from git. The worktree's files are found through git, which
/// leaves out ignored directories such as `node_modules`.
pub(crate) fn uses_attributes_gix_gets_wrong(root: &Path, git_dir: &Path) -> bool {
    let mut candidates = vec![git_dir.join("info").join("attributes")];
    let listed =
        run_git(root, &["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", ":(glob)**/.gitattributes"], None)
            .unwrap_or_default();
    candidates.extend(
        listed
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| root.join(String::from_utf8_lossy(path).as_ref())),
    );
    candidates.into_iter().any(|path| fs::read_to_string(path).is_ok_and(|text| sets_attributes_gix_gets_wrong(&text)))
}

/// Whether a rule of an attributes file sets `ident` or a
/// `working-tree-encoding`: the attributes, not words such as "identity".
fn sets_attributes_gix_gets_wrong(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .flat_map(|line| line.split_whitespace().skip(1))
        .any(|attribute| attribute == "ident" || attribute.starts_with("working-tree-encoding="))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_set_attributes_count() {
        assert!(sets_attributes_gix_gets_wrong("*.c ident\n"));
        assert!(sets_attributes_gix_gets_wrong("*.txt text working-tree-encoding=UTF-16LE\n"));
        assert!(!sets_attributes_gix_gets_wrong("# keeps the author's identity\n*.c text\n"));
        assert!(!sets_attributes_gix_gets_wrong("ident text\n*.c -ident\n"));
    }

    #[test]
    fn attributes_files_in_ignored_directories_are_skipped() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        run_git(root, &["init", "-q"], None).expect("init");
        fs::write(root.join(".gitignore"), "target/\n").expect("write");
        fs::create_dir_all(root.join("target/dep")).expect("mkdir");
        fs::write(root.join("target/dep/.gitattributes"), "*.c ident\n").expect("write");
        let git_dir = root.join(".git");
        assert!(!uses_attributes_gix_gets_wrong(root, &git_dir));
        fs::create_dir(root.join("src")).expect("mkdir");
        fs::write(root.join("src/.gitattributes"), "*.c ident\n").expect("write");
        assert!(uses_attributes_gix_gets_wrong(root, &git_dir), "untracked but not ignored");
    }
}
