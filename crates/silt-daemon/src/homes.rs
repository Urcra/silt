//! Where each family syncs (ADR 0032): one of the servers this machine is
//! signed in to, or this machine only.

use std::collections::BTreeSet;

use silt_types::{DeviceId, FamilyId, ThreadId};

use crate::core::{Core, CoreError};
use crate::remote::Remote;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Home {
    Server(String),
    Local,
}

impl Home {
    /// As the store keeps it: the server's URL, or empty for this machine.
    pub(crate) fn from_stored(text: String) -> Self {
        if text.is_empty() { Self::Local } else { Self::Server(text) }
    }

    pub(crate) fn stored(&self) -> &str {
        match self {
            Self::Server(url) => url,
            Self::Local => "",
        }
    }

    /// As people name it: the server's URL, or `local`.
    pub(crate) fn shown(&self) -> &str {
        match self {
            Self::Server(url) => url,
            Self::Local => "local",
        }
    }
}

impl Core {
    /// The server a family syncs with: its home, or the default for a
    /// family given none. `None` for this machine only, or for a home
    /// signed out of.
    pub(crate) fn remote_of(&self, family: FamilyId) -> Option<&Remote> {
        match self.homes.get(&family) {
            Some(Home::Server(url)) => self.remotes.iter().find(|remote| remote.config.url == *url),
            Some(Home::Local) => None,
            None => self.remotes.first(),
        }
    }

    /// [`Self::remote_of`] the thread's family.
    pub(crate) fn remote_for(&self, thread: ThreadId) -> Option<&Remote> {
        self.remote_of(self.threads.get(&thread)?.family)
    }

    /// The server a thread syncs with, for what needs one.
    pub(crate) fn server_for(&self, thread: ThreadId) -> Result<Remote, CoreError> {
        if self.threads.get(&thread).is_some_and(|record| self.homes.get(&record.family) == Some(&Home::Local)) {
            return Err(CoreError::Invalid(format!("thread {} stays on this machine only", thread.short())));
        }
        self.remote_for(thread)
            .cloned()
            .ok_or_else(|| CoreError::Invalid("not connected to a server; use `silt auth login`".into()))
    }

    /// Whether the thread syncs with the server at `url`.
    pub(crate) fn homed_at(&self, thread: ThreadId, url: &str) -> bool {
        self.remote_for(thread).is_some_and(|remote| remote.config.url == url)
    }

    /// The first server signed in to, where new projects' threads go.
    pub(crate) fn default_remote(&self) -> Option<&Remote> {
        self.remotes.first()
    }

    /// The server what belongs to no thread is asked of: the one the request
    /// names with `Request::OnServer`, or the default.
    pub(crate) fn asked_remote(&self) -> Option<&Remote> {
        match &self.asked_server {
            Some(url) => self.remotes.iter().find(|remote| remote.config.url == *url),
            None => self.default_remote(),
        }
    }

    /// The servers what belongs to no thread and spans them, such as the
    /// inbox, is read from: the one the request names, or all of them.
    pub(crate) fn asked_remotes(&self) -> Result<Vec<Remote>, CoreError> {
        let remotes: Vec<Remote> = match &self.asked_server {
            Some(_) => self.asked_remote().into_iter().cloned().collect(),
            None => self.remotes.clone(),
        };
        if remotes.is_empty() {
            return Err(CoreError::Invalid("not connected to a server; use `silt auth login`".into()));
        }
        Ok(remotes)
    }

    /// This machine's user on the thread's server; empty without one.
    pub(crate) fn own_user(&self, thread: ThreadId) -> String {
        self.remote_for(thread).map(|remote| remote.config.user.clone()).unwrap_or_default()
    }

    /// This machine's user on the family's server; empty without one.
    pub(crate) fn own_user_in(&self, family: FamilyId) -> String {
        self.remote_of(family).map(|remote| remote.config.user.clone()).unwrap_or_default()
    }

    /// This machine as a device of the thread's server.
    pub(crate) fn device_for(&self, thread: ThreadId) -> Option<DeviceId> {
        self.remote_for(thread).map(|remote| remote.config.device)
    }

    /// Where a new family of the project `repo` goes: the project's home,
    /// else the default server. `None` with neither, so the family syncs
    /// with the first server signed in to later.
    pub(crate) fn new_home(&self, repo: &str) -> Option<Home> {
        self.project_homes
            .get(repo)
            .cloned()
            .or_else(|| self.default_remote().map(|remote| Home::Server(remote.config.url.clone())))
    }

    /// The server a project's new threads sync with, for what needs one.
    pub(crate) fn server_for_project(&self, repo: &str) -> Result<Remote, CoreError> {
        match self.new_home(repo) {
            Some(Home::Server(url)) => self.remotes.iter().find(|remote| remote.config.url == url).cloned(),
            Some(Home::Local) => return Err(CoreError::Invalid(format!("project {repo} stays on this machine only"))),
            None => None,
        }
        .ok_or_else(|| CoreError::Invalid("not connected to a server; use `silt auth login`".into()))
    }

    /// Sets where the project's new threads go: a server signed in to, or
    /// `local` for this machine only. Its threads kept on this machine move
    /// to a new server home, and from then on sync there; threads already
    /// on a server stay where they are, as others may hold them there.
    pub(crate) async fn set_project_home(&mut self, repo: &str, home: &str) -> Result<String, CoreError> {
        let home = if home == "local" {
            Home::Local
        } else {
            let url = home.trim_end_matches('/');
            if !self.remotes.iter().any(|remote| remote.config.url == url) {
                return Err(CoreError::Invalid(format!("not signed in to {url}; run `silt auth login --server {url}` first")));
            }
            Home::Server(url.to_owned())
        };
        self.store.set_project_home(repo, home.stored()).await?;
        self.project_homes.insert(repo.to_owned(), home.clone());
        let held = self.held_imports().await?;
        let families: BTreeSet<FamilyId> = self
            .threads
            .values()
            .filter(|record| record.repo == repo && !held.contains(&record.family))
            .map(|record| record.family)
            .collect();
        let mut moved = 0;
        let mut staying = BTreeSet::new();
        for family in families {
            match (&home, self.homes.get(&family)) {
                (Home::Server(_), Some(Home::Local)) => {
                    self.set_home(family, home.clone()).await?;
                    moved += self.threads.values().filter(|record| record.family == family).count();
                }
                _ => {
                    if let Some(remote) = self.remote_of(family) {
                        staying.insert(remote.config.url.clone());
                    }
                }
            }
        }
        self.drive_sync();
        self.announce(crate::protocol::BusEvent::Threads);
        let mut said = match &home {
            Home::Local => format!("new threads of {repo} stay on this machine only"),
            Home::Server(url) => format!("new threads of {repo} sync with {url}"),
        };
        if moved > 0 {
            said.push_str(&format!("; {moved} threads kept on this machine now sync there too"));
        }
        if !staying.is_empty() {
            let servers: Vec<String> = staying.into_iter().collect();
            said.push_str(&format!("; its threads already on {} stay there", servers.join(" and ")));
        }
        Ok(said)
    }

    /// Gives a family a home, unless it has one.
    pub(crate) async fn settle_home(&mut self, family: FamilyId, home: Home) -> Result<(), CoreError> {
        if self.homes.contains_key(&family) {
            return Ok(());
        }
        self.set_home(family, home).await
    }

    pub(crate) async fn set_home(&mut self, family: FamilyId, home: Home) -> Result<(), CoreError> {
        self.store.set_family_home(family, home.stored()).await?;
        self.homes.insert(family, home);
        Ok(())
    }
}
