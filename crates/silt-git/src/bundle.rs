use std::fs;

use silt_types::GitOid;

use crate::{GitError, Repo};

const BUNDLE_REF: &str = "refs/silt/bundle";

impl Repo {
    pub fn has_commit(&self, commit: GitOid) -> bool {
        self.git(&["cat-file", "-e", &format!("{commit}^{{commit}}")], None).is_ok()
    }

    /// Fetches from origin; failures (offline, no origin) are not errors.
    pub fn fetch_origin(&self) {
        let _ = self.git(&["fetch", "-q", "origin"], None);
    }

    /// A git bundle with `commit` and the history origin's remote-tracking
    /// refs don't reach, or `None` when origin already has all of it.
    pub fn bundle(&self, commit: GitOid) -> Result<Option<Vec<u8>>, GitError> {
        let file = self.git_dir().join("silt-bundle.tmp");
        self.git(&["update-ref", BUNDLE_REF, &commit.to_string()], None)?;
        let created = self.git(&["bundle", "create", "-q", &file.to_string_lossy(), BUNDLE_REF, "--not", "--remotes"], None);
        let _ = self.git(&["update-ref", "-d", BUNDLE_REF], None);
        let bytes = match created {
            Ok(_) => Some(fs::read(&file)?),
            Err(GitError::Command { stderr, .. }) if stderr.contains("empty bundle") => None,
            Err(error) => return Err(error),
        };
        let _ = fs::remove_file(&file);
        Ok(bytes)
    }

    /// Fetches a bundle made by [`Repo::bundle`], keeping its commit
    /// reachable from `refs/silt/threads/<thread>`.
    pub fn fetch_bundle(&self, bytes: &[u8], thread: &str) -> Result<(), GitError> {
        let file = self.git_dir().join("silt-fetch.bundle");
        fs::write(&file, bytes)?;
        let result = self.git(
            &["fetch", "-q", &file.to_string_lossy(), &format!("+{BUNDLE_REF}:refs/silt/threads/{thread}")],
            None,
        );
        let _ = fs::remove_file(&file);
        result.map(|_| ())
    }
}
