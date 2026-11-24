use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use silt_crdt::InodeId;
use silt_log::{Applied, ConvOp, Delta, LogError, Op};
use silt_proto::{Listing, ThreadInfo};
use silt_store::{CheckoutId, SearchRow, Store, SyncState, ThreadRecord};
use silt_sync::{ClientSync, Effect, Outcome};
use silt_types::{BlobHash, DeltaId, FamilyId, ThreadId};
use tracing::warn;

use crate::core::{Core, CoreError, Event, RepoBases, touched_inode};
use crate::families::Families;
use crate::homes::Home;
use crate::remote::{Remote, RemoteError};

const LISTING_INTERVAL: Duration = Duration::from_secs(3);
const PULL_WAIT: Duration = Duration::from_secs(20);
/// How long a family first waits before pulling a delta again that
/// couldn't apply yet, or whose taking in failed: often a moment's trouble,
/// such as a busy store. Each time again in a row doubles it, up to
/// `PULL_RETRY_MAX`.
const PULL_RETRY: Duration = Duration::from_secs(1);
const PULL_RETRY_MAX: Duration = Duration::from_secs(30);

/// The family's deltas from `from` on: what came since the log was read
/// whole needn't read it again (ADR 0117).
pub(crate) fn log_since_in(families: &Families, family: FamilyId, from: usize) -> &[Delta] {
    if let Some(held) = families.get(&family) {
        return held.log_since(from).unwrap_or_else(|| held.log().get(from..).unwrap_or_default());
    }
    families.raw(&family).and_then(|deltas| deltas.get(from..)).unwrap_or_default()
}

fn log_of(families: &Families, family: FamilyId) -> &[Delta] {
    match families.get(&family) {
        Some(family) => family.log(),
        None => families.raw(&family).map_or(&[], Vec::as_slice),
    }
}

impl Core {
    /// Every delta of the family in local order, projected or not.
    pub(crate) fn local_log(&self, family: FamilyId) -> &[Delta] {
        log_of(&self.families, family)
    }

    /// How long [`Self::local_log`] is, without reading what a snapshot covers.
    pub(crate) fn local_log_len(&self, family: FamilyId) -> usize {
        if let Some(held) = self.families.loaded(&family) {
            return held.log_len();
        }
        if let Some(digest) = self.digested(family) {
            return digest.log_len;
        }
        match self.families.get(&family) {
            Some(family) => family.log_len(),
            None => self.families.raw(&family).map_or(0, Vec::len),
        }
    }

    pub(crate) fn family_ids(&self) -> Vec<FamilyId> {
        self.families.ids()
    }

    /// Starts any push, pull or listing that is due.
    pub(crate) fn drive_sync(&mut self) {
        if self.remotes.is_empty() {
            return;
        }
        self.drive_thread_states();
        for remote in self.remotes.clone() {
            let server = remote.config.url.clone();
            if self.last_listings.get(&server).is_none_or(|last| last.elapsed() >= LISTING_INTERVAL) {
                let requested = Instant::now();
                self.last_listings.insert(server.clone(), requested);
                let events = self.events.clone();
                let bound = self.binds_an_org(&server);
                tokio::spawn(async move {
                    let listing = remote.listing().await.map_err(|error| error.to_string());
                    let _ = events.send(Event::Listed { server: server.clone(), listing, requested });
                    // The policies of orgs a workspace is bound to, with each listing (ADR 0065).
                    if bound && let Ok(orgs) = remote.orgs().await {
                        let policies = orgs.orgs.into_iter().map(|org| (org.name, org.policy)).collect();
                        let _ = events.send(Event::OrgPolicies { server, policies });
                    }
                });
            }
        }
        for family in self.family_ids() {
            let Some(remote) = self.remote_of(family).cloned() else {
                continue;
            };
            if let Some(batch) = self.next_push(family) {
                let (events, remote, store, uploaded) =
                    (self.events.clone(), remote.clone(), self.store.clone(), self.uploaded.clone());
                tokio::spawn(async move {
                    let _ = events.send(send_push(&remote, &store, &uploaded, batch).await.into());
                });
            }
            if self.pull_paused.get(&family).is_some_and(|(until, _)| Instant::now() < *until) {
                continue;
            }
            let sync = self.syncs.get_mut(&family).expect("inserted above");
            if let Some(pull) = sync.next_pull() {
                let (events, remote, epoch) = (self.events.clone(), remote.clone(), self.sync_epoch(family));
                tokio::spawn(async move {
                    let (generation, outcome, error) = remote.pull(family, pull.after, pull.last, PULL_WAIT).await;
                    let _ = events.send(Event::Pulled {
                        family,
                        epoch,
                        generation,
                        outcome,
                        error: error.map(|error| error.to_string()),
                    });
                });
            }
        }
    }

