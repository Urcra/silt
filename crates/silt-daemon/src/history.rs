//! Reading a thread's full history the way agents and people need it
//! (ADR 0011): numbered entries across compactions and forks, search within
//! that history, which turns changed a file, and who else is changing it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, PoisonError};

use silt_agents::history::HistoryReader;
use silt_agents::tool::Shown;
use silt_agents::{Entry, Transcripts};
use silt_crdt::InodeId;
use silt_log::{Author, ConvOp, Delta, Family, GraphEvent, HistorySource, Op, TurnMark};
use silt_proto::view::{GraphQuery, GraphRow, opens_turn};
use silt_types::{DeltaId, FamilyId, ThreadId};

use crate::core::{CanonicalCwd, Core, CoreError, touched_inode};
use crate::protocol::{CommitView, FileTouch, HistoryEntry, HistoryHit, HistoryPage, InFlight, TurnInfo};
use crate::transcripts::{native_entries, shown_entries};

/// Threads whose history entries are kept at once.
const HISTORIES_KEPT: usize = 16;

/// A thread's numbered history entries, each with the delta it came from,
/// cut as a page carries them (ADR 0117).
pub(crate) type HistoryEntries = Arc<Vec<(DeltaId, HistoryEntry)>>;

/// Readings of a thread's history kept where it changed between.
const CHANGES_KEPT: usize = 64;

/// A thread's history as last read, at `version` (its family's log length).
#[derive(Clone)]
pub(crate) struct KeptHistory {
    pub(crate) version: usize,
    pub(crate) entries: HistoryEntries,
    turns: Arc<Vec<TurnInfo>>,
    /// The version read before the oldest change kept.
    oldest: usize,
    /// Each later version read, and the first entry that differed from the
    /// reading before it.
    changes: std::collections::VecDeque<(usize, usize)>,
}

impl KeptHistory {
    fn new(version: usize, read: Read) -> Self {
        Self {
            version,
            entries: Arc::new(read.entries),
            turns: Arc::new(read.turns),
            oldest: version,
            changes: std::collections::VecDeque::new(),
        }
    }

    /// Read again whole at `version`.
    fn grown(mut self, version: usize, read: Read) -> Self {
        // A log only grows, but for a family erased and recorded again.
        if version < self.version {
            return Self::new(version, read);
        }
        let first = first_difference(&self.entries, &read.entries);
        self.note_change(version, first);
        Self { version, entries: Arc::new(read.entries), turns: Arc::new(read.turns), ..self }
    }

    fn note_change(&mut self, version: usize, first: usize) {
        self.changes.push_back((version, first));
        while self.changes.len() > CHANGES_KEPT {
            if let Some((dropped, _)) = self.changes.pop_front() {
                self.oldest = dropped;
            }
        }
    }

    /// How many of the entries at version `known` are still the same: all
    /// of them for an unknown version.
    fn same_before(&self, known: usize) -> usize {
        if known == self.version {
            return self.entries.len();
        }
        if known != self.oldest && !self.changes.iter().any(|(version, _)| *version == known) {
            return 0;
        }
        self.changes.iter().filter(|(version, _)| *version > known).map(|(_, first)| *first).min().unwrap_or(0)
    }
}

/// A thread's history kept, and what reading on from it needs: the agents'
/// transcripts as read so far and the prompt the next turn starts with.
pub(crate) struct Kept {
    history: KeptHistory,
    reading: Reading,
    /// When it was last read, so the least recently read goes first.
    used: std::time::Instant,
}

#[derive(Default)]
struct Reading {
    reader: HistoryReader,
    last_prompt: Option<(usize, String)>,
}

/// A history read, entries and turns.
#[derive(Default)]
struct Read {
    entries: Vec<(DeltaId, HistoryEntry)>,
    turns: Vec<TurnInfo>,
}

impl Read {
    /// Takes in the history's next item, a delta of `source`, reading its
    /// entries cut as a page carries them.
    fn push(&mut self, reading: &mut Reading, delta: &Delta, source: HistorySource, rewound: bool) {
        let origin = match source {
            HistorySource::Own => "own",
            HistorySource::Inherited => "inherited",
            HistorySource::Upstream => "upstream",
        };
        // As the agent stamped its line, else when it was recorded.
        let time_ms = match &delta.op {
            Op::Conv(ConvOp::Native { line, .. }) => silt_agents::time::line_time(line).unwrap_or(delta.time_ms),
            _ => delta.time_ms,
        };
        for shown in entry_texts(&mut reading.reader, delta) {
            let index = self.entries.len();
            if opens_turn(&shown.kind) && shown.side.is_none() {
                reading.last_prompt = Some((index, shown.text.clone()));
            }
            let entry = shown.into_entry(index, delta.thread.to_string(), origin, time_ms, rewound);
            self.entries.push((delta.span.first(), silt_proto::view::cut_for_page(&entry)));
        }
        if source == HistorySource::Own
            && let Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. }) = delta.op
        {
            let (entry, prompt) = reading.last_prompt.take().map_or((None, String::new()), |(index, text)| (Some(index), text));
            self.turns.push(TurnInfo { number: self.turns.len() + 1, entry, prompt });
        }
    }
}

/// Which of a kept history's entries a page asks for.
fn page_range(kept: &KeptHistory, asked: PageAsked) -> (usize, usize) {
    let total = kept.entries.len();
    let (offset, limit) = match asked.tail {
        Some(tail) => {
            let end = asked.before.unwrap_or(total).min(total);
            let from = silt_proto::view::tail_from(|at| &kept.entries[at].1, end, tail);
            (from, end - from)
        }
        None => (asked.offset, asked.limit),
    };
    let from = asked.known.map_or(offset, |known| offset.min(kept.same_before(known))).min(total);
    (from, from + limit.min(total - from))
}

/// The deltas of `thread` among `added`, if each only adds entries at the
/// history's end: a rewind, a merge or a fork changes what came before.
fn only_appended(added: &[Delta], thread: ThreadId) -> Option<Vec<&Delta>> {
    let mut appended = Vec::new();
    for delta in added.iter().filter(|delta| delta.thread == thread) {
        match &delta.op {
            Op::Conv(ConvOp::Rewound { .. } | ConvOp::Merged { .. }) | Op::Merge { .. } | Op::Fork { .. } => return None,
            Op::Conv(_) => appended.push(delta),
            _ => {}
        }
    }
    Some(appended)
}

