//! What the daemon holds of the store, kept to what is in use (ADR 0116).

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime};

use silt_log::{Family, snapshot_files};
use silt_types::{BlobHash, FamilyId, ThreadId};
use tracing::{debug, warn};

use crate::core::Core;

/// How long a family's file or log may go unused before it's let go of.
pub(crate) const UNUSED: Duration = Duration::from_secs(5 * 60);

/// How long a family may go unused before it's let go of: once its files
/// and log are, holding it costs little, and loading it again is what
/// switching back to its thread would wait for (ADR 0117).
const FAMILY_UNUSED: Duration = Duration::from_secs(30 * 60);

/// How much the families held may take, as the store holds them, before
/// the least recently used are let go of sooner (`SILT_FAMILY_BUDGET_MB`).
const BUDGET_MB: u64 = 1024;

/// Deltas recorded since a family's last snapshot that make letting it go
/// take one first: replaying fewer when it's loaded again costs little.
const SNAPSHOT_ON_UNLOAD: usize = 50;

/// How often files' states no snapshot names are looked for.
const SWEEP_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// How old a file's state no snapshot names must be before it's removed: a
/// family loaded from an older snapshot, here or in a hook's process, may
/// still read it.
const SWEEP_GRACE: Duration = Duration::from_secs(60 * 60);

/// When the daemon last let go of what was unused, when it next sweeps, and
/// how much the families it holds may take.
#[derive(Debug)]
pub(crate) struct Kept {
    released: Instant,
    sweep_at: Instant,
    pub(crate) budget: u64,
}

impl Default for Kept {
    /// The first sweep waits out the grace: it reads every snapshot and
    /// lists every file's state, too much for each start, and the states a
    /// start writes are kept that long anyway.
    fn default() -> Self {
        let now = Instant::now();
        Self { released: now, sweep_at: now + SWEEP_GRACE, budget: budget() }
    }
}

impl Kept {
    /// Whether to sweep at `now`, setting when to sweep next if so.
    fn sweep_due(&mut self, now: Instant) -> bool {
        if now < self.sweep_at {
            return false;
        }
        self.sweep_at = now + SWEEP_EVERY;
        true
    }
}

impl Core {
    /// Lets go of the files unused for a while, replays the families whose
    /// stored files can't be read, and sweeps the files' states now and then.
    pub(crate) async fn keep_memory(&mut self) {
        if self.resident {
            self.unload_unused().await;
        }
        let unreadable: Vec<FamilyId> =
            self.families.iter_loaded().filter(|(_, family)| family.files_unreadable()).map(|(id, _)| *id).collect();
        for family in unreadable {
            warn!(%family, "replaying a family whose stored files can't be read");
            if let Err(error) = self.reload_family(family).await {
                warn!(%error, %family, "could not replay the family");
            }
        }
        let outdated: Vec<FamilyId> = self
            .families
            .loaded_ids()
            .into_iter()
            .filter(|family| self.families.snapshotted(family).is_some_and(|(_, outdated)| outdated))
            .collect();
        for family in outdated {
            self.snapshot_if_due(family).await;
        }
        if self.kept.released.elapsed() >= UNUSED {
            self.kept.released = Instant::now();
            let released: usize = self.families.loaded_values_mut().map(Family::release_files).sum();
            if released > 0 {
                debug!(released, "let go of files unused for a while");
            }
            for family in self.families.loaded_ids() {
                if self.families.loaded_mut(&family).is_some_and(Family::log_idle) {
                    self.release_log(family).await;
                }
            }
            silt_os::return_freed_memory();
        }
        if self.resident && self.kept.sweep_due(Instant::now()) {
            self.sweep_file_states().await;
        }
    }

    /// Lets go of a family's log, read again from the store when next read,
    /// if the store holds just the deltas the log does: it keeps those that
    /// no longer applied when the family was loaded, which the log doesn't.
    async fn release_log(&mut self, family: FamilyId) {
        let Ok(stored) = self.store.stored_deltas(family).await else {
            return;
        };
        let Some(held) = self.families.loaded_mut(&family) else {
            return;
        };
        let len = held.log_len();
        if len == 0 || stored.len() != len {
            return;
        }
        let last = held.log().last().map(|delta| delta.span.first());
        let stored_last = stored.decode(len - 1..len).ok().and_then(|deltas| deltas.first().map(|delta| delta.span.first()));
        if last != stored_last {
            return;
        }
        let stored = std::sync::Arc::new(stored);
        held.release_log(Box::new(move || stored.decode(0..len).expect("deltas that were read from the store decode again")));
    }