    /// The family's next deltas to push, redacted, unless a push is in
    /// flight or there is nothing to push.
    pub(crate) fn next_push(&mut self, family: FamilyId) -> Option<PushBatch> {
        // Read without loading a family not held, which has nothing to push
        // unless it was let go of before a server was set up for it.
        let len = self.local_log_len(family) as u64;
        if self.syncs.get(&family).is_some_and(|sync| sync.pushed() >= len) {
            return None;
        }
        let pushed = self.syncs.get(&family).map_or(0, ClientSync::pushed);
        let pending = log_since_in(&self.families, family, usize::try_from(pushed).unwrap_or(usize::MAX));
        let at = |position: u64| usize::try_from(position - pushed).unwrap_or(usize::MAX);
        let push =
            self.syncs.entry(family).or_default().next_push(len, |position| silt_log::stored_len(&pending[at(position)]))?;
        let mut found = Vec::new();
        let deltas: Vec<Delta> = pending[at(push.from)..at(push.to)]
            .iter()
            .map(|delta| {
                let redacted = self.redactor.redact_delta(delta);
                found.extend(redacted.found);
                redacted.delta
            })
            .collect();
        self.learn_secrets(found);
        Some(PushBatch { family, epoch: self.sync_epoch(family), blobs: referenced_blobs(&deltas), deltas })
    }

    /// Starts the family's sync over from `sync`. Answers to requests sent
    /// under the old state are ignored when they come.
    pub(crate) fn replace_sync(&mut self, family: FamilyId, sync: ClientSync) {
        *self.sync_epochs.entry(family).or_default() += 1;
        self.syncs.insert(family, sync);
    }

    fn sync_epoch(&self, family: FamilyId) -> u64 {
        self.sync_epochs.get(&family).copied().unwrap_or_default()
    }

    pub(crate) async fn pushed(
        &mut self,
        family: FamilyId,
        epoch: u64,
        generation: Option<u64>,
        outcome: Outcome,
        error: Option<String>,
    ) {
        if epoch != self.sync_epoch(family) {
            return;
        }
        self.note_family_error(family, error);
        let effects = self.syncs.entry(family).or_default().on_push(generation, outcome);
        self.handle_effects(family, effects).await;
        self.save_sync(family).await;
    }

    pub(crate) async fn pulled(
        &mut self,
        family: FamilyId,
        epoch: u64,
        generation: Option<u64>,
        outcome: Outcome,
        error: Option<String>,
    ) {
        if epoch != self.sync_epoch(family) {
            return;
        }
        self.note_family_error(family, error);
        let effects = self.syncs.entry(family).or_default().on_pull(generation, outcome);
        self.handle_effects(family, effects).await;
        self.save_sync(family).await;
        self.drive_sync();
    }

    fn note_family_error(&mut self, family: FamilyId, error: Option<String>) {
        if let Some(server) = self.remote_of(family).map(|remote| remote.config.url.clone()) {
            self.note_sync_error(&server, error);
        }
    }

    fn note_sync_error(&mut self, server: &str, error: Option<String>) {
        match error {
            Some(error) => {
                if self.sync_errors.get(server) != Some(&error) {
                    warn!(%error, server, "sync failed");
                }
                self.sync_errors.insert(server.to_owned(), error);
            }
            None => {
                self.sync_errors.remove(server);
            }
        }
    }