/// The first entry where `after` differs from `before`.
fn first_difference(before: &[(DeltaId, HistoryEntry)], after: &[(DeltaId, HistoryEntry)]) -> usize {
    before.iter().zip(after).position(|(old, new)| old.1 != new.1).unwrap_or_else(|| before.len().min(after.len()))
}

/// What a page of history asks for (`Request::HistoryRead`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageAsked {
    pub(crate) offset: usize,
    pub(crate) limit: usize,
    pub(crate) known: Option<usize>,
    pub(crate) cut: bool,
    pub(crate) tail: Option<usize>,
    pub(crate) before: Option<usize>,
}

/// A thread's history read whole off the core, at `version` of its family's
/// log: by the loader, with the family it loaded, or on a blocking thread.
pub(crate) struct WholeRead {
    pub(crate) thread: ThreadId,
    version: usize,
    read: Read,
    reading: Reading,
}

impl WholeRead {
    /// The length of the family's log it was read at.
    pub(crate) fn version(&self) -> usize {
        self.version
    }
}

impl std::fmt::Debug for WholeRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WholeRead").field("thread", &self.thread).field("version", &self.version).finish_non_exhaustive()
    }
}

/// A history's items, each delta with where it came from and whether a rewind
/// left it behind.
type Items<'a> = Vec<(&'a Delta, HistorySource, bool)>;

fn read_items<'a>(items: impl IntoIterator<Item = (&'a Delta, HistorySource, bool)>) -> (Read, Reading) {
    let mut read = Read::default();
    let mut reading = Reading::default();
    for (delta, source, rewound) in items {
        read.push(&mut reading, delta, source, rewound);
    }
    (read, reading)
}

/// `thread`'s history in `family`, read whole, as the loader reads it in a
/// family it just loaded, before the core holds it.
pub(crate) fn read_whole(family: &Family, thread: ThreadId) -> WholeRead {
    let items = family.history(thread).into_iter().map(|item| (item.delta, item.source, item.rewound));
    let (read, reading) = read_items(items);
    WholeRead { thread, version: family.log_len(), read, reading }
}

/// A history's items, cloned off the family, to read whole on a blocking
/// thread.
pub(crate) struct ToRead {
    thread: ThreadId,
    version: usize,
    items: Vec<(Delta, HistorySource, bool)>,
}

impl ToRead {
    pub(crate) fn read(self) -> WholeRead {
        let (read, reading) = read_items(self.items.iter().map(|(delta, source, rewound)| (delta, *source, *rewound)));
        WholeRead { thread: self.thread, version: self.version, read, reading }
    }
}

/// Keeps `history`, letting go of the least recently read when too many are
/// kept: clearing them all made switching back to any thread read its whole
/// history again.
fn keep(cache: &mut HashMap<ThreadId, Kept>, thread: ThreadId, history: KeptHistory, reading: Reading) {
    cache.remove(&thread);
    while cache.len() >= HISTORIES_KEPT {
        let Some(oldest) = cache.iter().min_by_key(|(_, kept)| kept.used).map(|(thread, _)| *thread) else {
            break;
        };
        cache.remove(&oldest);
    }
    cache.insert(thread, Kept { history, reading, used: std::time::Instant::now() });
}

impl Core {
    /// The thread's history as numbered entries, with the delta each came
    /// from. Works for threads only known from the server too. Kept until
    /// the family's log grows, as the web UI reads a long history a page at
    /// a time and each page would otherwise read the whole of it.
    pub(crate) fn history_entries(&self, thread: ThreadId) -> HistoryEntries {
        self.kept_history(thread).entries
    }

    /// The thread's history as kept (ADR 0117): what its family's log
    /// added since is read on from where it was, unless it changed what came
    /// before, which reads it again whole, noting where it first changed, so
    /// a viewer holding an earlier version reads only from there.
    fn kept_history(&self, thread: ThreadId) -> KeptHistory {
        let family = self.threads[&thread].family;
        let length = self.local_log_len(family);
        let mut cache = self.history_cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(kept) = cache.get_mut(&thread) {
            kept.used = std::time::Instant::now();
            if kept.history.version == length {
                return kept.history.clone();
            }
            if length > kept.history.version
                && let Some(added) = self.log_since(family, kept.history.version)
                && let Some(appended) = only_appended(added, thread)
            {
                let history = &mut kept.history;
                let before = history.entries.len();
                let mut read = Read {
                    entries: std::mem::take(Arc::make_mut(&mut history.entries)),
                    turns: std::mem::take(Arc::make_mut(&mut history.turns)),
                };
                for delta in appended {
                    read.push(&mut kept.reading, delta, HistorySource::Own, false);
                }
                history.entries = Arc::new(read.entries);
                history.turns = Arc::new(read.turns);
                // Each version is noted, those adding no entry too, or a
                // viewer holding one would be told to read it all again.
                history.note_change(length, before);
                history.version = length;
                return history.clone();
            }
        }
        let (read, reading) = read_items(self.history_items(thread));
        let history = match cache.remove(&thread) {
            Some(before) => before.history.grown(length, read),
            None => KeptHistory::new(length, read),
        };
        keep(&mut cache, thread, history.clone(), reading);
        history
    }

    /// Whether reading the thread's history now would read it whole: it
    /// isn't kept, or what its family's log added since changed what came
    /// before. Only for a family held, whose log is read here.
    pub(crate) fn history_unkept(&self, thread: ThreadId) -> bool {
        let family = self.threads[&thread].family;
        let length = self.local_log_len(family);
        let cache = self.history_cache.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(kept) = cache.get(&thread) else {
            return true;
        };
        let version = kept.history.version;
        version != length
            && (length < version || self.log_since(family, version).and_then(|added| only_appended(added, thread)).is_none())
    }

    /// The thread's history, cloned as it is now, to read whole off the core.
    pub(crate) fn history_to_read(&self, thread: ThreadId) -> ToRead {
        let version = self.local_log_len(self.threads[&thread].family);
        let items = self.history_items(thread).into_iter().map(|(delta, source, rewound)| (delta.clone(), source, rewound));
        ToRead { thread, version, items: items.collect() }
    }