    /// Lets go of the families unused for a while, and while those held
    /// take more than the budget, of the least recently used.
    async fn unload_unused(&mut self) {
        let held = self.families.loaded_ids();
        for family in &held {
            if self.families.size(family).is_none() {
                let size = self.store.family_size(*family).await.unwrap_or_default();
                self.families.set_size(family, size);
            }
        }
        let mut free: Vec<(FamilyId, Duration)> = held
            .iter()
            .filter(|family| !self.pinned(**family))
            .map(|family| (*family, self.families.unused_for(family)))
            .collect();
        free.sort_by_key(|(_, unused)| std::cmp::Reverse(*unused));
        let size = |family: &FamilyId| self.families.size(family).unwrap_or_default();
        let mut total: u64 = held.iter().map(size).sum();
        let budget = self.kept.budget;
        let mut going = Vec::new();
        for (family, unused) in free {
            if unused >= FAMILY_UNUSED || total > budget {
                total = total.saturating_sub(size(&family));
                going.push(family);
            }
        }
        if going.is_empty() {
            return;
        }
        let positions = match self.store.positions().await {
            Ok(positions) => positions,
            Err(error) => {
                warn!(%error, "not letting go of families: the store's positions couldn't be read");
                return;
            }
        };
        for family in going {
            let position = positions.get(&family).copied().unwrap_or(-1);
            self.unload(family, position).await;
        }
    }

    /// Keeps the digests of the families held whose logs grew, as the daemon
    /// stops, so the next start finds them up to date. Snapshots wait for the
    /// next load, which replays what came since: stopping stays quick.
    pub(crate) async fn keep_digests(&mut self) {
        let positions = self.store.positions().await.unwrap_or_default();
        for family in self.families.loaded_ids() {
            if self.digested(family).is_none()
                && let Err(error) = self.keep_digest(family, positions.get(&family).copied().unwrap_or(-1)).await
            {
                warn!(%error, %family, "could not keep the family's digest");
            }
        }
    }

    /// Lets go of the family after taking a snapshot if enough was recorded
    /// since its last, or it's in an older format, and its digest if what
    /// it says changed.
    pub(crate) async fn unload(&mut self, family: FamilyId, position: i64) {
        if let Some((covered, outdated)) = self.families.snapshotted(&family)
            && let Some(held) = self.families.loaded_mut(&family)
            && (outdated || held.log_len() >= covered + SNAPSHOT_ON_UNLOAD)
        {
            match crate::load::save_snapshot(&self.store, family, held).await {
                Ok(covers) => self.families.set_snapshotted(&family, covers),
                Err(error) => {
                    warn!(%error, %family, "keeping the family: its snapshot couldn't be saved");
                    return;
                }
            }
        }
        if self.digested(family).is_none()
            && let Err(error) = self.keep_digest(family, position).await
        {
            warn!(%error, %family, "keeping the family: its digest couldn't be saved");
            return;
        }
        debug!(%family, "letting go of a family unused for a while");
        self.let_go(family);
    }

    /// Lets go of the family, freeing it on a thread of its own: a long
    /// family takes a while to free.
    pub(crate) fn let_go(&mut self, family: FamilyId) {
        if let Some(held) = self.families.unload(family) {
            std::thread::spawn(move || {
                drop(held);
                silt_os::return_freed_memory();
            });
        }
    }

    /// Whether the family is in use beyond being read: an agent works in it,
    /// or it has deltas its server doesn't have yet.
    pub(crate) fn pinned(&self, family: FamilyId) -> bool {
        let in_family = |thread: &ThreadId| self.threads.get(thread).is_some_and(|record| record.family == family);
        self.importing
            || self
                .sessions
                .iter()
                .any(|session| session.family == family && (session.live || self.draining.contains(&session.record.session_id)))
            || self.pending.iter().any(|pending| pending.family == family)
            || self.acp.keys().any(in_family)
            || self.unpushed(family)
    }

