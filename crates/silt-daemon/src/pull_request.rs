//! Create PR: the thread's branch goes to `origin` with the person's own
//! git credentials, and the forge's page for a new pull request opens.

use silt_git::Repo;

use crate::core::{Core, CoreError};

/// The remote a thread's pull request goes to.
const REMOTE: &str = "origin";

impl Core {
    /// Pushes the branch the thread's checkout here is on, and answers the
    /// page that opens its pull request. Refused when the checkout isn't on
    /// a branch of its own or has nothing committed past its base.
    pub(crate) async fn open_pull_request(&mut self, selector: &str) -> Result<String, CoreError> {
        let thread = self.find_thread(selector)?;
        let path = self
            .checkouts
            .values()
            .find(|checkout| checkout.record.thread == thread)
            .map(|checkout| checkout.record.path.clone())
            .ok_or_else(|| CoreError::Invalid("this thread has no checkout on this machine".into()))?;
        tokio::task::spawn_blocking(move || {
            let repo = Repo::discover(&path)?;
            let branch = repo
                .head_branch()?
                .ok_or_else(|| CoreError::Invalid("the thread's checkout isn't on a branch; commit to one first".into()))?;
            let remote = repo
                .remote_url(REMOTE)?
                .ok_or_else(|| CoreError::Invalid(format!("the clone has no {REMOTE} to open a pull request on")))?;
            let base = repo.default_branch(REMOTE).unwrap_or_else(|| "main".to_owned());
            if branch == base {
                return Err(CoreError::Invalid(format!(
                    "the thread works on {base} itself; commit to a branch of its own first"
                )));
            }
            if repo.commits_ahead_of(&format!("{REMOTE}/{base}"))? == 0 {
                return Err(CoreError::Invalid(format!("{branch} has nothing committed past {base} yet")));
            }
            let url = silt_git::compare_url(&remote, &base, &branch)
                .ok_or_else(|| CoreError::Invalid(format!("{REMOTE} isn't on a forge with pull requests")))?;
            repo.push_branch(REMOTE, &branch)?;
            Ok(url)
        })
        .await?
    }
}
