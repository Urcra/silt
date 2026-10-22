//! Reads repositories with gix and runs the `git` CLI for writes and for the
//! cases gix doesn't cover, so behaviour matches the user's own git (ADR 0002).

mod bundle;
mod checkout;
pub mod conventions;
mod filters;
mod remote;

use std::cell::Cell;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gix::bstr::ByteSlice;
use silt_types::GitOid;

pub use checkout::ManagedCheckout;
pub use filters::Filters;
pub use remote::normalize_remote;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("{path} is not inside a git repository")]
    NotARepository { path: PathBuf },
    #[error("git operation failed")]
    Gix(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("`git {args}` failed: {stderr}")]
    Command { args: String, stderr: String },
    #[error("could not run git")]
    Spawn(#[from] std::io::Error),
}

impl GitError {
    fn gix(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Gix(Box::new(error))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Regular,
    Executable,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFile {
    pub path: String,
    pub oid: GitOid,
    pub mode: FileMode,
}

pub struct Repo {
    root: PathBuf,
    repo: gix::Repository,
    /// Whether filtering needs the git CLI, as of when `.git/info/attributes`
    /// was last modified then.
    needs_git_cli_filters: Cell<Option<(bool, Option<std::time::SystemTime>)>>,
}

impl Repo {
    /// The repository containing `path`, which must have a worktree.
    pub fn discover(path: &Path) -> Result<Self, GitError> {
        let not_a_repo = || GitError::NotARepository { path: path.to_owned() };
        let repo = gix::discover(path).map_err(|_| not_a_repo())?;
        let root = repo.workdir().ok_or_else(not_a_repo)?.to_owned();
        let root = root.canonicalize().unwrap_or(root);
        Ok(Self { root, repo, needs_git_cli_filters: Cell::new(None) })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn git_dir(&self) -> &Path {
        self.repo.git_dir()
    }

    /// The commit HEAD points at, or `None` in a repository without commits.
    pub fn head_commit(&self) -> Result<Option<GitOid>, GitError> {
        match self.repo.head_id() {
            Ok(id) => Ok(Some(oid(id.detach()))),
            Err(_) if self.repo.head().map_err(GitError::gix)?.is_unborn() => Ok(None),
            Err(error) => Err(GitError::gix(error)),
        }
    }

    pub fn head_branch(&self) -> Result<Option<String>, GitError> {
        let name = self.repo.head_name().map_err(GitError::gix)?;
        Ok(name.map(|name| name.shorten().to_str_lossy().into_owned()))
    }

    /// Local branches, then remote-tracking ones such as `origin/main`, by
    /// their short names.
    pub fn branches(&self) -> Result<Vec<String>, GitError> {
        let references = self.repo.references().map_err(GitError::gix)?;
        let mut local = Vec::new();
        let mut remote = Vec::new();
        for reference in references.local_branches().map_err(GitError::gix)?.flatten() {
            local.push(reference.name().shorten().to_str_lossy().into_owned());
        }
        for reference in references.remote_branches().map_err(GitError::gix)?.flatten() {
            let name = reference.name().shorten().to_str_lossy().into_owned();
            if !name.ends_with("/HEAD") {
                remote.push(name);
            }
        }
        local.sort();
        remote.sort();
        local.extend(remote);
        Ok(local)
    }

    /// The commit `rev` (a branch, tag or commit id) names, if any.
    pub fn commit_of(&self, rev: &str) -> Result<Option<GitOid>, GitError> {
        if rev.starts_with('-') {
            return Ok(None);
        }
        let Ok(out) = self.git(&["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")], None) else {
            return Ok(None);
        };
        Ok(String::from_utf8_lossy(&out).trim().parse().ok())
    }

    /// Whether executable bits on disk mean anything here; git turns this
    /// off where the filesystem can't keep them (WSL's `/mnt/c`).
    pub fn tracks_file_mode(&self) -> bool {
        self.repo.config_snapshot().boolean("core.fileMode").unwrap_or(true)
    }

    /// Whether the filesystem folds case, so `a` and `A` are one file; git
    /// finds out when it makes the repository.
    pub fn ignores_case(&self) -> bool {
        self.repo.config_snapshot().boolean("core.ignoreCase").unwrap_or(false)
    }

    pub fn remote_url(&self, name: &str) -> Result<Option<String>, GitError> {
        let Ok(remote) = self.repo.find_remote(name) else {
            return Ok(None);
        };
        Ok(remote.url(gix::remote::Direction::Fetch).map(|url| url.to_bstring().to_str_lossy().into_owned()))
    }

    /// Every file in `commit`'s tree. Submodules are skipped.
    pub fn files_at(&self, commit: GitOid) -> Result<Vec<TreeFile>, GitError> {
        let commit = self.repo.find_commit(object_id(commit)).map_err(GitError::gix)?;
        let tree = commit.tree().map_err(GitError::gix)?;
        let mut recorder = gix::traverse::tree::Recorder::default();
        tree.traverse().breadthfirst(&mut recorder).map_err(GitError::gix)?;
        Ok(recorder
            .records
            .into_iter()
            .filter_map(|entry| {
                let mode = if entry.mode.is_link() {
                    FileMode::Symlink
                } else if entry.mode.is_executable() {
                    FileMode::Executable
                } else if entry.mode.is_blob() {
                    FileMode::Regular
                } else {
                    return None;
                };
                Some(TreeFile { path: utf8_path(&entry.filepath)?, oid: oid(entry.oid), mode })
            })
            .collect())
    }

    pub fn blob(&self, oid: GitOid) -> Result<Vec<u8>, GitError> {
        let blob = self.repo.find_blob(object_id(oid)).map_err(GitError::gix)?;
        Ok(blob.data.clone())
    }

    /// Files in the worktree that Silt tracks: everything git tracks that
    /// still exists, plus untracked files that aren't ignored.
    pub fn worktree_files(&self) -> Result<Vec<String>, GitError> {
        self.worktree_files_under("")
    }

    /// `worktree_files` under the directory `dir` (relative, `""` for all),
    /// without listing the rest of the worktree.
    pub fn worktree_files_under(&self, dir: &str) -> Result<Vec<String>, GitError> {
        Ok(self.worktree_listing_under(dir)?.into_iter().map(|file| file.path).collect())
    }

    /// `worktree_files_under`, with what each file's `stat` found.
    pub fn worktree_listing_under(&self, dir: &str) -> Result<Vec<ListedFile>, GitError> {
        // Finding untracked files takes git a walk of the worktree; the
        // tracked ones are read from the index and looked at meanwhile.
        let mut args = vec!["ls-files", "-z", "--others", "--exclude-standard"];
        let pathspec = format!(":(literal){dir}");
        if !dir.is_empty() {
            args.extend(["--", &pathspec]);
        }
        let untracked = git_command(&self.root, &args, &[]).stdin(Stdio::null()).spawn()?;
        let index = self.repo.index_or_empty().map_err(GitError::gix)?;
        let prefix = format!("{dir}/");
        let mut paths: Vec<String> = index
            .entries()
            .iter()
            .map(|entry| entry.path(&index).as_bytes())
            .filter(|path| dir.is_empty() || *path == dir.as_bytes() || path.starts_with(prefix.as_bytes()))
            .filter_map(utf8_path)
            .collect();
        paths.dedup();
        let mut found = metadata_of(&self.root, &paths);
        let untracked = finish(untracked, &args)?;
        let others: Vec<String> =
            untracked.split(|byte| *byte == 0).filter(|path| !path.is_empty()).filter_map(utf8_path).collect();
        found.extend(metadata_of(&self.root, &others));
        paths.extend(others);
        let mut listed: Vec<ListedFile> = paths
            .into_iter()
            .zip(found)
            .filter_map(|(path, metadata)| {
                let metadata = metadata?;
                let stat = metadata.is_file().then(|| (metadata.len(), mtime_ns(&metadata)));
                Some(ListedFile { path, stat })
            })
            .collect();
        listed.sort_unstable_by(|one, other| one.path.cmp(&other.path));
        listed.dedup_by(|one, other| one.path == other.path);
        Ok(listed)
    }

    /// Which of `paths` git ignores. Tracked files are never ignored, even
    /// when they match an ignore rule, so only the others are asked about.
    pub fn ignored(&self, paths: &[String]) -> Result<HashSet<String>, GitError> {
        let index = self.repo.index_or_empty().map_err(GitError::gix)?;
        let untracked: Vec<&String> =
            paths.iter().filter(|path| index.entry_by_path(path.as_bytes().as_bstr()).is_none()).collect();
        if untracked.is_empty() {
            return Ok(HashSet::new());
        }
        let mut input = Vec::new();
        for path in untracked {
            input.extend_from_slice(path.as_bytes());
            input.push(0);
        }
        let output = match self.git(&["check-ignore", "-z", "--stdin"], Some(&input)) {
            Ok(output) => output,
            Err(GitError::Command { stderr, .. }) if stderr.is_empty() => Vec::new(),
            Err(error) => return Err(error),
        };
        Ok(output
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect())
    }

    /// Content conversion for this repository. Which attributes its
    /// `.gitattributes` files use is looked up once; call
    /// [`Repo::attributes_changed`] when one of them changes.
    pub fn filters(&self) -> Result<Filters<'_>, GitError> {
        // Not a worktree file, so no rescan sees it change.
        let info = std::fs::metadata(self.repo.git_dir().join("info").join("attributes")).and_then(|info| info.modified()).ok();
        let use_git_cli = match self.needs_git_cli_filters.get() {
            Some((known, seen)) if seen == info => known,
            _ => {
                let found = filters::uses_attributes_gix_gets_wrong(&self.root, self.repo.git_dir());
                self.needs_git_cli_filters.set(Some((found, info)));
                found
            }
        };
        Filters::new(&self.repo, &self.root, use_git_cli)
    }

    pub fn attributes_changed(&self) {
        self.needs_git_cli_filters.set(None);
    }

    /// Runs `git` in the worktree, failing on a non-zero exit.
    pub fn git(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, GitError> {
        run_git(&self.root, args, stdin)
    }

    /// Runs git with extra configuration given through the environment, so
    /// values such as tokens don't show up in process listings.
    pub fn git_with_config(&self, args: &[&str], config: &[(&str, &str)]) -> Result<Vec<u8>, GitError> {
        let mut envs = vec![("GIT_CONFIG_COUNT".to_owned(), config.len().to_string())];
        for (index, (key, value)) in config.iter().enumerate() {
            envs.push((format!("GIT_CONFIG_KEY_{index}"), (*key).to_owned()));
            envs.push((format!("GIT_CONFIG_VALUE_{index}"), (*value).to_owned()));
        }
        run_git_with(&self.root, args, None, &envs)
    }
}

pub(crate) fn run_git(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, GitError> {
    run_git_with(dir, args, stdin, &[])
}

/// Settings for every git command Silt runs. A checkout holds other
/// people's and agents' work, and git would run hooks and an fsmonitor
/// named in its config; Silt's own git commands run neither (ADR 0016).
const HARDENED: [&str; 4] = ["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"];

pub(crate) fn run_git_with(
    dir: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    envs: &[(String, String)],
) -> Result<Vec<u8>, GitError> {
    let mut child = git_command(dir, args, envs).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).spawn()?;
    // Fed from another thread: git writes output while it still reads
    // input, and both pipes would fill up.
    let feeding = stdin.map(|input| {
        let mut pipe = child.stdin.take().expect("stdin is piped");
        let input = input.to_vec();
        std::thread::spawn(move || pipe.write_all(&input))
    });
    let output = finish(child, args);
    if let Some(feeding) = feeding {
        feeding.join().map_err(|_| std::io::Error::other("feeding git panicked"))??;
    }
    output
}

/// git run in `dir` with Silt's settings and `envs`, its output piped.
fn git_command(dir: &Path, args: &[&str], envs: &[(String, String)]) -> Command {
    let mut command = Command::new("git");
    command
        .args(HARDENED)
        .arg("-C")
        .arg(dir)
        .args(args)
        .envs(envs.iter().map(|(key, value)| (key, value)))
        // The daemon has no terminal: a prompt would wait forever.
        .env("GIT_TERMINAL_PROMPT", "0")
        // Messages Silt reads must not be translated.
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    command
}

/// What a git process printed, once it exited well.
fn finish(child: std::process::Child, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(GitError::Command {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output.stdout)
}

/// A file in the worktree, as `Repo::worktree_listing_under` found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedFile {
    pub path: String,
    /// A regular file's size and `mtime_ns`; `None` for anything else, such
    /// as a symlink.
    pub stat: Option<(u64, i64)>,
}

/// A file's modification time in nanoseconds since the epoch, 0 if unknown.
pub fn mtime_ns(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX))
}

