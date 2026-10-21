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

    /// A git bundle with `commit` and the history that neither origin's
    /// remote-tracking refs nor `known` (commits bundled before) reach, or
    /// `None` when nothing is left. A managed checkout has no remote-tracking
    /// refs of its own, so without `known` every bundle would carry the
    /// whole history.
    pub fn bundle(&self, commit: GitOid, known: &[GitOid]) -> Result<Option<Vec<u8>>, GitError> {
        let file = self.git_dir().join(unique_name("silt-bundle"));
        self.git(&["update-ref", BUNDLE_REF, &commit.to_string()], None)?;
        let known: Vec<String> = known.iter().filter(|known| self.has_commit(**known)).map(GitOid::to_string).collect();
        let path = file.to_string_lossy();
        let mut args = vec!["bundle", "create", "-q", path.as_ref(), BUNDLE_REF, "--not", "--remotes"];
        args.extend(known.iter().map(String::as_str));
        let created = self.git(&args, None);
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
        let file = self.git_dir().join(unique_name("silt-fetch"));
        fs::write(&file, bytes)?;
        let result =
            self.git(&["fetch", "-q", &file.to_string_lossy(), &format!("+{BUNDLE_REF}:refs/silt/threads/{thread}")], None);
        let _ = fs::remove_file(&file);
        result.map(|_| ())
    }

    /// Pushes `commit` to a Silt server's git mirror as
    /// `refs/silt/threads/<thread>`, sending `headers` with every request.
    pub fn push_to_mirror(&self, url: &str, headers: &[String], commit: GitOid, thread: &str) -> Result<(), GitError> {
        let refspec = format!("+{commit}:refs/silt/threads/{thread}");
        self.git_with_config(&["push", "-q", "--no-verify", url, &refspec], &mirror_config(headers)).map(|_| ())
    }

    /// Fetches every thread ref of a Silt server's git mirror into
    /// `refs/silt/mirror/`.
    pub fn fetch_from_mirror(&self, url: &str, headers: &[String]) -> Result<(), GitError> {
        self.git_with_config(&["fetch", "-q", url, "+refs/silt/threads/*:refs/silt/mirror/*"], &mirror_config(headers))
            .map(|_| ())
    }
}

/// A file name no other bundle being made or fetched at the same time uses.
fn unique_name(prefix: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{prefix}-{}-{next}.bundle", std::process::id())
}

fn mirror_config(headers: &[String]) -> Vec<(&str, &str)> {
    headers.iter().map(|header| ("http.extraHeader", header.as_str())).collect()
}