    async fn handle_effects(&mut self, family: FamilyId, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Apply { after, deltas } => {
                    let positions: Vec<(u64, DeltaId)> = deltas.iter().map(|(seq, delta)| (*seq, delta.span.first())).collect();
                    let stopped = match self.apply_remote(family, deltas.into_iter().map(|(_, delta)| delta).collect()).await {
                        Ok(stopped) => stopped,
                        Err(error) => {
                            warn!(%error, %family, "could not take in deltas from the server; pulling them again shortly");
                            Some(0)
                        }
                    };
                    // Coming back to the delta that couldn't apply, after a
                    // pause rather than in a loop.
                    if let Some(index) = stopped {
                        let (seq, last) =
                            index.checked_sub(1).map_or(after, |previous| (positions[previous].0, Some(positions[previous].1)));
                        self.syncs.entry(family).or_default().rewind_pull(seq, last);
                        let wait = self.pull_paused.get(&family).map_or(PULL_RETRY, |(_, wait)| (*wait * 2).min(PULL_RETRY_MAX));
                        self.pull_paused.insert(family, (Instant::now() + wait, wait));
                    } else {
                        self.pull_paused.remove(&family);
                    }
                }
                Effect::Diverged(id) => {
                    warn!(%id, %family, "a delta collides with a different one on the server; recovering");
                    if let Err(error) = self.recover_diverged(family).await {
                        warn!(%error, %family, "could not recover; the family no longer syncs from here");
                    }
                }
            }
        }
    }

    pub(crate) async fn save_sync(&self, family: FamilyId) {
        let Some(sync) = self.syncs.get(&family) else {
            return;
        };
        // A family listed before any of it is stored here has nowhere to keep
        // its progress yet; it is saved with the first deltas pulled.
        if !self.families.knows(&family) {
            return;
        }
        let state = SyncState {
            generation: sync.generation(),
            pushed: sync.pushed(),
            pulled: sync.pulled(),
            last_pulled: sync.last_pulled(),
            view: sync.view(),
        };
        if let Err(error) = self.store.set_sync_state(family, &state).await {
            warn!(?error, "could not save sync progress");
        }
    }

    /// Applies deltas fetched outside the sync loop, deduplicating against
    /// what is already here.
    /// Applies deltas pulled outside the background sync, returning where
    /// they stopped as [`Self::apply_remote`] does.
    /// What search finds of `deltas`: the entries of their agents' lines.
    pub(crate) fn search_rows(&mut self, deltas: &[Delta]) -> Vec<SearchRow> {
        let mut rows = Vec::new();
        for delta in deltas {
            if let Op::Conv(ConvOp::Native { .. }) = &delta.op {
                for entry in crate::transcripts::all_entries(&mut self.index_transcripts, delta) {
                    if let Some(body) = entry.searchable_text().filter(|text| !text.trim().is_empty()) {
                        rows.push(SearchRow {
                            thread: delta.thread,
                            delta: delta.span.first(),
                            kind: entry.kind().to_owned(),
                            body,
                        });
                    }
                }
            }
        }
        rows
    }

    pub(crate) async fn apply_pulled(&mut self, family: FamilyId, deltas: Vec<Delta>) -> Result<Option<usize>, CoreError> {
        self.apply_remote(family, deltas).await
    }

    /// Applies deltas from the server and brings checkouts up to date.
    /// Deltas come from other people (ADR 0016): one that can't apply here is
    /// skipped, and so are its descendants, rather than stopping the family's
    /// sync. A delta that needs a git blob this clone lacks stops the batch;
    /// its index is returned so the pull can come back to it.
    async fn apply_remote(&mut self, family_id: FamilyId, deltas: Vec<Delta>) -> Result<Option<usize>, CoreError> {
        let at = self.local_log_len(family_id) as u64;
        let mut fresh = Vec::new();
        let mut stopped = None;
        let mut heads_before = HashMap::new();
        if self.families.contains_key(&family_id) {
            let repo = self.family_repo(family_id)?;
            let family = self.families.get_mut(&family_id).expect("checked above");
            for checkout in self.checkouts.values().filter(|checkout| checkout.family == family_id) {
                heads_before.insert(checkout.record.thread, family.thread_head(checkout.record.thread));
            }
            for (index, delta) in deltas.into_iter().enumerate() {
                match family.apply(delta.clone(), &mut RepoBases(&repo)) {
                    Ok(Applied::New) => fresh.push(delta),
                    Ok(Applied::AlreadyKnown) => {}
                    Err(LogError::MissingBase(oid)) => {
                        warn!(%oid, family = %family_id, "a delta from the server needs a git blob this clone doesn't have yet");
                        stopped = Some(index);
                        break;
                    }
                    Err(error) => {
                        let op: String = format!("{:?}", delta.op).chars().take(60).collect();
                        warn!(%error, id = %delta.span.first(), thread = %delta.thread, op, "skipping a delta from the server that can't apply here");
                    }
                }
            }
        } else {
            let raw = self.families.raw_entry(family_id);
            let known: HashSet<DeltaId> = raw.iter().map(|delta| delta.span.first()).collect();
            for delta in deltas {
                if let Err(error) = delta.check_shape() {
                    warn!(%error, id = %delta.span.first(), "skipping a malformed delta from the server");
                    continue;
                }
                if !known.contains(&delta.span.first()) {
                    raw.push(delta.clone());
                    fresh.push(delta);
                }
            }
        }
        if fresh.is_empty() {
            return Ok(stopped);
        }
        // Their search rows take as long again to write, and come after.
        self.persist_unindexed(family_id, &fresh).await?;
        // Others' work is what the server notifies people of.
        self.announce(crate::protocol::BusEvent::Inbox);
        self.syncs.entry(family_id).or_default().appended_from_server(at, fresh.len() as u64);
        if self.families.contains_key(&family_id) {
            self.lease_changes(&fresh, &heads_before).await?;
            self.update_checkouts(family_id, &fresh).await?;
            self.announce_requests(family_id, &fresh).await;
        }
        Ok(stopped)
    }

    /// Tells the agent of a thread here about merge requests opened on it
    /// elsewhere, and about changes asked of its own, on its next prompt.
    async fn announce_requests(&self, family_id: FamilyId, deltas: &[Delta]) {
        let runs_here = |thread: ThreadId| self.sessions.iter().any(|session| session.record.thread == thread);
        for delta in deltas {
            match &delta.op {
                Op::Mr(silt_log::MrOp::Opened { target, title, .. }) if runs_here(*target) => {
                    self.notify(
                        *target,
                        format!(
                            "A merge request \"{title}\" was opened on this thread from fork {}. The user can review it with `silt mr show` and `silt mr approve` or `silt mr decline`.",
                            delta.thread
                        ),
                    )
                    .await;
                }
                Op::Mr(silt_log::MrOp::ChangesRequested { mr, reason }) => {
                    let request = self.families.get(&family_id).and_then(|family| {
                        family.merge_requests().into_iter().find(|request| request.id == *mr && runs_here(request.fork))
                    });
                    if let Some(request) = request {
                        self.notify(request.fork, crate::forks::changes_requested_notice(&request.title, reason)).await;
                    }
                }
                _ => {}
            }
        }
    }

    /// Writes deltas made elsewhere into this family's checkouts.
    pub(crate) async fn update_checkouts(&mut self, family_id: FamilyId, deltas: &[Delta]) -> Result<(), CoreError> {
        let ids: Vec<CheckoutId> =
            self.checkouts.iter().filter(|(_, checkout)| checkout.family == family_id).map(|(id, _)| *id).collect();
        for id in ids {
            let checkout = &self.checkouts[&id];
            // A merge into this checkout's thread brings in the other
            // thread's changes without touching any file itself.
            let merged = deltas.iter().any(|delta| {
                delta.thread == checkout.record.thread && matches!(delta.op, Op::Merge { .. }) && !checkout.bridge.is_own(delta)
            });
            let inodes: Vec<InodeId> = if merged {
                self.families[&family_id].tree().inodes()
            } else {
                deltas.iter().filter(|delta| !checkout.bridge.is_own(delta)).filter_map(touched_inode).collect()
            };
            let actions = checkout.bridge.reconcile(&self.families[&family_id], &mut RepoBases(&checkout.repo), inodes);
            let again = self.perform(id, actions).await?;
            self.ingest_changed_meanwhile(id, again).await?;
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

    /// Every repository here holding the family's git objects: the person's
    /// clones first, as only they follow the remote's branches, then the
    /// thread checkouts made from them.
    pub(crate) fn family_repos(&self, family: FamilyId) -> Vec<silt_git::Repo> {
        let threads = self.threads.values().filter(|thread| thread.family == family);
        let clones = threads.clone().filter_map(|thread| self.clones.get(&thread.repo).cloned());
        let roots = threads.filter(|thread| !thread.repo_root.as_os_str().is_empty()).map(|thread| thread.repo_root.clone());
        let checkouts =
            self.checkouts.values().filter(|checkout| checkout.family == family).map(|checkout| checkout.record.path.clone());
        let mut seen = std::collections::HashSet::new();
        clones
            .chain(roots)
            .chain(checkouts)
            .filter_map(|path| silt_git::Repo::discover(&path).ok())
            .filter(|repo| seen.insert(repo.root().to_owned()))
            .collect()
    }

    /// Takes in the listing of the server at `server`, which speaks only for
    /// the families whose home it is (ADR 0032).
    pub(crate) async fn listed(&mut self, server: &str, listing: Result<Listing, String>, requested: Instant) {
        let listing = match listing {
            Ok(listing) => listing,
            Err(error) => {
                self.note_sync_error(server, Some(error));
                return;
            }
        };
        let home = Home::Server(server.to_owned());
        for family in listing.families.iter().map(|family| family.id).chain(listing.threads.iter().map(|info| info.family)) {
            if let Err(error) = self.settle_home(family, home.clone()).await {
                warn!(%error, "could not record where a family syncs");
            }
        }
        let deleted: Vec<ThreadId> = listing.deleted.iter().copied().filter(|thread| self.homed_at(*thread, server)).collect();
        let purged: Vec<ThreadId> = listing.purged.iter().copied().filter(|thread| self.homed_at(*thread, server)).collect();
        let own: Vec<_> = listing
            .families
            .iter()
            .filter(|family| self.remote_of(family.id).is_some_and(|remote| remote.config.url == server))
            .collect();
        for family in own {
            let sync = self.syncs.entry(family.id).or_default();
            let before = sync.view();
            if sync.observe_view(family.view) {
                tracing::info!(family = %family.id, "what this user may see changed; reading the family again");
            }
            if before != family.view {
                self.save_sync(family.id).await;
            }
        }
        let before = self.heard().then(|| self.listed_state());
        let remote_threads: HashSet<_> = listing.threads.iter().map(|thread| thread.id).collect();
        self.follow_trash(&deleted, &purged, &remote_threads, requested).await;
        let mut visibility: HashMap<ThreadId, silt_proto::Visibility> = self
            .visibility
            .iter()
            .filter(|(thread, _)| !self.homed_at(**thread, server))
            .map(|(thread, seen)| (*thread, seen.clone()))
            .collect();
        visibility.extend(listing.visibility);
        if visibility != self.visibility {
            self.visibility = visibility;
            self.announce(crate::protocol::BusEvent::Threads);
        }
        self.take_running_elsewhere(server, listing.running);
        self.take_machines(listing.devices, listing.made_on);
        for info in listing.threads {
            self.collaborators.insert(info.id, info.collaborators.clone());
            if let Some(known) = self.threads.get(&info.id) {
                // A chat another machine made a project of (ADR 0059).
                if silt_types::is_chat(&known.repo) && !silt_types::is_chat(&info.repo) && !info.repo.is_empty() {
                    let moved = vec![info.id];
                    let root = known.repo_root.clone();
                    if let Err(error) = self.store.move_threads(&moved, silt_types::CHATS, &info.repo, &root).await {
                        warn!(%error, "could not move a chat into its project");
                    } else {
                        self.threads.get_mut(&info.id).expect("known").repo = info.repo.clone();
                        self.announce(crate::protocol::BusEvent::Threads);
                    }
                }
                let known = &self.threads[&info.id];
                if known.owner != info.owner && !info.owner.is_empty() {
                    let updated = ThreadRecord { owner: info.owner.clone(), ..known.clone() };
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
            if !self.families.knows(&info.family) {
                self.families.raw_entry(info.family);
            }
            self.threads.insert(info.id, record);
        }
        if before.is_some_and(|before| self.listed_state() != before) {
            self.announce(crate::protocol::BusEvent::Threads);
        }
        let unregistered: Vec<ThreadRecord> = self
            .threads
            .values()
            .filter(|thread| {
                self.homed_at(thread.id, server) && !remote_threads.contains(&thread.id) && !self.deleted.contains(&thread.id)
            })
            .cloned()
            .collect();
        for thread in unregistered {
            self.register_thread(&thread);
        }
    }

    /// What the server's listing tells this daemon, to see whether one
    /// changed it.
    fn listed_state(&self) -> Vec<(ThreadId, String, Vec<String>, bool)> {
        let mut state: Vec<_> = self
            .threads
            .values()
            .map(|thread| {
                let collaborators = self.collaborators.get(&thread.id).cloned().unwrap_or_default();
                (thread.id, thread.owner.clone(), collaborators, self.deleted.contains(&thread.id))
            })
            .collect();
        state.sort();
        state
    }

    pub(crate) fn register_thread(&self, thread: &ThreadRecord) {
        let Some(remote) = self.remote_of(thread.family).cloned() else {
            return;
        };
        let info = thread_info(thread);
        let origin = origin_url(thread);
        tokio::spawn(async move {
            if let Err(error) = remote.register_thread(&info, None, origin).await {
                warn!(%error, "could not register a thread with the server");
            }
        });
    }

    /// Registers the thread and waits for the server, for when the next step
    /// needs the server to know it.
    pub(crate) async fn register_thread_now(&self, thread: ThreadId) -> Result<(), CoreError> {
        self.register_now(thread, None).await
    }

    /// [`Self::register_thread_now`] for a thread that recovers `of`'s work
    /// here, so its person hears of it (ADR 0028).
    pub(crate) async fn register_recovery(&self, thread: ThreadId, of: ThreadId) -> Result<(), CoreError> {
        self.register_now(thread, Some(of)).await
    }

    async fn register_now(&self, thread: ThreadId, recovery_of: Option<ThreadId>) -> Result<(), CoreError> {
        let Some(remote) = self.remote_for(thread) else {
            return Ok(());
        };
        let record = &self.threads[&thread];
        let body = silt_proto::RegisterThread {
            thread: thread_info(record),
            visibility: None,
            origin_url: origin_url(record),
            recovery_of,
        };
        remote.register(body).await.map_err(|source| CoreError::Server { doing: "register the thread", source })
    }

    /// A blob of the family's, fetched from its home server when not stored
    /// locally; no other server hears of it.
    pub(crate) async fn blob(&self, family: FamilyId, hash: BlobHash) -> Result<Vec<u8>, CoreError> {
        if let Ok(bytes) = self.store.blobs().get(hash).await {
            return Ok(bytes);
        }
        let remote = self.remote_of(family).ok_or_else(|| CoreError::Invalid(format!("blob {hash} is missing")))?;
        let bytes = remote.get_blob(hash).await.map_err(|source| CoreError::Server { doing: "fetch a file", source })?;
        self.store.blobs().put(&bytes).await?;
        Ok(bytes)
    }
}

pub(crate) fn thread_info(thread: &ThreadRecord) -> ThreadInfo {
    ThreadInfo {
        id: thread.id,
        family: thread.family,
        repo: thread.repo.clone(),
        created_ms: thread.created_ms,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    }
}

/// Where the thread's clone fetches from, without credentials (ADR 0023).
pub(crate) fn origin_url(thread: &ThreadRecord) -> Option<String> {
    if thread.repo_root.as_os_str().is_empty() {
        return None;
    }
    let url = silt_git::Repo::discover(&thread.repo_root).ok()?.remote_url("origin").ok()??;
    Some(silt_proto::shareable_url(&url))
}

pub(crate) struct PushBatch {
    family: FamilyId,
    epoch: u64,
    deltas: Vec<Delta>,
    blobs: Vec<BlobHash>,
}

impl PushBatch {
    pub(crate) fn len(&self) -> usize {
        self.deltas.len()
    }
}

/// How the server answered a push, for `Core::pushed`.
pub(crate) struct PushAnswer {
    pub(crate) family: FamilyId,
    pub(crate) epoch: u64,
    pub(crate) generation: Option<u64>,
    pub(crate) outcome: Outcome,
    pub(crate) error: Option<String>,
}

impl From<PushAnswer> for Event {
    fn from(answer: PushAnswer) -> Self {
        let PushAnswer { family, epoch, generation, outcome, error } = answer;
        Event::Pushed { family, epoch, generation, outcome, error }
    }
}

pub(crate) async fn send_push(
    remote: &Remote,
    store: &Store,
    uploaded: &Mutex<HashSet<BlobHash>>,
    batch: PushBatch,
) -> PushAnswer {
    let PushBatch { family, epoch, deltas, blobs } = batch;
    // Deltas go only after their blobs, or others would get deltas whose
    // content they can't fetch.
    if let Err(error) = upload_blobs(remote, store, uploaded, blobs).await {
        return PushAnswer {
            family,
            epoch,
            generation: None,
            outcome: Outcome::Failed,
            error: Some(format!("could not upload a file to the server: {error}")),
        };
    }
    let (generation, outcome, error) = remote.push(family, deltas).await;
    PushAnswer { family, epoch, generation, outcome, error: error.map(|error| error.to_string()) }
}

fn referenced_blobs(deltas: &[Delta]) -> Vec<BlobHash> {
    deltas.iter().filter_map(Delta::blob).collect()
}

/// Sends the server the blobs it hasn't had from here yet. A blob missing
/// here came from elsewhere, so the server has it already.
async fn upload_blobs(
    remote: &Remote,
    store: &Store,
    uploaded: &Mutex<HashSet<BlobHash>>,
    blobs: Vec<BlobHash>,
) -> Result<(), RemoteError> {
    for hash in blobs {
        if uploaded.lock().expect("lock").contains(&hash) {
            continue;
        }
        let Ok(bytes) = store.blobs().get(hash).await else {
            continue;
        };
        remote.put_blob(hash, bytes).await?;
        uploaded.lock().expect("lock").insert(hash);
    }
    Ok(())
}