    /// Keeps a history read whole off the core, unless what's kept is as
    /// new; what the log added since is read on from it when next asked.
    pub(crate) fn take_whole(&self, whole: WholeRead) {
        let WholeRead { thread, version, read, reading } = whole;
        if !self.threads.contains_key(&thread) {
            return;
        }
        let mut cache = self.history_cache.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.get(&thread).is_some_and(|kept| kept.history.version >= version) {
            return;
        }
        let history = match cache.remove(&thread) {
            Some(before) => before.history.grown(version, read),
            None => KeptHistory::new(version, read),
        };
        keep(&mut cache, thread, history, reading);
    }

    /// Whether the thread's history is kept as its family's log is now.
    pub(crate) fn history_kept_now(&self, thread: ThreadId) -> bool {
        let length = self.local_log_len(self.threads[&thread].family);
        let cache = self.history_cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.get(&thread).is_some_and(|kept| kept.history.version == length)
    }

    /// The thread's turns, read with its history.
    pub(crate) fn kept_turns(&self, thread: ThreadId) -> Arc<Vec<TurnInfo>> {
        Arc::clone(&self.kept_history(thread).turns)
    }

    /// The family's deltas from `from` on, in local order.
    pub(crate) fn log_since(&self, family: FamilyId, from: usize) -> Option<&[Delta]> {
        Some(crate::sync::log_since_in(&self.families, family, from))
    }

    /// The deltas a page of `thread`'s history asked for whole is read from,
    /// from the store, so an old entry read whole needn't read the log.
    pub(crate) async fn deltas_for_page(&self, thread: ThreadId, asked: PageAsked) -> HashMap<DeltaId, Delta> {
        if asked.cut {
            return HashMap::new();
        }
        let kept = self.kept_history(thread);
        let (from, to) = page_range(&kept, asked);
        let mut ids: Vec<DeltaId> = kept.entries[from..to].iter().map(|(id, _)| *id).collect();
        ids.dedup();
        let family = self.threads[&thread].family;
        match self.store.deltas_by_id(family, &ids).await {
            Ok(deltas) => deltas.into_iter().map(|delta| (delta.span.first(), delta)).collect(),
            Err(error) => {
                tracing::warn!(%error, "could not read a page's deltas from the store; they're read from the log");
                HashMap::new()
            }
        }
    }

    /// The thread's family's delta `id`: among those read for the page, else
    /// from the log.
    fn delta_of(&self, thread: ThreadId, id: DeltaId, read: &HashMap<DeltaId, Delta>) -> Option<Delta> {
        if let Some(delta) = read.get(&id) {
            return Some(delta.clone());
        }
        let family = self.threads.get(&thread)?.family;
        self.local_log(family).iter().find(|delta| delta.span.first() == id).cloned()
    }

