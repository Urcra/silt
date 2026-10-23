//! One round of pushing to the server without the daemon (ADR 0018), which
//! a hook starts at `Stop` and the wrapper when its agent exits: record the
//! commits made in the checkouts, then push the families' deltas and blobs.
//! Pulling is left to the daemon. Writing takes turns as capturing does,
//! but the server is talked to between turns, so hooks never wait on it.

use std::collections::HashSet;
use std::path::Path;

use silt_store::Store;
use silt_sync::{ClientSync, Outcome};
use silt_types::FamilyId;

use crate::capture::turn_in_time;
use crate::capture_lock::CaptureLock;
use crate::core::{Core, CoreError, stored_entries};
use crate::load;
use crate::sync::{PushAnswer, send_push, thread_info};

/// What a round pushed.
#[derive(Debug, Default)]
pub struct Synced {
    pub families: usize,
    pub deltas: usize,
}

/// Pushes what `thread`'s family recorded here, or every family's without
/// one, for the data in `data_dir`.
pub async fn sync_once(data_dir: &Path, thread: Option<&str>) -> Result<Synced, CoreError> {
    let lock = CaptureLock::open(data_dir)?;
    let store = Store::open(data_dir).await?;
    let actor = crate::server::load_actor(data_dir)?;
    let mut core = Core::visiting(store, actor, data_dir).await?;
    if core.remotes.is_empty() {
        return Err(CoreError::Invalid("not connected to a server; use `silt auth login`".into()));
    }
    let families: Vec<FamilyId> = match thread {
        Some(thread) => vec![core.threads[&core.find_thread(thread)?].family],
        None => {
            let all: HashSet<FamilyId> = core.threads.values().map(|thread| thread.family).collect();
            all.into_iter().collect()
        }
    };
    core.read_notes_settings().await;
    {
        let _turn = turn_in_time(&lock).await?;
        for family in &families {
            core.hold_family(*family).await?;
        }
        core.check_commits().await?;
        core.end_turn().await?;
    }
    // Each family goes only to its own server (ADR 0032); one kept on this
    // machine goes nowhere.
    let families: Vec<(FamilyId, crate::remote::Remote)> =
        families.into_iter().filter_map(|family| core.remote_of(family).cloned().map(|remote| (family, remote))).collect();
    let mut servers: Vec<crate::remote::Remote> = Vec::new();
    for (_, remote) in &families {
        if !servers.iter().any(|known| known.config.url == remote.config.url) {
            servers.push(remote.clone());
        }
    }
    for remote in &servers {
        let listing = remote.listing().await.map_err(|source| CoreError::Server { doing: "list threads", source })?;
        let listed: HashSet<_> = listing.threads.iter().map(|thread| thread.id).collect();
        let unlisted = core.threads.values().filter(|record| {
            !listed.contains(&record.id)
                && families.iter().any(|(family, home)| *family == record.family && home.config.url == remote.config.url)
        });
        for record in unlisted {
            remote
                .register_thread(&thread_info(record), None, crate::sync::origin_url(record))
                .await
                .map_err(|source| CoreError::Server { doing: "register the thread", source })?;
        }
    }
    let mut synced = Synced { families: families.len(), deltas: 0 };
    for (family, remote) in families {
        loop {
            let batch = {
                let _turn = turn_in_time(&lock).await?;
                core.catch_up_with_store().await?;
                core.end_turn().await?;
                core.next_push(family)
            };
            let Some(batch) = batch else {
                break;
            };
            let sent = batch.len();
            let PushAnswer { family, epoch, generation, outcome, error } =
                send_push(&remote, &core.store, &core.uploaded, batch).await;
            let failed = (!matches!(outcome, Outcome::Pushed))
                .then(|| error.clone().unwrap_or_else(|| "the server refused the deltas".to_owned()));
            let _turn = turn_in_time(&lock).await?;
            core.catch_up_with_store().await?;
            core.pushed(family, epoch, generation, outcome, error).await;
            core.end_turn().await?;
            if let Some(error) = failed {
                return Err(CoreError::Push(error));
            }
            synced.deltas += sent;
        }
    }
    Ok(synced)
}

impl Core {
    /// Takes on the family here with its checkouts and sync progress.
    pub(crate) async fn hold_family(&mut self, family_id: FamilyId) -> Result<(), CoreError> {
        let repo = self.family_repo(family_id).ok();
        let loaded = load::family(&self.store, family_id, repo.as_ref()).await?;
        self.take_loaded(family_id, loaded);
        for record in self.store.checkouts().await? {
            let ours = self.threads.get(&record.thread).is_some_and(|thread| thread.family == family_id);
            if ours && record.path.exists() {
                let entries = stored_entries(&self.store, record.id).await?;
                self.start_checkout(record, entries)?;
            }
        }
        let state = self.store.sync_state(family_id).await?;
        self.syncs
            .insert(family_id, ClientSync::new(state.generation, state.pushed, state.pulled, state.last_pulled, state.view));
        Ok(())
    }
}
