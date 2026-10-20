//! Reading a thread's full history the way agents and people need it
//! (ADR 0011): numbered entries across compactions and forks, search within
//! that history, which turns changed a file, and who else is changing it.

use std::collections::{BTreeMap, HashMap, HashSet};

use silt_agents::{Entry, claude};
use silt_crdt::InodeId;
use silt_log::{Author, ConvOp, Delta, Family, HistorySource, Op, TurnMark};
use silt_types::{DeltaId, ThreadId};

use crate::core::{Core, CoreError, touched_inode};
use crate::protocol::{FileTouch, HistoryEntry, HistoryHit, HistoryPage, InFlight};

/// Longest text of one entry in a history page.
const ENTRY_CHARS: usize = 4000;

impl Core {
    /// The thread's history as numbered entries, with the delta each came
    /// from. Works for threads only known from the server too.
    fn history_entries(&self, thread: ThreadId) -> Vec<(DeltaId, HistoryEntry)> {
        let family_id = self.threads[&thread].family;
        let items: Vec<(&Delta, HistorySource)> = match self.families.get(&family_id) {
            Some(family) => family
                .history(thread)
                .into_iter()
                .map(|item| (item.delta, item.source))
                .collect(),
            None => self
                .local_log(family_id)
                .iter()
                .filter(|delta| delta.thread == thread && matches!(delta.op, Op::Conv(_)))
                .map(|delta| (delta, HistorySource::Own))
                .collect(),
        };
        let mut entries = Vec::new();
        for (delta, source) in items {
            let origin = match source {
                HistorySource::Own => "own",
                HistorySource::Inherited => "inherited",
                HistorySource::Upstream => "upstream",
            };
            let mut push = |kind: &str, text: String| {
                let index = entries.len();
                entries.push((
                    delta.span.first(),
                    HistoryEntry {
                        index,
                        thread: delta.thread.to_string(),
                        origin: origin.to_owned(),
                        kind: kind.to_owned(),
                        text,
                        time_ms: delta.time_ms,
                    },
                ));
            };
            match &delta.op {
                Op::Conv(ConvOp::Native { line, .. }) => {
                    for entry in claude::normalize(line) {
                        match entry {
                            Entry::Other { .. } => {}
                            Entry::ToolCall { name, input, .. } => push("tool_call", format!("{name} {input}")),
                            Entry::Compaction { summary } => push(
                                "compaction",
                                summary.unwrap_or_else(|| "(context compacted; everything above is still here)".into()),
                            ),
                            other => push(other.kind(), other.searchable_text().unwrap_or_default()),
                        }
                    }
                }
                Op::Conv(ConvOp::Merged { from, title, summary, .. }) => {
                    push("merge", format!("merged \"{title}\" from fork {from}: {summary}"));
                }
                Op::Merge { from, mr: None } => push("pull", format!("pulled newer work from thread {from}")),
                _ => {}
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
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|(_, mut entry)| {
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
        for (delta, entry) in &entries {
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
            let index = indexes
                .iter()
                .copied()
                .find(|index| entries[*index].1.kind == hit.kind)
                .unwrap_or(indexes[0]);
            hits.push(HistoryHit {
                index,
                kind: hit.kind,
                snippet: hit.snippet,
            });
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
        let Some(remote) = &self.remote else {
            return Ok(self.in_flight(&identity, &relative, here));
        };
        let reply = remote
            .in_flight(&silt_proto::InFlightQuery {
                repo: identity,
                path: relative,
            })
            .await
            .map_err(|error| CoreError::Invalid(format!("the server could not answer: {error}")))?;
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
    fn repo_path(&self, cwd: &std::path::Path, path: &str) -> Result<(String, String, Option<ThreadId>), CoreError> {
        let cwd = cwd.canonicalize()?;
        let repo = silt_git::Repo::discover(&cwd)?;
        let identity = repo
            .remote_url("origin")?
            .map(|url| silt_git::normalize_remote(&url))
            .unwrap_or_default();
        let here = self.thread_at(None, &cwd).ok();
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

/// Edits to `inode` grouped by turn: the prompt that started the turn and
/// the tool calls that made them.
fn file_touches(family: &Family, inode: InodeId, include: impl Fn(&Delta) -> bool) -> Vec<FileTouch> {
    let mut touches: Vec<FileTouch> = Vec::new();
    let mut prompts: HashMap<ThreadId, String> = HashMap::new();
    let mut pending_prompt: HashMap<ThreadId, String> = HashMap::new();
    for delta in family.log().iter().filter(|delta| include(delta)) {
        match &delta.op {
            Op::Conv(ConvOp::Native { line, .. }) => {
                for entry in claude::normalize(line) {
                    if let Entry::User { text } = entry {
                        pending_prompt.insert(delta.thread, text);
                    }
                }
            }
            Op::Conv(ConvOp::Turn {
                mark: TurnMark::PromptSubmitted,
                ..
            }) => {
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
            Op::Text {
                op: silt_crdt::TextOp::Insert { content, .. },
                ..
            } => (content.chars().count(), 0),
            Op::Text {
                op: silt_crdt::TextOp::Delete { len, .. },
                ..
            } => (0, *len),
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