    /// What the thread's history is read from, whole.
    fn history_items(&self, thread: ThreadId) -> Items<'_> {
        let family_id = self.threads[&thread].family;
        match self.families.get(&family_id) {
            Some(family) => family.history(thread).into_iter().map(|item| (item.delta, item.source, item.rewound)).collect(),
            None => {
                let log = self.local_log(family_id);
                let rewound = silt_log::rewound_in(log, thread);
                log.iter()
                    .filter(|delta| delta.thread == thread && matches!(delta.op, Op::Conv(_)))
                    .map(|delta| (delta, HistorySource::Own, rewound.contains(&delta.span.first())))
                    .collect()
            }
        }
    }

    /// The `page` of `kept` whole, as it was recorded, rather than cut: each
    /// entry read again from its delta alone. One that reads otherwise
    /// alone, as a transcript whose earlier lines change how a line reads
    /// may, comes from the history read again whole.
    fn whole_entries(
        &self,
        thread: ThreadId,
        kept: &[(DeltaId, HistoryEntry)],
        page: std::ops::Range<usize>,
        read: &HashMap<DeltaId, Delta>,
    ) -> Vec<HistoryEntry> {
        let entries = &kept[page.clone()];
        let mut whole = Vec::with_capacity(entries.len());
        let mut at = 0;
        while at < entries.len() {
            let delta = entries[at].0;
            let same = entries[at..].iter().take_while(|(id, _)| *id == delta).count();
            // A page may start partway through a delta's entries, as an image
            // the web UI asks for alone, after its tool's output.
            let earlier = kept[..page.start + at].iter().rev().take_while(|(id, _)| *id == delta).count();
            let first = entries[at].1.index;
            match self.delta_of(thread, delta, read).map(|delta| entry_texts(&mut HistoryReader::default(), &delta)) {
                Some(shown)
                    if shown.len() >= earlier + same
                        && entries[at..at + same]
                            .iter()
                            .zip(&shown[earlier..])
                            .all(|((_, cut), shown)| cut.kind == shown.kind) =>
                {
                    for ((_, cut), shown) in entries[at..at + same].iter().zip(shown.into_iter().skip(earlier)) {
                        let read = shown.into_entry(cut.index, cut.thread.clone(), &cut.origin, cut.time_ms, cut.rewound);
                        whole.push(HistoryEntry { text: read.text, image: read.image, cut: None, ..cut.clone() });
                    }
                }
                _ => {
                    whole.extend(self.whole_read_again(thread, first, entries.len() - at));
                    return whole;
                }
            }
            at += same;
        }
        whole
    }

    /// `count` entries from `first` of the thread's history read again whole.
    fn whole_read_again(&self, thread: ThreadId, first: usize, count: usize) -> Vec<HistoryEntry> {
        let family_id = self.threads[&thread].family;
        let Some(family) = self.families.get(&family_id) else {
            return Vec::new();
        };
        let mut reader = HistoryReader::default();
        let mut whole = Vec::new();
        for item in family.history(thread) {
            for shown in entry_texts(&mut reader, item.delta) {
                let index = whole.len();
                let origin = match item.source {
                    HistorySource::Own => "own",
                    HistorySource::Inherited => "inherited",
                    HistorySource::Upstream => "upstream",
                };
                let time_ms = match &item.delta.op {
                    Op::Conv(ConvOp::Native { line, .. }) => silt_agents::time::line_time(line).unwrap_or(item.delta.time_ms),
                    _ => item.delta.time_ms,
                };
                whole.push(shown.into_entry(index, item.delta.thread.to_string(), origin, time_ms, item.rewound));
            }
        }
        whole.into_iter().skip(first).take(count).collect()
    }

    /// A page of `thread`'s history (ADR 0117): from entry `offset`, or from
    /// where it changed since version `known` when that's earlier, or the
    /// last `tail` entries from the start of their turn. `cut` leaves out
    /// long outputs and images, as the web UI asks for them.
    pub(crate) fn history_page(&self, thread: ThreadId, asked: PageAsked, read: &HashMap<DeltaId, Delta>) -> HistoryPage {
        let kept = self.kept_history(thread);
        let total = kept.entries.len();
        let (from, to) = page_range(&kept, asked);
        HistoryPage {
            thread: thread.to_string(),
            total,
            entries: if asked.cut {
                kept.entries[from..to].iter().map(|(_, entry)| entry.clone()).collect()
            } else {
                self.whole_entries(thread, &kept.entries, from..to, read)
            },
            from: Some(from),
            version: Some(kept.version),
            tasks: kept.entries[..from]
                .iter()
                .filter(|(_, entry)| entry.task.is_some())
                .map(|(_, entry)| entry.clone())
                .collect(),
        }
    }

    /// Search within `thread`'s history, including what it inherited from
    /// the threads it was forked from and what was compacted away.
    pub(crate) async fn history_search(&self, thread: ThreadId, query: &str, limit: u32) -> Result<Vec<HistoryHit>, CoreError> {
        let entries = self.history_entries(thread);
        let mut by_delta: HashMap<DeltaId, Vec<usize>> = HashMap::new();
        for (delta, entry) in entries.iter() {
            by_delta.entry(*delta).or_default().push(entry.index);
        }
        let lineage: HashSet<String> = entries.iter().map(|(_, entry)| entry.thread.clone()).collect();
        let mut hits = Vec::new();
        for hit in self.store.search(query, limit.saturating_mul(8)).await? {
            if !lineage.contains(&hit.thread.to_string()) {
                continue;
            }
            let Some(indexes) = by_delta.get(&hit.delta) else {
                continue;
            };
            let index = indexes.iter().copied().find(|index| entries[*index].1.kind == hit.kind).unwrap_or(indexes[0]);
            hits.push(HistoryHit { index, kind: hit.kind, snippet: hit.snippet });
            if hits.len() >= limit as usize {
                break;
            }
        }
        Ok(hits)
    }

    /// The turns of `thread` (and what it inherited) that changed the file
    /// now at `path`, oldest first.
    pub(crate) fn file_history(&self, thread: ThreadId, path: &str) -> Result<Vec<FileTouch>, CoreError> {
        let family_id = self.threads[&thread].family;
        let family = self
            .families
            .get(&family_id)
            .ok_or_else(|| CoreError::Invalid("the thread isn't on this machine yet; resume or fork it first".into()))?;
        let view = family.thread_view(thread);
        let inode = view
            .paths()
            .get(path.trim_start_matches("./"))
            .copied()
            .ok_or_else(|| CoreError::Invalid(format!("{path} is not a file of this thread")))?;
        let head = family.graph().version_vector(view.version());
        Ok(file_touches(family, inode, |delta| head.contains(delta.span.first())))
    }

    /// Who else is changing `path` (relative to `cwd`): the server's view
    /// when connected, else what this machine knows.
    /// `caller`, the asking agent's thread, is left out; else the one checked
    /// out at `cwd`.
    pub(crate) async fn in_flight_anywhere(
        &self,
        cwd: &std::path::Path,
        path: &str,
        caller: Option<ThreadId>,
    ) -> Result<Vec<InFlight>, CoreError> {
        let (identity, relative, here) = self.repo_path(cwd, path)?;
        let here = caller.or(here);
        let Ok(remote) = self.server_for_project(&identity) else {
            return Ok(self.in_flight(&identity, &relative, here));
        };
        let reply = remote
            .in_flight(&silt_proto::InFlightQuery { repo: identity, path: relative })
            .await
            .map_err(|source| CoreError::Server { doing: "list the threads changing it", source })?;
        Ok(reply
            .threads
            .into_iter()
            .filter(|thread| Some(thread.thread) != here)
            .map(|thread| InFlight {
                thread: thread.thread.to_string(),
                owner: thread.owner,
                files: thread.paths,
                last_ms: thread.last_ms,
            })
            .collect())
    }

    /// The repository identity at `cwd`, `path` relative to the checkout
    /// root, and the thread checked out there.
    pub(crate) fn repo_path(&self, cwd: &std::path::Path, path: &str) -> Result<(String, String, Option<ThreadId>), CoreError> {
        let cwd = CanonicalCwd::new(cwd)?;
        let repo = silt_git::Repo::discover(&cwd)?;
        let identity = repo.project()?;
        let here = self.thread_checked_out_at(&cwd);
        let root = here
            .and_then(|_| {
                self.checkouts
                    .values()
                    .filter(|checkout| cwd.starts_with(&checkout.record.path))
                    .max_by_key(|checkout| checkout.record.path.as_os_str().len())
                    .map(|checkout| checkout.record.path.clone())
            })
            .unwrap_or_else(|| repo.root().to_owned());
        let relative = cwd.join(path);
        let relative = relative.strip_prefix(&root).unwrap_or(&relative).to_string_lossy().replace('\\', "/");
        Ok((identity, relative, here))
    }

    /// Other threads of the same repository that changed `path` (a file or
    /// a directory), most recently active first.
    pub(crate) fn in_flight(&self, repo: &str, path: &str, except: Option<ThreadId>) -> Vec<InFlight> {
        let path = path.trim_start_matches("./").trim_end_matches('/');
        let mut found = Vec::new();
        for record in self.threads.values().filter(|record| record.repo == repo) {
            if Some(record.id) == except {
                continue;
            }
            let (files, last_ms) = self.changed_under(record.id, path);
            if !files.is_empty() {
                found.push(InFlight { thread: record.id.to_string(), owner: record.owner.clone(), files, last_ms });
            }
        }
        found.sort_by_key(|item| std::cmp::Reverse(item.last_ms));
        found
    }

    /// The files under `path` (a file or a directory, `""` for all) the
    /// thread's own deltas changed, and when it last changed one.
    pub(crate) fn changed_under(&self, thread: ThreadId, path: &str) -> (Vec<String>, i64) {
        let path = path.trim_start_matches("./").trim_end_matches('/');
        let prefix = format!("{path}/");
        let mut files = Vec::new();
        let mut last_ms = 0;
        for (file, changed_ms) in self.files_changed_by(thread) {
            if path.is_empty() || file == path || file.starts_with(&prefix) {
                files.push(file);
                last_ms = last_ms.max(changed_ms);
            }
        }
        (files, last_ms)
    }

    /// Each file the thread's own deltas changed, by its path now, and when
    /// they last did, in path order.
    pub(crate) fn files_changed_by(&self, thread: ThreadId) -> Vec<(String, i64)> {
        let Some(record) = self.threads.get(&thread) else {
            return Vec::new();
        };
        if let Some(digested) = self.digested(record.family).and_then(|digest| digest.threads.get(&thread)) {
            return digested.touched.clone();
        }
        let Some(family) = self.viewed(record.family) else {
            return Vec::new();
        };
        let paths = family.thread_view(thread).paths();
        let names: HashMap<InodeId, &String> = paths.iter().map(|(name, inode)| (*inode, name)).collect();
        let mut files: BTreeMap<String, i64> = BTreeMap::new();
        for delta in family.log().iter().filter(|delta| delta.thread == thread) {
            if let Some(name) = touched_inode(delta).and_then(|inode| names.get(&inode)) {
                let changed = files.entry((*name).clone()).or_default();
                *changed = (*changed).max(delta.time_ms);
            }
        }
        files.into_iter().collect()
    }
}