/// Each path's `symlink_metadata` under `root`, if it's there: on several
/// threads for a large worktree, whose files are mostly waiting on the
/// kernel one `stat` at a time.
fn metadata_of(root: &Path, paths: &[String]) -> Vec<Option<std::fs::Metadata>> {
    const PER_THREAD: usize = 2048;
    let stat = |paths: &[String]| paths.iter().map(|path| root.join(path).symlink_metadata().ok()).collect::<Vec<_>>();
    if paths.len() <= PER_THREAD {
        return stat(paths);
    }
    std::thread::scope(|scope| {
        let workers: Vec<_> = paths.chunks(PER_THREAD).map(|chunk| scope.spawn(move || stat(chunk))).collect();
        workers.into_iter().flat_map(|worker| worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect()
    })
}

/// A path Silt can track. Paths aren't UTF-8 text in the family, so other
/// names are left alone: a lossy copy would be a different file, and the
/// real one would look deleted.
fn utf8_path(path: &[u8]) -> Option<String> {
    match std::str::from_utf8(path) {
        Ok(path) => Some(path.to_owned()),
        Err(_) => {
            tracing::warn!(path = %path.to_str_lossy(), "leaving out a file whose name isn't UTF-8");
            None
        }
    }
}

fn oid(id: gix::ObjectId) -> GitOid {
    let mut bytes = [0u8; 20];
    bytes.copy_from_slice(id.as_bytes());
    GitOid(bytes)
}

fn object_id(oid: GitOid) -> gix::ObjectId {
    gix::ObjectId::from_bytes_or_panic(&oid.0)
}
