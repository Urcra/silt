//! Create PR: the thread's branch goes to `origin` with the person's own
//! git credentials, and the forge's page for a new pull request opens.

use std::path::{Path, PathBuf};

use silt_git::Repo;

use crate::core::{Core, CoreError};

/// The remote a thread's pull request goes to.
const REMOTE: &str = "origin";

impl Core {
    /// Pushes the branch the thread's checkout here is on, and answers the
    /// page that opens its pull request. Refused when the checkout isn't on
    /// a branch of its own or has nothing committed past its base.
    pub(crate) async fn open_pull_request(&mut self, selector: &str) -> Result<String, CoreError> {
        let path = self.pull_request_checkout(selector)?;
        tokio::task::spawn_blocking(move || {
            let (repo, branch, url) = plan(&path)?;
            repo.push_branch(REMOTE, &branch)?;
            Ok(url)
        })
        .await?
    }

    /// Why the thread can't open a pull request now, or none when it can:
    /// what [`Self::open_pull_request`] checks, without pushing.
    pub(crate) async fn pull_request_blocker(&self, selector: &str) -> Result<Option<String>, CoreError> {
        let path = match self.pull_request_checkout(selector) {
            Ok(path) => path,
            Err(CoreError::Invalid(why)) => return Ok(Some(why)),
            Err(error) => return Err(error),
        };
        Ok(match tokio::task::spawn_blocking(move || plan(&path).map(|_| ())).await? {
            Ok(()) => None,
            Err(CoreError::Invalid(why)) => Some(why),
            Err(error) => Some(error.to_string()),
        })
    }

    fn pull_request_checkout(&self, selector: &str) -> Result<PathBuf, CoreError> {
        let thread = self.find_thread(selector)?;
        self.checkouts
            .values()
            .find(|checkout| checkout.record.thread == thread)
            .map(|checkout| checkout.record.path.clone())
            .ok_or_else(|| CoreError::Invalid("this thread has no checkout on this machine".into()))
    }
}

/// The checkout's repository, the branch to push and the forge's page for
/// its pull request, or why there's none.
fn plan(path: &Path) -> Result<(Repo, String, String), CoreError> {
    let repo = Repo::discover(path)?;
    let branch = repo
        .head_branch()?
        .ok_or_else(|| CoreError::Invalid("the thread's checkout isn't on a branch; commit to one first".into()))?;
    let remote = repo
        .remote_url(REMOTE)?
        .ok_or_else(|| CoreError::Invalid(format!("the clone has no {REMOTE} to open a pull request on")))?;
    let base = repo.default_branch(REMOTE).unwrap_or_else(|| "main".to_owned());
    if branch == base {
        return Err(CoreError::Invalid(format!("the thread works on {base} itself; commit to a branch of its own first")));
    }
    let url = silt_git::compare_url(&remote, &base, &branch)
        .ok_or_else(|| CoreError::Invalid(format!("{REMOTE} isn't on a forge with pull requests")))?;
    if repo.commits_ahead_of(&format!("{REMOTE}/{base}"))? == 0 {
        return Err(CoreError::Invalid(format!("{branch} has nothing committed past {base} yet")));
    }
    Ok((repo, branch, url))
}