impl Core {
    /// The thread's own turns, each with the history entry of its prompt.
    /// The thread's commits with the turns they came in (ADR 0030), and
    /// their subjects as this machine's clone of it has them.
    pub(crate) fn commits(&self, thread: ThreadId) -> Vec<CommitView> {
        if let Some(digested) = self.digested(self.threads[&thread].family).and_then(|digest| digest.threads.get(&thread)) {
            return digested.commits.clone();
        }
        let Some(family) = self.viewed(self.threads[&thread].family) else {
            return Vec::new();
        };
        let repo = self.code_of(thread).and_then(|path| silt_git::Repo::discover(&path).ok());
        silt_log::commits_in(family.log(), thread)
            .into_iter()
            .map(|commit| CommitView {
                subject: repo.as_ref().and_then(|repo| subject(repo, &commit.commit.to_string())),
                commit: commit.commit.to_string(),
                branch: commit.branch,
                turn: commit.turn,
                time_ms: commit.time_ms,
            })
            .collect()
    }

    /// Where the thread's turn `turn` ended (ADR 0029).
    pub(crate) fn turn_point(&self, thread: ThreadId, turn: usize) -> Option<silt_types::Frontier> {
        let family = self.families.get(&self.threads.get(&thread)?.family)?;
        let end = self.turn_end(thread, turn)?;
        Some(silt_log::thread_head_in(&family.log()[..end], family.graph(), thread))
    }

    /// How much of the family's log the thread's turn `turn` ended with,
    /// knowing its prompts from its transcripts.
    pub(crate) fn turn_end(&self, thread: ThreadId, turn: usize) -> Option<usize> {
        let family = self.families.get(&self.threads.get(&thread)?.family)?;
        let mut reader = HistoryReader::default();
        silt_log::turn_end_by(family.log(), thread, turn, |delta| {
            entry_texts(&mut reader, delta).iter().any(|shown| opens_turn(&shown.kind) && shown.side.is_none())
        })
    }

    /// The history graph over the families here, as `query` asks: turns,
    /// commits, forks, merge requests and merges, newest first.
    pub(crate) fn graph(&self, query: &GraphQuery) -> Vec<GraphRow> {
        let summaries: HashMap<String, crate::protocol::ThreadSummary> =
            self.thread_summaries().into_iter().map(|summary| (summary.id.clone(), summary)).collect();
        let mut rows = Vec::new();
        let thread_family = match &query.thread {
            Some(wanted) => match self.find_thread(wanted).ok().and_then(|thread| self.threads.get(&thread)) {
                Some(record) => Some(record.family),
                None => return Vec::new(),
            },
            None => None,
        };
        // A person's memory is a family only to sync it (ADR 0137).
        let families: Vec<FamilyId> = self
            .families
            .keys()
            .copied()
            .filter(|family| !self.threads.values().any(|record| record.family == *family && silt_types::is_memory(&record.repo)))
            .collect();
        for family_id in families {
            if query.family.as_ref().is_some_and(|wanted| *wanted != family_id.to_string())
                || thread_family.is_some_and(|wanted| wanted != family_id)
            {
                continue;
            }
            for mut row in self.graph_in(family_id) {
                let Some(summary) = summaries.get(&row.thread) else {
                    continue;
                };
                let mine = || row.thread.parse::<ThreadId>().is_ok_and(|thread| summary.owner == self.own_user(thread));
                if query.project.as_ref().is_some_and(|project| *project != summary.repo)
                    || (query.mine && !summary.owner.is_empty() && !mine())
                    || !query.shows(&row.kind)
                {
                    continue;
                }
                row.owner.clone_from(&summary.owner);
                row.agent.clone_from(&summary.agent);
                row.settled = summary.mine.settled;
                rows.push(row);
            }
        }
        query.page(rows)
    }

