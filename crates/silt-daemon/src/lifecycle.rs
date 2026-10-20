//! The trash and purging (ADR 0013): a deleted thread is hidden on every
//! device until it is restored, and a purged one is dropped everywhere,
//! keeping only what other threads build on.

use std::collections::HashSet;

use silt_git::Repo;
use silt_log::{Family, kept_after_purge};
use silt_sync::ClientSync;
use silt_types::{FamilyId, ThreadId};
use tracing::warn;

use crate::core::{Core, CoreError, RepoBases, now_ms};
use crate::protocol::ThreadSummary;

/// What happens to a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    Delete,
    Restore,
    Purge,
}

impl Lifecycle {
    fn action(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Restore => "restore",
            Self::Purge => "purge",
        }
    }
}

impl Core {
    pub(crate) async fn thread_lifecycle(&mut self, selector: &str, change: Lifecycle) -> Result<String, CoreError> {
        let thread = self.find_thread(selector)?;
        let deleted = self.deleted.contains(&thread);
        if change == Lifecycle::Purge && !deleted {
            return Err(CoreError::Invalid(
                "delete the thread first (`silt thread delete`); purging works on the trash".into(),
            ));
        }
        if let Some(remote) = &self.remote {
            remote
                .thread_lifecycle(thread, change.action())
                .await
                .map_err(|error| CoreError::Invalid(error.to_string()))?;
        }
        let short = crate::forks::short(thread);
        match change {
            Lifecycle::Delete => {
                self.mark_deleted(thread, true).await?;
                Ok(format!("thread {short} is in the trash; `silt thread restore {short}` brings it back"))
            }
            Lifecycle::Restore => {
                self.mark_deleted(thread, false).await?;
                Ok(format!("thread {short} is back"))
            }
            Lifecycle::Purge => {
                self.drop_threads(&HashSet::from([thread])).await?;
                Ok(format!("thread {short} is purged for good"))
            }
        }
    }

    pub(crate) async fn mark_deleted(&mut self, thread: ThreadId, deleted: bool) -> Result<(), CoreError> {
        let changed = if deleted { self.deleted.insert(thread) } else { self.deleted.remove(&thread) };
        if changed {
            self.store.set_thread_deleted(thread, deleted.then(now_ms)).await?;
        }
        Ok(())
    }

    /// The threads in this machine's trash.
    pub(crate) fn trash(&self) -> Vec<ThreadSummary> {
        self.summaries(|thread| self.deleted.contains(&thread))
    }

    /// Follows the server's trash: threads deleted or restored elsewhere,
    /// and threads purged for good.
    pub(crate) async fn follow_trash(&mut self, deleted: &[ThreadId], purged: &[ThreadId], listed: &HashSet<ThreadId>) {
        let purged: HashSet<ThreadId> = purged.iter().copied().filter(|thread| self.threads.contains_key(thread)).collect();
        if !purged.is_empty()
            && let Err(error) = self.drop_threads(&purged).await
        {
            warn!(%error, "could not drop purged threads");
        }
        let deleted: HashSet<ThreadId> = deleted.iter().copied().collect();
        let known: Vec<ThreadId> = self.threads.keys().copied().collect();
        for thread in known {
            let result = if deleted.contains(&thread) {
                self.mark_deleted(thread, true).await
            } else if listed.contains(&thread) {
                self.mark_deleted(thread, false).await
            } else {
                Ok(())
            };
            if let Err(error) = result {
                warn!(%error, "could not follow the trash");
            }
        }
    }

    /// Forgets threads here and every delta of theirs that no remaining
    /// thread builds on.
    pub(crate) async fn drop_threads(&mut self, threads: &HashSet<ThreadId>) -> Result<(), CoreError> {
        let families: HashSet<FamilyId> = threads
            .iter()
            .filter_map(|thread| self.threads.get(thread).map(|record| record.family))
            .collect();
        let checkouts: Vec<i64> = self
            .checkouts
            .values()
            .filter(|checkout| threads.contains(&checkout.record.thread))
            .map(|checkout| checkout.record.id)
            .collect();
        for checkout in checkouts {
            self.checkouts.remove(&checkout);
        }
        self.sessions.retain(|session| !threads.contains(&session.record.thread));
        self.pending.retain(|pending| !threads.contains(&pending.thread));
        for thread in threads {
            self.notices.remove(thread);
            self.deleted.remove(thread);
            self.threads.remove(thread);
            self.store.remove_thread(*thread).await?;
        }
        for family in families {
            let remaining = self.threads.values().filter(|record| record.family == family).count();
            if remaining == 0 {
                self.families.remove(&family);
                self.raw.remove(&family);
                self.syncs.remove(&family);
                self.store.clear_deltas(family).await?;
                continue;
            }
            let log = self.local_log(family).to_vec();
            let keep = kept_after_purge(&log, threads);
            let kept: Vec<_> = log.into_iter().zip(keep).filter(|(_, kept)| *kept).map(|(delta, _)| delta).collect();
            self.store.replace_deltas(family, &kept).await?;
            self.syncs.insert(family, ClientSync::default());
            if self.families.contains_key(&family) {
                let repo = self
                    .threads
                    .values()
                    .filter(|record| record.family == family && !record.repo_root.as_os_str().is_empty())
                    .find_map(|record| Repo::discover(&record.repo_root).ok());
                let Some(repo) = repo else {
                    self.families.remove(&family);
                    self.raw.insert(family, kept);
                    continue;
                };
                let mut projected = Family::new();
                for delta in kept {
                    if let Err(error) = projected.apply(delta, &mut RepoBases(&repo)) {
                        warn!(%error, "skipping a delta that no longer applies");
                    }
                }
                self.families.insert(family, projected);
            } else {
                self.raw.insert(family, kept);
            }
        }
        Ok(())
    }
}
