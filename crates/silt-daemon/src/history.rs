//! Reading a thread's full history the way agents and people need it
//! (ADR 0011): numbered entries across compactions and forks, search within
//! that history, which turns changed a file, and who else is changing it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, PoisonError};

use silt_agents::tool::Shown;
use silt_agents::{Entry, Transcripts};
use silt_crdt::InodeId;
use silt_log::{Author, ConvOp, Delta, Family, GraphEvent, HistorySource, Op, TurnMark};
use silt_proto::view::{GraphQuery, GraphRow};
use silt_types::{DeltaId, ThreadId};

use crate::core::{CanonicalCwd, Core, CoreError, touched_inode};
use crate::protocol::{CommitView, FileTouch, HistoryEntry, HistoryHit, HistoryPage, InFlight, TurnInfo};
use crate::transcripts::native_entries;

/// Longest text of one entry in a history page.
const ENTRY_CHARS: usize = 4000;
/// Threads whose history entries are kept at once.
const HISTORIES_KEPT: usize = 16;

/// A thread's numbered history entries, each with the delta it came from.
pub(crate) type HistoryEntries = Arc<Vec<(DeltaId, HistoryEntry)>>;

impl Core {
    /// The thread's history as numbered entries, with the delta each came
    /// from. Works for threads only known from the server too. Kept until
    /// the family's log grows, as the web UI reads a long history a page at
    /// a time and each page would otherwise read the whole of it.
    pub(crate) fn history_entries(&self, thread: ThreadId) -> HistoryEntries {
        let length = self.local_log_len(self.threads[&thread].family);
        let mut cache = self.history_cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((cached, entries)) = cache.get(&thread)
            && *cached == length
        {
            return Arc::clone(entries);
        }
        let entries = Arc::new(self.read_history_entries(thread));
        if cache.len() >= HISTORIES_KEPT {
            cache.clear();
        }
        cache.insert(thread, (length, Arc::clone(&entries)));
        entries
    }

    fn read_history_entries(&self, thread: ThreadId) -> Vec<(DeltaId, HistoryEntry)> {
        let family_id = self.threads[&thread].family;
        let items: Vec<(&Delta, HistorySource, bool)> = match self.families.get(&family_id) {
            Some(family) => family.history(thread).into_iter().map(|item| (item.delta, item.source, item.rewound)).collect(),
            None => {
                let log = self.local_log(family_id);
                let rewound = silt_log::rewound_in(log, thread);
                log.iter()
                    .filter(|delta| delta.thread == thread && matches!(delta.op, Op::Conv(_)))
                    .map(|delta| (delta, HistorySource::Own, rewound.contains(&delta.span.first())))
                    .collect()
            }
        };
        let mut entries = Vec::new();
        let mut transcripts = Transcripts::default();
        for (delta, source, rewound) in items {
            let origin = match source {
                HistorySource::Own => "own",
                HistorySource::Inherited => "inherited",
                HistorySource::Upstream => "upstream",
            };
            for shown in entry_texts(&mut transcripts, delta) {
                let index = entries.len();
                entries.push((
                    delta.span.first(),
                    shown.into_entry(index, delta.thread.to_string(), origin, delta.time_ms, rewound),
                ));
            }
        }
        entries
    }