    /// The family's rows of the history graph, without what the threads'
    /// summaries add: from the family if held, else from its digest.
    pub(crate) fn graph_in(&self, family_id: FamilyId) -> Vec<GraphRow> {
        if let Some(digest) = self.digested(family_id) {
            return digest.graph.clone();
        }
        let Some(family) = self.viewed(family_id) else {
            return Vec::new();
        };
        let mut turns: HashMap<ThreadId, Vec<TurnInfo>> = HashMap::new();
        let mut commits: HashMap<ThreadId, Vec<CommitView>> = HashMap::new();
        let requests = family.merge_requests();
        let mut rows = Vec::new();
        for item in silt_log::graph_of(family.log()) {
            let thread = graph_thread(&item.event);
            if !self.threads.contains_key(&thread) {
                continue;
            }
            let (kind, turn, text, other, status) = match &item.event {
                GraphEvent::Turn { number, .. } => {
                    let turns = turns.entry(thread).or_insert_with(|| self.turns(thread));
                    let prompt = turns.get(number - 1).map(|turn| first_line(&turn.prompt)).unwrap_or_default();
                    ("turn", Some(*number), prompt, None, None)
                }
                GraphEvent::Commit { commit, turn, .. } => {
                    let commits = commits.entry(thread).or_insert_with(|| self.commits(thread));
                    let id = commit.to_string();
                    let subject = commits.iter().find(|known| known.commit == id).and_then(|known| known.subject.clone());
                    ("commit", *turn, subject.unwrap_or_else(|| id[..8].to_owned()), None, None)
                }
                GraphEvent::Fork { from, .. } => {
                    ("fork", None, format!("forked from {}", from.short()), Some(from.to_string()), None)
                }
                GraphEvent::Request { target, mr, title, .. } => {
                    let status = requests.iter().find(|request| request.id == *mr).map(|request| match request.status {
                        silt_log::MrStatus::Open => "open",
                        silt_log::MrStatus::Approved => "approved",
                        silt_log::MrStatus::Declined { .. } => "declined",
                    });
                    ("request", None, title.clone(), Some(target.to_string()), status.map(str::to_owned))
                }
                GraphEvent::Merged { from, title, .. } => ("merge", None, title.clone(), Some(from.to_string()), None),
            };
            let commit = match &item.event {
                GraphEvent::Commit { commit, .. } => Some(commit.to_string()),
                _ => None,
            };
            rows.push(GraphRow {
                commit,
                id: item.delta.to_string(),
                kind: kind.to_owned(),
                thread: thread.to_string(),
                family: family_id.to_string(),
                time_ms: item.time_ms,
                turn,
                text,
                other,
                status,
                owner: String::new(),
                agent: None,
                settled: false,
            });
        }
        rows
    }

    /// Each thread here that made a commit whose id starts with `prefix`,
    /// with the turn it came in.
    pub(crate) fn find_commit(&self, prefix: &str) -> Result<Vec<(String, Option<usize>)>, CoreError> {
        let prefix = prefix.trim().to_lowercase();
        if prefix.len() < 4 || !prefix.chars().all(|char| char.is_ascii_hexdigit()) {
            return Err(CoreError::Invalid("give at least four hex digits of the commit".into()));
        }
        let mut found = Vec::new();
        for thread in self.threads.keys() {
            for commit in self.commits(*thread) {
                if commit.commit.starts_with(&prefix) {
                    found.push((thread.to_string(), commit.turn));
                }
            }
        }
        found.sort();
        Ok(found)
    }

    /// Where this machine has the thread's commits: its checkout, or a
    /// clone of its project.
    fn code_of(&self, thread: ThreadId) -> Option<std::path::PathBuf> {
        match self.checkouts.values().find(|checkout| checkout.record.thread == thread) {
            Some(checkout) => Some(checkout.record.path.clone()),
            None => self.clone_of(thread).ok(),
        }
    }

    pub(crate) fn turns(&self, thread: ThreadId) -> Vec<TurnInfo> {
        let Some(family) = self.families.get(&self.threads[&thread].family) else {
            return Vec::new();
        };
        let mut turns = Vec::new();
        let mut entries = 0;
        let mut last_prompt: Option<(usize, String)> = None;
        let mut reader = HistoryReader::default();
        for item in family.history(thread) {
            for shown in entry_texts(&mut reader, item.delta) {
                if opens_turn(&shown.kind) && shown.side.is_none() {
                    last_prompt = Some((entries, shown.text));
                }
                entries += 1;
            }
            if item.source == HistorySource::Own
                && let Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. }) = item.delta.op
            {
                let (entry, prompt) = last_prompt.take().map_or((None, String::new()), |(index, text)| (Some(index), text));
                turns.push(TurnInfo { number: turns.len() + 1, entry, prompt });
            }
        }
        turns
    }

    /// The thread's history in order, marking where its own turns started
    /// and where its agent compacted its context, to place a search hit.
    pub(crate) fn timeline(&self, thread: ThreadId) -> Option<Timeline> {
        let family = self.families.get(&self.threads.get(&thread)?.family)?;
        let mut reader = HistoryReader::default();
        let items = family
            .history(thread)
            .into_iter()
            .map(|item| Moment {
                delta: item.delta.span.first(),
                time_ms: item.delta.time_ms,
                prompt: item.source == HistorySource::Own
                    && matches!(item.delta.op, Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. })),
                said: matches!(item.delta.op, Op::Conv(ConvOp::Native { .. })),
                compaction: entry_texts(&mut reader, item.delta)
                    .iter()
                    .any(|shown| shown.kind == "compaction" && shown.side.is_none()),
            })
            .collect();
        Some(Timeline { items })
    }
}

#[derive(Debug)]
struct Moment {
    delta: silt_types::DeltaId,
    time_ms: i64,
    prompt: bool,
    /// A line of the agent's session: something said.
    said: bool,
    compaction: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Timeline {
    items: Vec<Moment>,
}

impl Timeline {
    /// Where a hit is: at its delta when known, else after the last thing
    /// said by its time.
    fn position(&self, delta: Option<silt_types::DeltaId>, at: i64) -> Option<usize> {
        delta
            .and_then(|delta| self.items.iter().position(|moment| moment.delta == delta))
            .or_else(|| self.items.iter().rposition(|moment| moment.time_ms <= at))
    }

