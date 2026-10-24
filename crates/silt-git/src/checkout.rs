use std::fs;
use std::io::Write;
use std::path::PathBuf;

use silt_types::GitOid;

use crate::{GitError, Repo, run_git};

/// Set in a managed checkout's git config as the last step of making it.
const READY_KEY: &str = "silt.ready";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedCheckout {
    pub path: PathBuf,
    pub git_dir: PathBuf,
}

impl Repo {
    /// Where the managed checkout `id` lives unless placed elsewhere.
    pub fn managed_checkout_path(&self, id: &str) -> PathBuf {
        self.root().join(".silt").join("checkouts").join(id)
    }

    /// Creates a cheap checkout of `base` (ADR 0002): a separate git dir under
    /// the source's `.git/silt/clones` that borrows the source's objects
    /// through alternates, kept safe from `git gc` by `refs/silt/threads/<id>`
    /// in the source. A finished checkout of `id` is returned as it is; what
    /// an interrupted attempt left is cleared and made again.
    pub fn create_managed_checkout(&self, id: &str, base: GitOid, path: Option<PathBuf>) -> Result<ManagedCheckout, GitError> {
        let common_dir = self.common_dir();
        let path = path.unwrap_or_else(|| self.managed_checkout_path(id));
        let git_dir = self.managed_git_dir(id);
        if self.managed_checkout_ready(id) {
            return Ok(ManagedCheckout { path, git_dir });
        }
        // Files are written last, so a checkout with any has been made, and
        // may hold someone's work: checkouts made before the ready mark
        // existed get it rather than being cleared.
        if git_dir.exists() && has_files(&path) {
            run_git(&git_dir, &["config", READY_KEY, "true"], None)?;
            return Ok(ManagedCheckout { path, git_dir });
        }
        for leftover in [&git_dir, &path] {
            if leftover.exists() {
                fs::remove_dir_all(leftover)?;
            }
        }
        self.exclude_silt_dir()?;
        self.protect(id, base)?;

        fs::create_dir_all(git_dir.parent().expect("clone dir has a parent"))?;
        fs::create_dir_all(&path)?;
        run_git(&path, &["init", "-q", &format!("--separate-git-dir={}", git_dir.display())], None)?;
        fs::write(
            git_dir.join("objects").join("info").join("alternates"),
            format!("{}\n", common_dir.join("objects").display()),
        )?;
        if common_dir.join("shallow").exists() {
            fs::copy(common_dir.join("shallow"), git_dir.join("shallow"))?;
        }
        let config = |key: &str, value: &str| run_git(&git_dir, &["config", key, value], None);
        config("core.alternateRefsCommand", ": \"$@\"")?;
        config("core.worktree", &path.to_string_lossy())?;
        if let Some(origin) = self.remote_url("origin")? {
            run_git(&git_dir, &["remote", "add", "origin", &origin], None)?;
        }
        run_git(&git_dir, &["remote", "add", "local", &common_dir.to_string_lossy()], None)?;
        config("remote.local.receivepack", "git -c receive.denyCurrentBranch=updateInstead receive-pack")?;
        let base = base.to_string();
        run_git(&path, &["update-ref", "--no-deref", "HEAD", &base], None)?;
        run_git(&path, &["-c", "checkout.workers=8", "reset", "-q", "--hard", &base], None)?;
        config(READY_KEY, "true")?;
        Ok(ManagedCheckout { path, git_dir })
    }

    /// Whether the managed checkout `id` was made to the end.
    pub fn managed_checkout_ready(&self, id: &str) -> bool {
        let git_dir = self.managed_git_dir(id);
        git_dir.exists() && run_git(&git_dir, &["config", "--get", READY_KEY], None).is_ok()
    }

    fn managed_git_dir(&self, id: &str) -> PathBuf {
        self.common_dir().join("silt").join("clones").join(format!("{id}.git"))
    }

    /// Points `refs/silt/threads/<id>` at `commit`, so everything the thread
    /// depends on stays reachable in the source repository.
    pub fn protect(&self, id: &str, commit: GitOid) -> Result<(), GitError> {
        self.git(&["update-ref", &thread_ref(id), &commit.to_string()], None)?;
        Ok(())
    }

    pub fn remove_managed_checkout(&self, id: &str, checkout: &ManagedCheckout) -> Result<(), GitError> {
        if checkout.path.exists() {
            fs::remove_dir_all(&checkout.path)?;
        }
        if checkout.git_dir.exists() {
            fs::remove_dir_all(&checkout.git_dir)?;
        }
        self.git(&["update-ref", "-d", &thread_ref(id)], None)?;
        Ok(())
    }

    fn common_dir(&self) -> PathBuf {
        let common = self.repo.common_dir();
        silt_os::canonicalize(common).unwrap_or_else(|_| common.to_owned())
    }

    fn exclude_silt_dir(&self) -> Result<(), GitError> {
        let exclude = self.common_dir().join("info").join("exclude");
        let current = fs::read_to_string(&exclude).unwrap_or_default();
        if current.lines().any(|line| line.trim() == "/.silt/") {
            return Ok(());
        }
        fs::create_dir_all(exclude.parent().expect("exclude has a parent"))?;
        let mut file = fs::OpenOptions::new().create(true).append(true).open(&exclude)?;
        if !current.is_empty() && !current.ends_with('\n') {
            writeln!(file)?;
        }
        writeln!(file, "/.silt/")?;
        Ok(())
    }
}

fn thread_ref(id: &str) -> String {
    format!("refs/silt/threads/{id}")
}

/// Whether `dir` holds anything but its `.git` link.
fn has_files(dir: &std::path::Path) -> bool {
    fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|entry| entry.file_name() != ".git"))
}