    /// A page of `thread`'s history, from entry `offset`.
    pub(crate) fn history_page(&self, thread: ThreadId, offset: usize, limit: usize) -> HistoryPage {
        let entries = self.history_entries(thread);
        let total = entries.len();
        HistoryPage {
            thread: thread.to_string(),
            total,
            entries: entries
                .iter()
                .skip(offset)
                .take(limit)
                .map(|(_, entry)| {
                    let mut entry = entry.clone();
                    if entry.text.chars().count() > ENTRY_CHARS {
                        entry.text = entry.text.chars().take(ENTRY_CHARS).collect::<String>() + " …";
                    }
                    entry
                })
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
    pub(crate) async fn in_flight_anywhere(&self, cwd: &std::path::Path, path: &str) -> Result<Vec<InFlight>, CoreError> {
        let (identity, relative, here) = self.repo_path(cwd, path)?;
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
        let identity = repo.remote_url("origin")?.map(|url| silt_git::normalize_remote(&url)).unwrap_or_default();
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
            let Some(family) = self.families.get(&record.family) else {
                continue;
            };
            let paths = family.thread_view(record.id).paths();
            let under: HashSet<InodeId> = paths
                .iter()
                .filter(|(candidate, _)| path.is_empty() || *candidate == path || candidate.starts_with(&format!("{path}/")))
                .map(|(_, inode)| *inode)
                .collect();
            let names: BTreeMap<InodeId, &String> = paths.iter().map(|(name, inode)| (*inode, name)).collect();
            let mut files = BTreeMap::new();
            let mut last_ms = 0;
            for delta in family.log().iter().filter(|delta| delta.thread == record.id) {
                if let Some(inode) = touched_inode(delta).filter(|inode| under.contains(inode)) {
                    files.insert(names[&inode].clone(), ());
                    last_ms = last_ms.max(delta.time_ms);
                }
            }
            if !files.is_empty() {
                found.push(InFlight {
                    thread: record.id.to_string(),
                    owner: record.owner.clone(),
                    files: files.into_keys().collect(),
                    last_ms,
                });
            }
        }
        found.sort_by_key(|item| std::cmp::Reverse(item.last_ms));
        found
    }
}

impl Core {
    /// The thread's own turns, each with the history entry of its prompt.
    /// The thread's commits with the turns they came in (ADR 0030), and
    /// their subjects as this machine's clone of it has them.
    pub(crate) fn commits(&self, thread: ThreadId) -> Vec<CommitView> {
        let Some(family) = self.families.get(&self.threads[&thread].family) else {
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
        let mut transcripts = Transcripts::default();
        silt_log::turn_end_by(family.log(), thread, turn, |delta| {
            entry_texts(&mut transcripts, delta).iter().any(|shown| shown.kind == "user")
        })
    }

    /// The history graph over the families here, as `query` asks: turns,
    /// commits, forks, merge requests and merges, newest first.
    pub(crate) fn graph(&self, query: &GraphQuery) -> Vec<GraphRow> {
        let summaries: HashMap<String, crate::protocol::ThreadSummary> =
            self.thread_summaries().into_iter().map(|summary| (summary.id.clone(), summary)).collect();
        let mut turns: HashMap<ThreadId, Vec<TurnInfo>> = HashMap::new();
        let mut commits: HashMap<ThreadId, Vec<CommitView>> = HashMap::new();
        let mut rows = Vec::new();
        for (family_id, family) in &self.families {
            if query.family.as_ref().is_some_and(|wanted| *wanted != family_id.to_string()) {
                continue;
            }
            let requests = family.merge_requests();
            for item in silt_log::graph_of(family.log()) {
                let thread = graph_thread(&item.event);
                let Some(summary) = summaries.get(&thread.to_string()) else {
                    continue;
                };
                if query.project.as_ref().is_some_and(|project| *project != summary.repo)
                    || (query.mine && !summary.owner.is_empty() && summary.owner != self.own_user(thread))
                {
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
                if !query.shows(kind) {
                    continue;
                }
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
                    owner: summary.owner.clone(),
                    agent: summary.agent.clone(),
                    settled: summary.mine.settled,
                });
            }
        }
        query.page(rows)
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
        let mut transcripts = Transcripts::default();
        for item in family.history(thread) {
            for shown in entry_texts(&mut transcripts, item.delta) {
                if shown.kind == "user" {
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
        let mut transcripts = Transcripts::default();
        let items = family
            .history(thread)
            .into_iter()
            .map(|item| Moment {
                delta: item.delta.span.first(),
                time_ms: item.delta.time_ms,
                prompt: item.source == HistorySource::Own
                    && matches!(item.delta.op, Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. })),
                compaction: entry_texts(&mut transcripts, item.delta).iter().any(|shown| shown.kind == "compaction"),
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
    /// them, and whether the agent compacted its context after it.
    pub(crate) fn place(&self, delta: Option<silt_types::DeltaId>, at: i64) -> (Option<usize>, bool) {
        let Some(position) = self.position(delta, at) else {
            return (None, false);
        };
        let started = self.items[..=position].iter().filter(|moment| moment.prompt).count();
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

fn entry_texts(transcripts: &mut Transcripts, delta: &Delta) -> Vec<Shown> {
    match &delta.op {
        Op::Conv(ConvOp::Native { .. }) => {
            native_entries(transcripts, delta).into_iter().filter_map(silt_agents::tool::shown).collect()
        }
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
                by_agent: matches!(delta.author, Author::Agent { .. }),
                inserted,
                deleted,
                time_ms: delta.time_ms,
            }),
        }
    }
    touches
}
