//! Offering a thread's branch as a forge pull request: the branch goes to
//! the remote with the person's own git credentials, and the forge's page
//! for a new pull request opens with it chosen, as `git push` itself
//! suggests. Silt needs no forge token for it.

use crate::{GitError, Repo, normalize_remote};

/// The page where the forge that `remote` names opens a pull request of
/// `branch` into `base`; none for a remote not on a web host.
pub fn compare_url(remote: &str, base: &str, branch: &str) -> Option<String> {
    let remote = normalize_remote(remote);
    let (host, path) = remote.split_once('/')?;
    if !host.contains('.') || path.is_empty() {
        return None;
    }
    Some(if host.contains("gitlab") {
        format!(
            "https://{host}/{path}/-/merge_requests/new?merge_request%5Bsource_branch%5D={branch}&merge_request%5Btarget_branch%5D={base}"
        )
    } else {
        // GitHub's form, which Forgejo and Gitea answer too.
        format!("https://{host}/{path}/compare/{base}...{branch}?expand=1")
    })
}

impl Repo {
    /// The branch `remote`'s HEAD names, such as `main`.
    pub fn default_branch(&self, remote: &str) -> Option<String> {
        let head = self.git(&["symbolic-ref", "--quiet", "--short", &format!("refs/remotes/{remote}/HEAD")], None).ok()?;
        let head = String::from_utf8_lossy(&head).trim().to_owned();
        head.strip_prefix(&format!("{remote}/")).map(str::to_owned)
    }

    /// How many commits HEAD has that `base` lacks.
    pub fn commits_ahead_of(&self, base: &str) -> Result<u64, GitError> {
        let count = self.git(&["rev-list", "--count", &format!("{base}..HEAD")], None)?;
        Ok(String::from_utf8_lossy(&count).trim().parse().unwrap_or(0))
    }

    /// Pushes `branch` to `remote`, tracking it there.
    pub fn push_branch(&self, remote: &str, branch: &str) -> Result<(), GitError> {
        self.git(&["push", "--quiet", "--set-upstream", remote, &format!("refs/heads/{branch}:refs/heads/{branch}")], None)
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;

    #[test]
    fn each_forge_opens_its_own_page() {
        assert_eq!(
            compare_url("git@github.com:acme/web.git", "main", "silt/fix-upload").as_deref(),
            Some("https://github.com/acme/web/compare/main...silt/fix-upload?expand=1")
        );
        assert_eq!(
            compare_url("https://gitlab.com/acme/group/web", "main", "fix").as_deref(),
            Some(
                "https://gitlab.com/acme/group/web/-/merge_requests/new?merge_request%5Bsource_branch%5D=fix&merge_request%5Btarget_branch%5D=main"
            )
        );
        assert_eq!(
            compare_url("https://codeberg.org/acme/web.git", "trunk", "fix").as_deref(),
            Some("https://codeberg.org/acme/web/compare/trunk...fix?expand=1")
        );
        assert_eq!(compare_url("/srv/git/web.git", "main", "fix"), None);
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@example.com", "-c", "init.defaultBranch=main"])
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn a_branch_with_commits_past_its_base_is_pushed() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (remote, work) = (dir.path().join("remote.git"), dir.path().join("work"));
        git(dir.path(), &["init", "--quiet", "--bare", remote.to_str().unwrap()]);
        git(dir.path(), &["clone", "--quiet", remote.to_str().unwrap(), work.to_str().unwrap()]);
        std::fs::write(work.join("a"), "a\n").unwrap();
        git(&work, &["add", "a"]);
        git(&work, &["commit", "--quiet", "-m", "first"]);
        git(&work, &["push", "--quiet", "origin", "main"]);
        git(&work, &["remote", "set-head", "origin", "main"]);
        git(&work, &["switch", "--quiet", "-c", "fix"]);
        std::fs::write(work.join("a"), "b\n").unwrap();
        git(&work, &["commit", "--quiet", "-am", "fix"]);

        let repo = Repo::discover(&work).expect("a repo");
        assert_eq!(repo.default_branch("origin").as_deref(), Some("main"));
        assert_eq!(repo.commits_ahead_of("origin/main").expect("counted"), 1);
        repo.push_branch("origin", "fix").expect("pushed");
        let pushed = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", "refs/heads/fix"])
            .current_dir(&remote)
            .output()
            .unwrap();
        assert!(pushed.status.success(), "the remote has the branch");
    }
}