    /// The turn a hit belongs to, counted as `silt diff --turn` counts
    /// them, and whether the agent compacted its context after it. A
    /// prompt starts its turn whichever comes first, its line or the turn's
    /// mark: hooks mark the turn first, ACP agents after the prompt's line.
    pub(crate) fn place(&self, delta: Option<silt_types::DeltaId>, at: i64, prompt: bool) -> (Option<usize>, bool) {
        let Some(position) = self.position(delta, at) else {
            return (None, false);
        };
        let marked_after =
            || self.items[position + 1..].iter().find(|moment| moment.prompt || moment.said).is_some_and(|moment| moment.prompt);
        let started =
            self.items[..=position].iter().filter(|moment| moment.prompt).count() + usize::from(prompt && marked_after());
        let compacted = self.items[position + 1..].iter().any(|moment| moment.compaction);
        ((started > 0).then_some(started), compacted)
    }
}

/// The history entries one delta contributes, as kind and text.
fn graph_thread(event: &GraphEvent) -> ThreadId {
    match event {
        GraphEvent::Turn { thread, .. }
        | GraphEvent::Commit { thread, .. }
        | GraphEvent::Fork { thread, .. }
        | GraphEvent::Request { thread, .. }
        | GraphEvent::Merged { thread, .. } => *thread,
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().chars().take(120).collect()
}

/// A commit's subject line, when the clone has the commit.
fn subject(repo: &silt_git::Repo, commit: &str) -> Option<String> {
    let out = repo.git(&["log", "-1", "--format=%s", commit, "--"], None).ok()?;
    let subject = String::from_utf8_lossy(&out).trim().to_owned();
    (!subject.is_empty()).then_some(subject)
}

fn entry_texts(reader: &mut HistoryReader, delta: &Delta) -> Vec<Shown> {
    match &delta.op {
        Op::Conv(ConvOp::Native { .. }) => shown_entries(reader, delta),
        Op::Conv(ConvOp::Merged { from, title, summary, .. }) => {
            vec![Shown::plain("merge", format!("merged \"{title}\" from fork {from}: {summary}"))]
        }
        Op::Merge { from, mr: None } => vec![Shown::plain("pull", format!("pulled newer work from thread {from}"))],
        _ => Vec::new(),
    }
}

/// Edits to `inode` grouped by turn: the prompt that started the turn and
/// the tool calls that made them.
fn file_touches(family: &Family, inode: InodeId, include: impl Fn(&Delta) -> bool) -> Vec<FileTouch> {
    let mut touches: Vec<FileTouch> = Vec::new();
    let mut prompts: HashMap<ThreadId, String> = HashMap::new();
    let mut pending_prompt: HashMap<ThreadId, String> = HashMap::new();
    let mut transcripts = Transcripts::default();
    for delta in family.log().iter().filter(|delta| include(delta)) {
        match &delta.op {
            Op::Conv(ConvOp::Native { .. }) => {
                for entry in native_entries(&mut transcripts, delta) {
                    if let Entry::User { text } = entry {
                        pending_prompt.insert(delta.thread, text);
                    }
                }
            }
            Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. }) => {
                if let Some(prompt) = pending_prompt.remove(&delta.thread) {
                    prompts.insert(delta.thread, prompt);
                }
            }
            _ => {}
        }
        if touched_inode(delta) != Some(inode) {
            continue;
        }
        let tool = match &delta.author {
            Author::Agent { tool_use_id, .. } => tool_use_id.clone(),
            _ => None,
        };
        let prompt = prompts.get(&delta.thread).cloned();
        let (inserted, deleted) = match &delta.op {
            Op::Text { op: silt_crdt::TextOp::Insert { content, .. }, .. } => (content.chars().count(), 0),
            Op::Text { op: silt_crdt::TextOp::Delete { len, .. }, .. } => (0, *len),
            _ => (0, 0),
        };
        match touches.last_mut() {
            Some(last) if last.thread == delta.thread.to_string() && last.prompt == prompt && last.tool_use_id == tool => {
                last.inserted += inserted;
                last.deleted += deleted;
                last.time_ms = delta.time_ms;
            }
            _ => touches.push(FileTouch {
                thread: delta.thread.to_string(),
                prompt,
                tool_use_id: tool,
                by_agent: delta.author.is_agents(),
                inserted,
                deleted,
                time_ms: delta.time_ms,
            }),
        }
    }
    touches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(texts: &[&str]) -> Read {
        let delta = DeltaId::new(silt_types::ActorId(1), silt_types::Lamport(0));
        let entries = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                (
                    delta,
                    silt_agents::tool::Shown::plain("assistant", (*text).to_owned()).into_entry(
                        index,
                        "t".into(),
                        "own",
                        0,
                        false,
                    ),
                )
            })
            .collect();
        Read { entries, turns: Vec::new() }
    }

    #[test]
    fn a_viewer_reads_a_history_again_only_from_where_it_changed() {
        let kept = KeptHistory::new(3, entries(&["a", "b"]));
        assert_eq!(kept.same_before(3), 2, "nothing changed");
        let kept = kept.grown(5, entries(&["a", "b", "c"]));
        assert_eq!(kept.same_before(3), 2, "only added to");
        let kept = kept.grown(6, entries(&["a", "B", "c"]));
        assert_eq!(kept.same_before(5), 1);
        assert_eq!(kept.same_before(3), 1, "the earliest change since counts");
        assert_eq!(kept.same_before(6), 3);
        assert_eq!(kept.same_before(4), 0, "a version never read is read again whole");
        let kept = kept.grown(2, entries(&["x"]));
        assert_eq!(kept.same_before(6), 0, "an erased family starts over");
    }

    #[test]
    fn only_the_latest_changes_are_kept() {
        let mut kept = KeptHistory::new(0, entries(&["a"]));
        for version in 1..=CHANGES_KEPT + 5 {
            kept = kept.grown(version, entries(&["a"]));
        }
        assert_eq!(kept.same_before(CHANGES_KEPT + 4), 1);
        assert_eq!(kept.same_before(1), 0, "too old to say");
        assert_eq!(kept.same_before(5), 1, "the version before the oldest change kept");
    }

    #[test]
    fn the_least_recently_read_history_is_let_go_of_first() {
        let mut cache = HashMap::new();
        let threads: Vec<ThreadId> = (0..=HISTORIES_KEPT as u128).map(ThreadId).collect();
        for thread in &threads[..HISTORIES_KEPT] {
            keep(&mut cache, *thread, KeptHistory::new(1, entries(&["a"])), Reading::default());
        }
        cache.get_mut(&threads[0]).expect("kept").used = std::time::Instant::now();
        keep(&mut cache, threads[HISTORIES_KEPT], KeptHistory::new(1, entries(&["a"])), Reading::default());
        assert_eq!(cache.len(), HISTORIES_KEPT);
        assert!(cache.contains_key(&threads[0]), "read again lately, it stays");
        assert!(!cache.contains_key(&threads[1]), "read least lately, it goes");
        assert!(threads[2..].iter().all(|thread| cache.contains_key(thread)), "the others stay");
    }

    mod read_on {
        use std::process::Command;

        use silt_log::{Author, SessionFile, SessionRef};
        use silt_store::{Store, ThreadRecord};
        use silt_types::ActorId;

        use super::*;

        const THREAD: ThreadId = ThreadId(7);
        const FAMILY: FamilyId = FamilyId(9);

        fn session() -> SessionRef {
            SessionRef { agent: "claude-code".into(), id: "s".into() }
        }

        fn said(line: &str) -> Op {
            Op::Conv(ConvOp::Native { session: session(), file: SessionFile::Main, line: line.into() })
        }

        fn prompt(text: &str) -> Op {
            said(&serde_json::json!({"type": "user", "message": {"role": "user", "content": text}}).to_string())
        }

        fn reply(text: &str) -> Op {
            let content = serde_json::json!([{"type": "text", "text": text}]);
            said(&serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": content}}).to_string())
        }

        fn submitted() -> Op {
            Op::Conv(ConvOp::Turn { session: session(), mark: TurnMark::PromptSubmitted })
        }

        async fn core() -> (tempfile::TempDir, tempfile::TempDir, Core) {
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
            let (events, _) = tokio::sync::mpsc::unbounded_channel();
            let core = Core::load(store, ActorId(1), data.path(), events).await.expect("loads");
            (data, repo, core)
        }

        fn write(core: &mut Core, op: Op) -> Delta {
            let family = core.families.get_mut(&FAMILY).expect("projected");
            let delta = family.delta(ActorId(2), THREAD, Author::User, 1, family.thread_head(THREAD), op);
            family.apply(delta.clone(), &mut std::collections::HashMap::new()).expect("applies");
            delta
        }

        fn read_again(core: &Core) -> Read {
            read_items(core.history_items(THREAD)).0
        }

        #[tokio::test]
        async fn what_the_log_adds_is_read_on_and_reads_as_the_history_read_whole() {
            let (_data, _repo, mut core) = core().await;
            core.families.hold(
                FAMILY,
                crate::load::Loaded::Projected { family: Box::new(silt_log::Family::new()), covered: 0, outdated: false },
                None,
            );
            for op in [prompt("draw a tree"), submitted(), reply("Drawing it.")] {
                write(&mut core, op);
            }
            let first = core.kept_history(THREAD);
            assert_eq!(first.entries.len(), 2);
            for op in [prompt("now a bush"), submitted(), reply("A bush."), reply("Done.")] {
                write(&mut core, op);
            }
            let kept = core.kept_history(THREAD);
            let whole = read_again(&core);
            assert_eq!(kept.entries.as_slice(), whole.entries.as_slice(), "read on, as read whole");
            assert_eq!(*kept.turns, whole.turns);
            assert_eq!(kept.turns.len(), 2);
            assert_eq!(kept.same_before(first.version), 2, "a viewer at the first reading reads on from its end");
        }

        /// Deltas that add no entry, as an artifact's page data, change
        /// nothing a viewer holds: told otherwise, it read every entry again,
        /// and an open artifact's page lost what it watched.
        #[tokio::test]
        async fn what_adds_no_entry_leaves_what_a_viewer_holds_as_it_was() {
            let (_data, _repo, mut core) = core().await;
            core.families.hold(
                FAMILY,
                crate::load::Loaded::Projected { family: Box::new(silt_log::Family::new()), covered: 0, outdated: false },
                None,
            );
            for op in [prompt("draw a tree"), reply("Drawing it.")] {
                write(&mut core, op);
            }
            let first = core.kept_history(THREAD);
            write(&mut core, submitted());
            let second = core.kept_history(THREAD);
            assert!(second.version > first.version);
            assert_eq!(second.entries.len(), first.entries.len(), "the mark is no entry");
            assert_eq!(second.same_before(first.version), 2, "a viewer at the first reading holds all it needs");
            write(&mut core, reply("Done."));
            let third = core.kept_history(THREAD);
            assert_eq!(third.same_before(second.version), 2, "and one at the second reads on from its end");
            assert_eq!(third.same_before(first.version), 2);
        }

        #[tokio::test]
        async fn a_rewind_reads_the_history_again_whole() {
            let (_data, _repo, mut core) = core().await;
            core.families.hold(
                FAMILY,
                crate::load::Loaded::Projected { family: Box::new(silt_log::Family::new()), covered: 0, outdated: false },
                None,
            );
            let asked = write(&mut core, prompt("draw a tree"));
            write(&mut core, submitted());
            write(&mut core, reply("Drawing it."));
            assert!(!core.kept_history(THREAD).entries.iter().any(|(_, entry)| entry.rewound));
            write(&mut core, Op::Conv(ConvOp::Rewound { to: asked.span.first() }));
            let kept = core.kept_history(THREAD);
            assert_eq!(kept.entries.as_slice(), read_again(&core).entries.as_slice());
        }

        #[tokio::test]
        async fn a_page_reads_entries_whole_from_their_deltas() {
            let (_data, _repo, mut core) = core().await;
            core.families.hold(
                FAMILY,
                crate::load::Loaded::Projected { family: Box::new(silt_log::Family::new()), covered: 0, outdated: false },
                None,
            );
            let long = "line\n".repeat(20_000);
            write(&mut core, prompt("draw a tree"));
            write(&mut core, submitted());
            write(&mut core, reply(&long));
            let asked = |cut, tail| PageAsked { offset: 0, limit: usize::MAX, known: None, cut, tail, before: None };
            let read = core.deltas_for_page(THREAD, asked(false, None)).await;
            let whole = core.history_page(THREAD, asked(false, None), &read);
            assert_eq!(whole.entries[1].text, long);
            let tail = core.history_page(THREAD, asked(true, Some(1)), &HashMap::new());
            assert_eq!(tail.from, Some(0), "from the prompt of its turn");
        }
    }
}
