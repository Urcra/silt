use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use silt_agents::claude;
use silt_crdt::InodeId;
use silt_log::{Applied, Content, ConvOp, Delta, GitOp, Op};
use silt_proto::{Listing, ThreadInfo};
use silt_store::{SyncState, ThreadRecord};
use silt_sync::{ClientSync, Effect, Outcome};
use silt_types::{BlobHash, DeltaId, FamilyId, ThreadId};
use tracing::warn;

use crate::core::{Core, CoreError, Event, RepoBases, touched_inode};

const LISTING_INTERVAL: Duration = Duration::from_secs(3);
const PULL_WAIT: Duration = Duration::from_secs(20);

impl Core {
    /// Every delta of the family in local order, projected or not.
    pub(crate) fn local_log(&self, family: FamilyId) -> &[Delta] {
        match self.families.get(&family) {
            Some(family) => family.log(),
            None => self.raw.get(&family).map_or(&[], Vec::as_slice),
        }
    }

    fn family_ids(&self) -> Vec<FamilyId> {
        let mut ids: Vec<FamilyId> = self.families.keys().chain(self.raw.keys()).copied().collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Starts any push, pull or listing that is due.
    pub(crate) fn drive_sync(&mut self) {
        let Some(remote) = self.remote.clone() else {
            return;
        };
        if self.last_listing.is_none_or(|last| last.elapsed() >= LISTING_INTERVAL) {
            self.last_listing = Some(Instant::now());
            let events = self.events.clone();
            let remote = remote.clone();
            tokio::spawn(async move {
                let listing = remote.listing().await.map_err(|error| error.to_string());
                let _ = events.send(Event::Listed(listing));
            });
        }
        for family in self.family_ids() {
            let local_len = self.local_log(family).len() as u64;
            let sync = self.syncs.entry(family).or_default();
            if let Some(push) = sync.next_push(local_len) {
                let batch: Vec<Delta> = self.local_log(family)[push.from as usize..push.to as usize]
                    .iter()
                    .map(|delta| self.redactor.redact_delta(delta))
                    .collect();
                let blobs = referenced_blobs(&batch);
                let (events, remote, store) = (self.events.clone(), remote.clone(), self.store.clone());
                tokio::spawn(async move {
                    for hash in blobs {
                        if let Ok(bytes) = store.blobs().get(hash).await {
                            let _ = remote.put_blob(hash, bytes).await;
                        }
                    }
                    let (generation, outcome, error) = remote.push(family, batch).await;
                    let _ = events.send(Event::Pushed {
                        family,
                        generation,
                        outcome,
                        error: error.map(|error| error.to_string()),
                    });
                });
            }
            let sync = self.syncs.get_mut(&family).expect("inserted above");
            if let Some(pull) = sync.next_pull() {
                let (events, remote) = (self.events.clone(), remote.clone());
                tokio::spawn(async move {
                    let (generation, outcome, error) = remote.pull(family, pull.after, pull.last, PULL_WAIT).await;
                    let _ = events.send(Event::Pulled {
                        family,
                        generation,
                        outcome,
                        error: error.map(|error| error.to_string()),
                    });
                });
            }
        }
    }

    pub(crate) async fn pushed(&mut self, family: FamilyId, generation: Option<u64>, outcome: Outcome, error: Option<String>) {
        self.note_sync_error(error);
        let effects = self.syncs.entry(family).or_default().on_push(generation, outcome);
        self.handle_effects(family, effects).await;
        self.save_sync(family).await;
    }

    pub(crate) async fn pulled(&mut self, family: FamilyId, generation: Option<u64>, outcome: Outcome, error: Option<String>) {
        self.note_sync_error(error);
        let effects = self.syncs.entry(family).or_default().on_pull(generation, outcome);
        self.handle_effects(family, effects).await;
        self.save_sync(family).await;
        self.drive_sync();
    }

    fn note_sync_error(&mut self, error: Option<String>) {
        if let Some(error) = &error
            && self.sync_error.as_ref() != Some(error)
        {
            warn!(%error, "sync failed");
        }
        self.sync_error = error;
    }

    async fn handle_effects(&mut self, family: FamilyId, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Apply(deltas) => {
                    if let Err(error) = self.apply_remote(family, deltas).await {
                        warn!(%error, %family, "could not apply deltas from the server; syncing the family again");
                        self.syncs.insert(family, ClientSync::default());
                    }
                }
                Effect::Diverged(id) => {
                    warn!(%id, %family, "a delta collides with a different one on the server; no longer pushing the family");
                }
            }
        }
    }

    async fn save_sync(&self, family: FamilyId) {
        let Some(sync) = self.syncs.get(&family) else {
            return;
        };
        let state = SyncState {
            generation: sync.generation(),
            pushed: sync.pushed(),
            pulled: sync.pulled(),
            last_pulled: sync.last_pulled(),
            view: sync.view(),
        };
        if let Err(error) = self.store.set_sync_state(family, &state).await {
            warn!(%error, "could not save sync progress");
        }
    }

    /// Applies deltas fetched outside the sync loop, deduplicating against
    /// what is already here.
    pub(crate) async fn apply_pulled(&mut self, family: FamilyId, deltas: Vec<Delta>) -> Result<(), CoreError> {
        self.apply_remote(family, deltas).await
    }

    /// Applies deltas from the server and brings checkouts up to date.
    async fn apply_remote(&mut self, family_id: FamilyId, deltas: Vec<Delta>) -> Result<(), CoreError> {
        let at = self.local_log(family_id).len() as u64;
        let mut fresh = Vec::new();
        let mut heads_before = HashMap::new();
        if self.families.contains_key(&family_id) {
            let repo = self.family_repo(family_id)?;
            let family = self.families.get_mut(&family_id).expect("checked above");
            for checkout in self.checkouts.values().filter(|checkout| checkout.family == family_id) {
                heads_before.insert(checkout.record.thread, family.thread_head(checkout.record.thread));
            }
            for delta in deltas {
                if family.apply(delta.clone(), &mut RepoBases(&repo))? == Applied::New {
                    fresh.push(delta);
                }
            }
        } else {
            let raw = self.raw.entry(family_id).or_default();
            let known: HashSet<DeltaId> = raw.iter().map(|delta| delta.span.first()).collect();
            for delta in deltas {
                if !known.contains(&delta.span.first()) {
                    raw.push(delta.clone());
                    fresh.push(delta);
                }
            }
        }
        if fresh.is_empty() {
            return Ok(());
        }
        self.store.append_deltas(family_id, &fresh).await?;
        self.syncs
            .entry(family_id)
            .or_default()
            .appended_from_server(at, fresh.len() as u64);
        for delta in &fresh {
            if let Op::Conv(ConvOp::Native { line, .. }) = &delta.op {
                for entry in claude::normalize(line) {
                    if let Some(text) = entry.searchable_text().filter(|text| !text.trim().is_empty()) {
                        self.store.index(delta.thread, delta.span.first(), entry.kind(), &text).await?;
                    }
                }
            }
        }
        if self.families.contains_key(&family_id) {
            self.lease_changes(&fresh, &heads_before).await?;
            self.update_checkouts(family_id, &fresh).await?;
        }
        Ok(())
    }

    /// Writes deltas made elsewhere into this family's checkouts.
    pub(crate) async fn update_checkouts(&mut self, family_id: FamilyId, deltas: &[Delta]) -> Result<(), CoreError> {
        let ids: Vec<i64> = self
            .checkouts
            .iter()
            .filter(|(_, checkout)| checkout.family == family_id)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let checkout = &self.checkouts[&id];
            let inodes: Vec<InodeId> = deltas
                .iter()
                .filter(|delta| !checkout.bridge.is_own(delta))
                .filter_map(touched_inode)
                .collect();
            let actions = checkout
                .bridge
                .reconcile(&self.families[&family_id], &mut RepoBases(&checkout.repo), inodes);
            let again = self.perform(id, actions).await?;
            for path in again {
                Box::pin(self.ingest_file(id, &path)).await?;
            }
        }
        Ok(())
    }

    /// A repository to read the family's git blobs from.
    pub(crate) fn family_repo(&self, family: FamilyId) -> Result<silt_git::Repo, CoreError> {
        self.checkouts
            .values()
            .filter(|checkout| checkout.family == family)
            .map(|checkout| checkout.record.path.clone())
            .chain(
                self.threads
                    .values()
                    .filter(|thread| thread.family == family && !thread.repo_root.as_os_str().is_empty())
                    .map(|thread| thread.repo_root.clone()),
            )
            .find_map(|path| silt_git::Repo::discover(&path).ok())
            .ok_or_else(|| CoreError::Invalid(format!("no local clone for family {family}")))
    }

    pub(crate) async fn listed(&mut self, listing: Result<Listing, String>) {
        let listing = match listing {
            Ok(listing) => listing,
            Err(error) => {
                self.note_sync_error(Some(error));
                return;
            }
        };
        for family in &listing.families {
            let sync = self.syncs.entry(family.id).or_default();
            let before = sync.view();
            if sync.observe_view(family.view) {
                tracing::info!(family = %family.id, "what this user may see changed; reading the family again");
            }
            if before != family.view {
                self.save_sync(family.id).await;
            }
        }
        let remote_threads: HashSet<_> = listing.threads.iter().map(|thread| thread.id).collect();
        for info in listing.threads {
            if let Some(known) = self.threads.get(&info.id) {
                if known.owner != info.owner && !info.owner.is_empty() {
                    let updated = ThreadRecord {
                        owner: info.owner.clone(),
                        ..known.clone()
                    };
                    if let Err(error) = self.store.create_thread(&updated).await {
                        warn!(%error, "could not record a thread's owner");
                    }
                    self.threads.insert(info.id, updated);
                }
                continue;
            }
            let record = ThreadRecord {
                id: info.id,
                family: info.family,
                repo_root: Default::default(),
                repo: info.repo,
                created_ms: info.created_ms,
                owner: info.owner,
            };
            if let Err(error) = self.store.create_thread(&record).await {
                warn!(%error, "could not record a thread from the server");
                continue;
            }
            if !self.families.contains_key(&info.family) {
                self.raw.entry(info.family).or_default();
            }
            self.threads.insert(info.id, record);
        }
        let unregistered: Vec<ThreadRecord> = self
            .threads
            .values()
            .filter(|thread| !remote_threads.contains(&thread.id))
            .cloned()
            .collect();
        for thread in unregistered {
            self.register_thread(&thread);
        }
    }

    pub(crate) fn register_thread(&self, thread: &ThreadRecord) {
        let Some(remote) = self.remote.clone() else {
            return;
        };
        let info = thread_info(thread);
        tokio::spawn(async move {
            if let Err(error) = remote.register_thread(&info).await {
                warn!(%error, "could not register a thread with the server");
            }
        });
    }

    /// Registers the thread and waits for the server, for when the next step
    /// needs the server to know it.
    pub(crate) async fn register_thread_now(&self, thread: ThreadId) -> Result<(), CoreError> {
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        remote
            .register_thread(&thread_info(&self.threads[&thread]))
            .await
            .map_err(|error| CoreError::Invalid(format!("could not register the thread with the server: {error}")))
    }

    /// The blob's bytes, fetched from the server when not stored locally.
    pub(crate) async fn blob(&self, hash: BlobHash) -> Result<Vec<u8>, CoreError> {
        if let Ok(bytes) = self.store.blobs().get(hash).await {
            return Ok(bytes);
        }
        let remote = self
            .remote
            .as_ref()
            .ok_or_else(|| CoreError::Invalid(format!("blob {hash} is missing")))?;
        let bytes = remote
            .get_blob(hash)
            .await
            .map_err(|error| CoreError::Invalid(format!("could not fetch blob {hash}: {error}")))?;
        self.store.blobs().put(&bytes).await?;
        Ok(bytes)
    }
}

fn thread_info(thread: &ThreadRecord) -> ThreadInfo {
    ThreadInfo {
        id: thread.id,
        family: thread.family,
        repo: thread.repo.clone(),
        created_ms: thread.created_ms,
        lease: None,
        owner: String::new(),
    }
}

fn referenced_blobs(deltas: &[Delta]) -> Vec<BlobHash> {
    deltas
        .iter()
        .filter_map(|delta| match &delta.op {
            Op::Create {
                content: Content::Blob(hash),
                ..
            }
            | Op::SetContent {
                content: Content::Blob(hash),
                ..
            }
            | Op::Conv(ConvOp::Attachment { blob: hash, .. })
            | Op::Git(GitOp::Bundle { blob: hash, .. }) => Some(*hash),
            _ => None,
        })
        .collect()
}