    fn unpushed(&self, family: FamilyId) -> bool {
        self.remote_of(family).is_some()
            && self
                .syncs
                .get(&family)
                .is_some_and(|sync| sync.diverged().is_none() && sync.pushed() < self.local_log_len(family) as u64)
    }

    /// Removes the files' states that no snapshot and no family here names.
    async fn sweep_file_states(&self) {
        let mut named: HashSet<BlobHash> = self.families.iter_loaded().flat_map(|(_, family)| family.stored_files()).collect();
        let families: HashSet<FamilyId> = self.threads.values().map(|record| record.family).collect();
        for family in families {
            match self.store.snapshot(family).await {
                Ok(Some(snapshot)) => {
                    if let Ok(hashes) = snapshot_files(&snapshot.data, snapshot.format) {
                        named.extend(hashes);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    warn!(%error, %family, "not sweeping files' states: a snapshot couldn't be read");
                    return;
                }
            }
        }
        let stored = match self.store.files().list().await {
            Ok(stored) => stored,
            Err(error) => {
                warn!(%error, "could not list files' states");
                return;
            }
        };
        let now = SystemTime::now();
        let mut removed = 0;
        for (hash, written) in stored {
            if !named.contains(&hash)
                && now.duration_since(written).is_ok_and(|age| age > SWEEP_GRACE)
                && self.store.files().remove(hash).await.is_ok()
            {
                removed += 1;
            }
        }
        if removed > 0 {
            debug!(removed, "removed files' states no snapshot names");
        }
    }
}

fn budget() -> u64 {
    let megabytes = std::env::var("SILT_FAMILY_BUDGET_MB").ok().and_then(|mb| mb.parse().ok()).unwrap_or(BUDGET_MB);
    megabytes.saturating_mul(1024 * 1024)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use silt_log::{Author, ConvOp, Delta, Family, GitOp, Op};
    use silt_proto::view::GraphQuery;
    use silt_store::{Store, ThreadRecord};
    use silt_types::ActorId;
    use tokio::sync::{mpsc, oneshot};

    use std::collections::HashMap;

    use super::*;
    use crate::core::Event;
    use crate::protocol::Request;

    #[test]
    fn files_are_swept_once_the_grace_has_passed_since_start_then_now_and_then() {
        let mut kept = Kept::default();
        let start = Instant::now();
        assert!(!kept.sweep_due(start), "not as the daemon starts");
        assert!(!kept.sweep_due(start + SWEEP_GRACE / 2));
        assert!(kept.sweep_due(start + SWEEP_GRACE + Duration::from_secs(1)));
        assert!(!kept.sweep_due(start + SWEEP_GRACE + Duration::from_secs(2)), "not again at once");
        assert!(kept.sweep_due(start + SWEEP_GRACE + SWEEP_EVERY + Duration::from_secs(1)));
    }

    const THREAD: ThreadId = ThreadId(7);
    const FAMILY: FamilyId = FamilyId(9);
    const COMMIT: silt_types::GitOid = silt_types::GitOid([7; 20]);

    /// A store with one titled thread in a clone; the directories go with it.
    async fn store_with_thread() -> (tempfile::TempDir, tempfile::TempDir, Store) {
        let committed = Op::Git(GitOp::Committed { commit: COMMIT, branch: None });
        store_with(vec![
            Op::Conv(ConvOp::Title("held or not".into())),
            committed,
            Op::Conv(ConvOp::Title("still the same".into())),
        ])
        .await
    }

    /// A store with one thread in a clone, whose family's log is `ops`.
    async fn store_with(ops: Vec<Op>) -> (tempfile::TempDir, tempfile::TempDir, Store) {
        let (data, repo) = (tempfile::tempdir().expect("temp dir"), tempfile::tempdir().expect("temp dir"));
        assert!(Command::new("git").args(["init", "-q"]).current_dir(repo.path()).status().expect("git").success());
        let store = Store::open(data.path()).await.expect("opens");
        let thread = ThreadRecord {
            id: THREAD,
            family: FAMILY,
            repo_root: repo.path().to_owned(),
            repo: "example.com/repo".into(),
            created_ms: 1,
            owner: String::new(),
        };
        store.create_thread(&thread).await.expect("thread");
        let mut family = Family::new();
        let mut log: Vec<Delta> = Vec::new();
        for op in ops {
            let delta = family.delta(ActorId(2), THREAD, Author::User, 5, family.thread_head(THREAD), op);
            family.apply(delta.clone(), &mut std::collections::HashMap::new()).expect("applies");
            log.push(delta);
        }
        store.append_deltas(FAMILY, &log).await.expect("appends");
        (data, repo, store)
    }

    /// The daemon's core once loaded and through the digests it takes then.
    async fn loaded_core(store: Store, data: &std::path::Path) -> (Core, mpsc::UnboundedReceiver<Event>) {
        let (events, inbox) = mpsc::unbounded_channel();
        let mut core = Core::load(store, ActorId(1), data, events).await.expect("loads");
        while !core.stale_digests.is_empty() {
            core.take_stale_digest().await;
        }
        (core, inbox)
    }

    /// What the views across families say.
    fn views(core: &Core) -> String {
        let summaries = serde_json::to_string(&core.thread_summaries()).expect("encodes");
        let requests = serde_json::to_string(&core.mr_list(None).expect("lists")).expect("encodes");
        let graph = serde_json::to_string(&core.graph(&GraphQuery::default())).expect("encodes");
        format!("{summaries}\n{requests}\n{graph}\n{}", core.local_log_len(FAMILY))
    }

    #[tokio::test]
    async fn the_graph_narrows_to_a_threads_family_by_its_id() {
        let (data, _repo, store) = store_with_thread().await;
        let (core, _inbox) = loaded_core(store, data.path()).await;
        let all = core.graph(&GraphQuery::default());
        assert!(!all.is_empty(), "the thread's commit is in the graph");
        let of = |thread: String| core.graph(&GraphQuery { thread: Some(thread), ..GraphQuery::default() });
        assert_eq!(of(THREAD.to_string()), all);
        assert_eq!(of(THREAD.short()), all, "by the short id too");
        assert!(of(ThreadId(404).to_string()).is_empty(), "no thread, no rows rather than every family's");
    }

    #[tokio::test]
    async fn a_family_not_held_answers_the_views_across_families_as_one_held_does() {
        let (data, _repo, store) = store_with_thread().await;
        let (core, _inbox) = loaded_core(store.clone(), data.path()).await;
        assert!(!core.families.is_loaded(&FAMILY), "the daemon holds no family once loaded");
        assert!(store.digests().await.expect("reads").contains_key(&FAMILY), "its digest was taken and kept");
        let digested = views(&core);
        assert!(!core.families.is_loaded(&FAMILY), "reading the views across families loads none");
        assert!(core.families.get(&FAMILY).is_some(), "anything else reading it loads it");
        let stored = store.family_size(FAMILY).await.expect("sized");
        assert_eq!(core.families.size(&FAMILY), Some(stored), "with its size, for the tick not to ask");
        assert!(core.families.is_loaded(&FAMILY));
        assert_eq!(digested, views(&core));
        assert!(digested.contains("still the same"));
    }

    #[tokio::test]
    async fn what_a_digest_holds_is_read_without_loading_the_family() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, _inbox) = loaded_core(store, data.path()).await;
        assert_eq!(core.recorded_commit(FAMILY, THREAD), Some(COMMIT));
        core.tick().await;
        core.tick().await;
        assert!(!core.families.is_loaded(&FAMILY), "ticking loads no family");
        assert!(core.families.get(&FAMILY).is_some());
        assert!(core.digested(FAMILY).is_some(), "held for a look, its log unchanged, the views read its digest");
    }

    #[tokio::test]
    async fn what_another_writer_stores_for_a_family_not_held_is_taken_in_with_it() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, _inbox) = loaded_core(store.clone(), data.path()).await;
        let log = store.deltas(FAMILY).await.expect("reads");
        let mut family = Family::new();
        for delta in &log {
            family.apply(delta.clone(), &mut std::collections::HashMap::new()).expect("applies");
        }
        let renamed = Op::Conv(ConvOp::Title("renamed by a hook".into()));
        let by_hook = family.delta(ActorId(3), THREAD, Author::User, 9, family.thread_head(THREAD), renamed);
        store.append_deltas(FAMILY, std::slice::from_ref(&by_hook)).await.expect("appends");
        core.catch_up_with_store().await.expect("catches up");
        assert!(core.families.is_loaded(&FAMILY));
        assert!(core.digested(FAMILY).is_none(), "the digest no longer says what the family holds");
        let title = core
            .thread_summaries()
            .into_iter()
            .find(|summary| summary.id == THREAD.to_string())
            .and_then(|summary| summary.title);
        assert_eq!(title.as_deref(), Some("renamed by a hook"));
    }

    #[tokio::test]
    async fn a_log_unused_is_let_go_of_and_read_back_from_the_store() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, _inbox) = loaded_core(store.clone(), data.path()).await;
        let held = core.families.get_mut(&FAMILY).expect("loads");
        let log = held.log().to_vec();
        assert!(held.log_decoded());
        core.release_log(FAMILY).await;
        let held = core.families.loaded(&FAMILY).expect("still held");
        assert!(!held.log_decoded(), "let go of");
        assert_eq!(held.log(), log);
        assert_eq!(log, store.deltas(FAMILY).await.expect("reads"));
    }

    #[tokio::test]
    async fn an_active_familys_views_read_what_it_added_without_reading_its_log_back() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, _inbox) = loaded_core(store.clone(), data.path()).await;
        let held = core.families.get_mut(&FAMILY).expect("loads");
        held.log();
        core.thread_summaries();
        core.history_page(
            THREAD,
            crate::history::PageAsked { offset: 0, limit: usize::MAX, known: None, cut: true, tail: Some(50), before: None },
            &HashMap::new(),
        );
        core.release_log(FAMILY).await;
        let held = core.families.get_mut(&FAMILY).expect("held");
        let renamed = Op::Conv(ConvOp::Title("renamed while held".into()));
        let delta = held.delta(ActorId(1), THREAD, Author::User, 9, held.thread_head(THREAD), renamed);
        held.apply(delta, &mut HashMap::new()).expect("applies");
        let title = core
            .thread_summaries()
            .into_iter()
            .find(|summary| summary.id == THREAD.to_string())
            .and_then(|summary| summary.title);
        assert_eq!(title.as_deref(), Some("renamed while held"));
        core.mr_list(None).expect("lists");
        core.history_page(
            THREAD,
            crate::history::PageAsked { offset: 0, limit: usize::MAX, known: None, cut: true, tail: Some(50), before: None },
            &HashMap::new(),
        );
        assert!(!core.families.loaded(&FAMILY).expect("held").log_decoded(), "what it added was read alone");
    }

    /// The web UI asks for each image it shows whole on its own, and an
    /// image comes in the same line as its tool's output, after it.
    #[tokio::test]
    async fn an_entry_asked_for_whole_after_another_of_its_line_is_read_from_its_delta_alone() {
        let acp = |line: serde_json::Value| {
            let session = silt_log::SessionRef { agent: silt_agents::acp::AGENT.into(), id: "s".into() };
            Op::Conv(ConvOp::Native { session, file: silt_log::SessionFile::Main, line: line.to_string() })
        };
        let picture = "A".repeat(4000);
        let output = serde_json::json!([
            {"type": "text", "text": "rendered"},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": picture}},
        ]);
        let (data, _repo, store) = store_with(vec![
            acp(serde_json::json!({"t": "prompt", "text": "render it"})),
            acp(serde_json::json!({"t": "tool_call", "id": "c1", "title": "Render", "kind": "execute", "input": {}})),
            acp(serde_json::json!({"t": "tool_result", "id": "c1", "status": "completed", "output": output.to_string()})),
        ])
        .await;
        let (mut core, _inbox) = loaded_core(store, data.path()).await;
        let entries = core.history_entries(THREAD);
        let image = entries.iter().position(|(_, entry)| entry.kind == "image").expect("an image");
        assert_eq!(entries[image - 1].1.kind, "tool_result");
        assert_eq!(entries[image - 1].0, entries[image].0, "both from the one line");
        core.release_log(FAMILY).await;
        let asked = crate::history::PageAsked { offset: image, limit: 1, known: None, cut: false, tail: None, before: None };
        let read = core.deltas_for_page(THREAD, asked).await;
        let page = core.history_page(THREAD, asked, &read);
        let whole = &page.entries[0];
        assert_eq!(whole.kind, "image");
        assert!(whole.cut.is_none());
        assert_eq!(whole.image.as_ref().map(|image| image.data.as_str()), Some(picture.as_str()));
        assert!(
            !core.families.loaded(&FAMILY).expect("held").log_decoded(),
            "read from its line alone, not from the history read again whole"
        );
    }

    #[tokio::test]
    async fn a_page_of_a_history_kept_as_it_is_now_is_answered_without_loading_the_family() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, _inbox) = loaded_core(store, data.path()).await;
        let page = |cut| Request::HistoryRead {
            thread: THREAD.to_string(),
            offset: 0,
            limit: usize::MAX,
            known: None,
            cut,
            tail: Some(50),
            before: None,
        };
        let (reply, _answer) = oneshot::channel();
        assert!(core.unless_loading(Event::Request(page(true), reply)).is_none(), "not kept yet, it waits for the load");
        core.history_entries(THREAD);
        assert!(core.history_kept_now(THREAD));
        core.let_go(FAMILY);
        core.waiting = crate::loading::Waiting::default();
        let (reply, _answer) = oneshot::channel();
        assert!(core.unless_loading(Event::Request(page(true), reply)).is_some(), "kept, it's answered at once");
        assert!(!core.families.is_loaded(&FAMILY));
        let (reply, _answer) = oneshot::channel();
        assert!(core.unless_loading(Event::Request(page(false), reply)).is_none(), "read whole, it waits");
    }

    fn page_asked(cut: bool) -> Request {
        Request::HistoryRead {
            thread: THREAD.to_string(),
            offset: 0,
            limit: usize::MAX,
            known: None,
            cut,
            tail: Some(50),
            before: None,
        }
    }

    #[tokio::test]
    async fn a_history_whose_family_isnt_held_is_read_by_the_loader_with_it() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, mut inbox) = loaded_core(store, data.path()).await;
        while inbox.try_recv().is_ok() {}
        let (reply, _answer) = oneshot::channel();
        assert!(core.unless_loading(Event::Request(page_asked(true), reply)).is_none(), "it waits for the load");
        let Some(Event::Loaded { family, result, size, histories }) = inbox.recv().await else {
            panic!("the loader answers");
        };
        assert_eq!(histories.len(), 1, "with the history asked for");
        core.loaded(family, *result, size, histories);
        assert!(core.history_kept_now(THREAD), "kept without the core reading it");
        let next = core.views.receiver.try_recv();
        assert!(matches!(next, Ok(Event::Request(Request::HistoryRead { .. }, _))), "the page goes on, ahead of other events");
    }

    #[tokio::test]
    async fn a_history_not_kept_in_a_family_held_is_read_off_the_core() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, mut inbox) = loaded_core(store, data.path()).await;
        core.families.get_mut(&FAMILY).expect("loads").log();
        while inbox.try_recv().is_ok() {}
        assert!(core.history_unkept(THREAD));
        let (reply, _answer) = oneshot::channel();
        assert!(core.unless_loading(Event::Request(page_asked(true), reply)).is_none(), "it waits for the read");
        let (reply, _answer) = oneshot::channel();
        let turns = Event::Request(Request::Turns { thread: THREAD.to_string() }, reply);
        assert!(core.unless_loading(turns).is_none(), "and so do its turns, read with it");
        let Some(Event::HistoryRead { thread, whole, copy }) = inbox.recv().await else {
            panic!("the history is read off the core");
        };
        assert!(whole.is_some() && !copy, "on a blocking thread, from its log as held");
        core.history_read(thread, whole.map(|whole| *whole), copy);
        assert!(core.history_kept_now(THREAD));
        let sent = std::iter::from_fn(|| core.views.receiver.try_recv().ok()).count();
        assert_eq!(sent, 2, "both go on");
        let read_off = core.history_entries(THREAD);
        core.history_cache.lock().expect("lock").clear();
        assert_eq!(*core.history_entries(THREAD), *read_off, "as the core reads it");
    }

    #[tokio::test]
    async fn a_history_whose_familys_log_was_let_go_of_is_read_by_the_loader_in_a_copy() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, mut inbox) = loaded_core(store, data.path()).await;
        core.families.get_mut(&FAMILY).expect("loads").log();
        core.release_log(FAMILY).await;
        while inbox.try_recv().is_ok() {}
        let (reply, _answer) = oneshot::channel();
        assert!(core.unless_loading(Event::Request(page_asked(true), reply)).is_none(), "it waits for the read");
        let Some(Event::HistoryRead { thread, whole, copy }) = inbox.recv().await else {
            panic!("the history is read off the core");
        };
        assert!(copy && whole.is_some(), "by the loader");
        core.history_read(thread, whole.map(|whole| *whole), copy);
        assert!(core.history_kept_now(THREAD));
        assert!(!core.families.loaded(&FAMILY).expect("held").log_decoded(), "the log stays let go of");
        let next = core.views.receiver.try_recv();
        assert!(matches!(next, Ok(Event::Request(Request::HistoryRead { .. }, _))), "the page goes on, ahead of other events");
    }

    #[tokio::test]
    async fn a_pages_view_is_answered_while_long_work_is_waited_for() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, _inbox) = loaded_core(store, data.path()).await;
        let (reply, answer) = oneshot::channel();
        core.views.sender.send(Event::Request(Request::Threads, reply)).expect("sends");
        // Work that ends only once the view is answered.
        let work = tokio::spawn(async move { answer.await.is_ok() });
        assert!(core.answering_views(work).await.expect("joins"), "the view was answered meanwhile");
    }

    #[tokio::test]
    async fn families_are_let_go_of_over_budget_unless_in_use() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, _inbox) = loaded_core(store.clone(), data.path()).await;
        core.kept.budget = 0;
        core.families.get(&FAMILY);
        core.importing = true;
        core.unload_unused().await;
        assert!(core.families.is_loaded(&FAMILY), "an import holds every family");
        core.importing = false;
        core.unload_unused().await;
        assert!(!core.families.is_loaded(&FAMILY), "over budget, a family not in use is let go of");
        let position = store.positions().await.expect("reads")[&FAMILY];
        assert_eq!(store.digests().await.expect("reads")[&FAMILY].position, position);
    }

    #[tokio::test]
    async fn a_request_naming_a_family_not_held_waits_for_it_to_load() {
        let (data, _repo, store) = store_with_thread().await;
        let (mut core, mut inbox) = loaded_core(store, data.path()).await;
        while inbox.try_recv().is_ok() {}
        let (reply, _answer) = oneshot::channel();
        let show = Event::Request(Request::Show { thread: THREAD.to_string() }, reply);
        assert!(core.unless_loading(show).is_none(), "it waits");
        let (reply, _answer) = oneshot::channel();
        let again = Event::Request(Request::Show { thread: THREAD.to_string()[..12].to_owned() }, reply);
        assert!(core.unless_loading(again).is_none(), "and so does what comes after for the family");
        assert!(!core.families.is_loaded(&FAMILY));
        let Some(Event::Loaded { family, result, size, histories }) = inbox.recv().await else {
            panic!("the loader answers");
        };
        core.loaded(family, *result, size, histories);
        assert!(core.families.is_loaded(&FAMILY));
        let stored = core.store.family_size(FAMILY).await.expect("sized");
        assert_eq!(core.families.size(&FAMILY), Some(stored), "its size came with it, for the tick not to ask");
        let shown: Vec<String> = std::iter::from_fn(|| match core.views.receiver.try_recv() {
            Ok(Event::Request(Request::Show { thread }, _)) => Some(thread),
            _ => None,
        })
        .collect();
        assert_eq!(shown, [THREAD.to_string(), THREAD.to_string()[..12].to_owned()], "in the order they came");
        let (reply, _answer) = oneshot::channel();
        assert!(core.unless_loading(Event::Request(Request::Show { thread: THREAD.to_string() }, reply)).is_some());
    }
}
