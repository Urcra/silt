use std::collections::{BTreeMap, BTreeSet, HashMap};

use silt_agents::{Entry, Transcripts};
use silt_crdt::{InodeId, InodeKind, VersionVector};
use silt_log::{Author, BaseText, ConvOp, Delta, Family, FileView, GitOp, HistoryItem, HistorySource, Op, TurnMark};
use silt_types::{FamilyId, Frontier, ThreadId};
use similar::TextDiff;

use crate::core::{Core, CoreError, RepoBases, touched_inode};
use crate::transcripts::native_entries;
use crate::protocol::{FileEdit, SearchFilters, SearchResult, ShowLine, ThreadSummary};

impl Core {
    /// Every thread here but those in the trash.
    pub(crate) fn thread_summaries(&self) -> Vec<ThreadSummary> {
        self.summaries(|thread| !self.deleted.contains(&thread))
    }

    pub(crate) fn summaries(&self, include: impl Fn(ThreadId) -> bool) -> Vec<ThreadSummary> {
        let device = self.remote.as_ref().map(|remote| remote.config.device);
        let mut summaries: Vec<ThreadSummary> = self
            .thread_records()
            .values()
            .filter(|record| include(record.id))
            .map(|record| {
                let deltas: Vec<&Delta> = self
                    .local_log(record.family)
                    .iter()
                    .filter(|delta| delta.thread == record.id)
                    .collect();
                let conversation: Vec<&Delta> = deltas
                    .iter()
                    .copied()
                    .filter(|delta| matches!(delta.op, Op::Conv(_)))
                    .collect();
                let lease = deltas.iter().rev().find_map(|delta| match delta.op {
                    Op::Lease { device, .. } => Some(device),
                    _ => None,
                });
                let checked_out = self.checkouts.values().any(|checkout| checkout.record.thread == record.id);
                let diverged = self.syncs.get(&record.family).is_some_and(|sync| sync.diverged().is_some());
                let location = if diverged {
                    "diverged"
                } else if lease.is_some_and(|holder| Some(holder) != device) {
                    "moved"
                } else if checked_out {
                    "here"
                } else {
                    "remote"
                };
                ThreadSummary {
                    id: record.id.to_string(),
                    repo: if record.repo.is_empty() {
                        record.repo_root.display().to_string()
                    } else {
                        record.repo.clone()
                    },
                    title: title(&conversation).or_else(|| {
                        let family = self.families.get(&record.family)?;
                        let inherited: Vec<&Delta> = family.history(record.id).into_iter().map(|item| item.delta).collect();
                        title(&inherited)
                    }),
                    created_ms: record.created_ms,
                    agent: conversation.iter().rev().find_map(|delta| match &delta.op {
                        Op::Conv(ConvOp::Native { session, .. }) => {
                            Some(silt_agents::agent(&session.agent).map_or_else(|| session.agent.clone(), |agent| agent.command().to_owned()))
                        }
                        _ => None,
                    }),
                    messages: {
                        let mut transcripts = Transcripts::default();
                        conversation
                            .iter()
                            .flat_map(|delta| native_entries(&mut transcripts, delta))
                            .filter(|entry| matches!(entry, Entry::User { .. } | Entry::Assistant { .. }))
                            .count()
                    },
                                        edits: deltas.iter().filter(|delta| matches!(delta.op, Op::Text { .. })).count(),
                    location: location.into(),
                    forked_from: deltas.iter().find_map(|delta| match delta.op {
                        Op::Fork { from } => Some(from.to_string()),
                        _ => None,
                    }),
                    owner: record.owner.clone(),
                    collaborators: self.collaborators.get(&record.id).cloned().unwrap_or_default(),
                    writable: self.may_write(record.id),
                }
            })
            .collect();
        summaries.sort_by_key(|summary| summary.created_ms);
        summaries
    }

    /// The thread's conversation across compactions, with the files each tool
    /// call changed listed after its result.
    pub(crate) fn show(&self, thread: ThreadId) -> Vec<ShowLine> {
        let family_id = self.thread_records()[&thread].family;
        if let Some(family) = self.families().get(&family_id) {
            return self.show_items(family, thread, family.history(thread));
        }
        let conversation: Vec<HistoryItem<'_>> = self
            .local_log(family_id)
            .iter()
            .filter(|delta| delta.thread == thread && matches!(delta.op, Op::Conv(_)))
            .map(|delta| HistoryItem {
                delta,
                source: HistorySource::Own,
            })
            .collect();
        self.show_lines(None, thread, conversation)
    }

    /// Lines for `items` of `thread`'s history, with headings where the
    /// history moves between inherited, upstream and own parts.
    pub(crate) fn show_items(&self, family: &Family, thread: ThreadId, items: Vec<HistoryItem<'_>>) -> Vec<ShowLine> {
        self.show_lines(Some(family), thread, items)
    }

    fn show_lines(&self, family: Option<&Family>, thread: ThreadId, items: Vec<HistoryItem<'_>>) -> Vec<ShowLine> {
        let edits = family.map(|family| edits_by_tool(family, thread)).unwrap_or_default();
        let mut lines = Vec::new();
        let mut transcripts = Transcripts::default();
        let mut segment = (HistorySource::Own, thread);
        for item in items {
            let delta = item.delta;
            if (item.source, delta.thread) != segment && (item.source != HistorySource::Own || segment.0 != HistorySource::Own) {
                let label = match item.source {
                    HistorySource::Inherited => format!("from thread {} (before this fork)", short_id(delta.thread)),
                    HistorySource::Upstream => format!("upstream progress in thread {}", short_id(delta.thread)),
                    HistorySource::Own => format!("thread {}", short_id(thread)),
                };
                lines.push(ShowLine::Segment { label });
            }
            segment = (item.source, delta.thread);
            match &delta.op {
                Op::Merge { from, mr: None } => lines.push(ShowLine::Event {
                    text: format!("pulled newer work from thread {}", short_id(*from)),
                }),
                Op::Conv(ConvOp::Merged { from, title, summary, .. }) => lines.push(ShowLine::Event {
                    text: format!(
                        "merged \"{title}\" from fork {} (read it with `silt show {}`): {summary}",
                        short_id(*from),
                        short_id(*from)
                    ),
                }),
                Op::Conv(ConvOp::Turn { mark, .. }) => {
                    if let TurnMark::PromptSubmitted | TurnMark::Stopped = mark {
                        lines.push(ShowLine::Turn {
                            mark: format!("{mark:?}"),
                        });
                    }
                }
                Op::Conv(ConvOp::Native { .. }) => {
                    for entry in native_entries(&mut transcripts, delta) {
                        match entry {
                            Entry::ToolCall { id, name, input } => lines.push(ShowLine::ToolCall {
                                id,
                                name,
                                input: input.to_string(),
                            }),
                            Entry::ToolResult { tool_use_id, text, .. } => {
                                lines.push(ShowLine::Entry {
                                    kind: "tool_result".into(),
                                    text,
                                });
                                if let Some(files) = edits.get(&Some(tool_use_id.clone())) {
                                    lines.push(ShowLine::Edits {
                                        tool_use_id: Some(tool_use_id),
                                        files: files.clone(),
                                    });
                                }
                            }
                            Entry::Compaction { summary } => lines.push(ShowLine::Compaction { summary }),
                            Entry::Other { .. } => {}
                            other => lines.push(ShowLine::Entry {
                                kind: other.kind().to_owned(),
                                text: other.searchable_text().unwrap_or_default(),
                            }),
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(files) = edits.get(&None) {
            lines.push(ShowLine::Edits {
                tool_use_id: None,
                files: files.clone(),
            });
        }
        lines
    }

    pub(crate) fn diff(&self, thread: ThreadId, turn: Option<usize>, base: Option<&str>) -> Result<String, CoreError> {
        let family_id = self.thread_records()[&thread].family;
        let family = self
            .families()
            .get(&family_id)
            .ok_or_else(|| CoreError::Invalid("the thread isn't on this machine yet; use `silt resume`".into()))?;
        let graph = family.graph();
        let head = family.thread_head(thread);
        let (from, to) = match (turn, base) {
            (Some(turn), _) => {
                let marks = prompt_marks(family, thread);
                let start = turn
                    .checked_sub(1)
                    .and_then(|index| marks.get(index))
                    .ok_or_else(|| CoreError::Invalid(format!("thread has {} turns", marks.len())))?;
                let end = marks.get(turn).cloned().unwrap_or_else(|| head.clone());
                (graph.version_vector(start), graph.version_vector(&end))
            }
            (None, base) => (diff_base(family, thread, base)?, graph.version_vector(&head)),
        };
        Ok(render_diff(&self.changes(family_id, &from, &to)))
    }

    /// How the family's files differ between two versions.
    pub(crate) fn changes(&self, family_id: FamilyId, before: &VersionVector, after: &VersionVector) -> Vec<FileChange> {
        let Some(family) = self.families().get(&family_id) else {
            return Vec::new();
        };
        let repo = self.family_repo(family_id).ok();
        let mut bases = repo.as_ref().map(RepoBases);
        let files_at = |vv: &VersionVector| -> BTreeMap<InodeId, String> {
            let tree = family.tree().at(|id| vv.contains(id));
            tree.paths()
                .into_iter()
                .filter(|(_, inode)| tree.kind(*inode) == Some(InodeKind::File))
                .map(|(path, inode)| (inode, path))
                .collect()
        };
        let (before_files, after_files) = (files_at(before), files_at(after));
        let inodes: BTreeSet<InodeId> = before_files.keys().chain(after_files.keys()).copied().collect();
        let mut changes = Vec::new();
        for inode in inodes {
            let (old_path, new_path) = (before_files.get(&inode).cloned(), after_files.get(&inode).cloned());
            let mut content = |present: bool, vv: &VersionVector| -> String {
                if !present {
                    return String::new();
                }
                match (family.text_doc(inode), family.file(inode)) {
                    (Some(doc), _) => doc.text_at_vv(vv),
                    (None, Some(FileView::GitText(oid))) => {
                        bases.as_mut().and_then(|bases| bases.text(oid)).unwrap_or_default()
                    }
                    _ => String::new(),
                }
            };
            let (old_text, new_text) = (content(old_path.is_some(), before), content(new_path.is_some(), after));
            if old_text != new_text || old_path != new_path {
                changes.push(FileChange {
                    inode,
                    old_path,
                    new_path,
                    old_text,
                    new_text,
                });
            }
        }
        changes.sort_by(|a, b| a.path().cmp(b.path()));
        changes
    }

    pub(crate) async fn search(&self, query: &str, limit: u32, filters: &SearchFilters) -> Result<Vec<SearchResult>, CoreError> {
        if let Some(remote) = &self.remote {
            let request = silt_proto::SearchQuery {
                q: query.to_owned(),
                repo: filters.repo.clone(),
                author: filters.author.clone(),
                status: filters.status.clone(),
                branch: filters.branch.clone(),
                file: filters.file.clone(),
                since: filters.since,
                until: filters.until,
                limit: Some(limit),
            };
            match remote.search(&request).await {
                Ok(reply) => {
                    return Ok(reply
                        .hits
                        .into_iter()
                        .map(|hit| SearchResult {
                            thread: hit.thread.to_string(),
                            kind: hit.kind,
                            snippet: hit.snippet,
                            owner: Some(hit.owner),
                            status: Some(hit.status),
                            repo: Some(hit.repo),
                        })
                        .collect());
                }
                Err(error) if !filters.is_empty() => {
                    return Err(CoreError::Invalid(format!("the server could not search: {error}")));
                }
                Err(error) => tracing::warn!(%error, "server search failed; searching this machine"),
            }
        } else if !filters.is_empty() {
            return Err(CoreError::Invalid("search filters need a server; use `silt auth login`".into()));
        }
        Ok(self
            .store()
            .search(query, limit)
            .await?
            .into_iter()
            .map(|hit| SearchResult {
                thread: hit.thread.to_string(),
                kind: hit.kind,
                snippet: hit.snippet,
                owner: None,
                status: None,
                repo: None,
            })
            .collect())
    }
}

/// The title the agent gave the conversation, or else its first prompt.
fn title(conversation: &[&Delta]) -> Option<String> {
    let named = conversation.iter().rev().find_map(|delta| match &delta.op {
        Op::Conv(ConvOp::Native { line, .. }) => {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            (value.get("type")?.as_str()? == "ai-title")
                .then(|| value.get("aiTitle")?.as_str().map(str::to_owned))
                .flatten()
        }
        Op::Conv(ConvOp::Title(title)) => Some(title.clone()),
        _ => None,
    });
    named.or_else(|| {
        let mut transcripts = Transcripts::default();
        conversation
            .iter()
            .flat_map(|delta| native_entries(&mut transcripts, delta))
            .find_map(|entry| match entry {
                Entry::User { text } => text.lines().find(|line| !line.trim().is_empty()).map(|line| clip_title(line.trim())),
                _ => None,
            })
    })
}

fn clip_title(text: &str) -> String {
    const TITLE_CHARS: usize = 60;
    if text.chars().count() <= TITLE_CHARS {
        text.to_owned()
    } else {
        text.chars().take(TITLE_CHARS).collect::<String>() + "…"
    }
}

/// Files each tool call changed (`None`: changes made outside tool calls).
fn edits_by_tool(family: &Family, thread: ThreadId) -> BTreeMap<Option<String>, Vec<FileEdit>> {
    let mut counts: BTreeMap<Option<String>, BTreeMap<InodeId, (usize, usize)>> = BTreeMap::new();
    for delta in family.log().iter().filter(|delta| delta.thread == thread) {
        let tool = match &delta.author {
            Author::Agent { tool_use_id, .. } => tool_use_id.clone(),
            Author::User => None,
            Author::Import => continue,
        };
        let Some(inode) = touched_inode(delta) else {
            continue;
        };
        let counter = counts.entry(tool).or_default().entry(inode).or_default();
        if let Op::Text { op, .. } = &delta.op {
            match op {
                silt_crdt::TextOp::Insert { content, .. } => counter.0 += content.chars().count(),
                silt_crdt::TextOp::Delete { len, .. } => counter.1 += len,
            }
        }
    }
    let paths: HashMap<InodeId, String> = family.tree().paths().into_iter().map(|(path, inode)| (inode, path)).collect();
    counts
        .into_iter()
        .map(|(tool, files)| {
            let files = files
                .into_iter()
                .filter(|(inode, _)| family.tree().kind(*inode) != Some(silt_crdt::InodeKind::Dir))
                .map(|(inode, (inserted, deleted))| FileEdit {
                    path: paths.get(&inode).cloned().unwrap_or_else(|| "(deleted)".into()),
                    inserted,
                    deleted,
                })
                .collect();
            (tool, files)
        })
        .collect()
}

/// One file's change between two versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileChange {
    pub(crate) inode: InodeId,
    pub(crate) old_path: Option<String>,
    pub(crate) new_path: Option<String>,
    pub(crate) old_text: String,
    pub(crate) new_text: String,
}

impl FileChange {
    pub(crate) fn path(&self) -> &str {
        self.new_path.as_deref().or(self.old_path.as_deref()).unwrap_or_default()
    }
}

/// Changes as a git-style unified diff, with renames.
pub(crate) fn render_diff(changes: &[FileChange]) -> String {
    let mut out = String::new();
    for change in changes {
        let shown = change.path();
        let (old, new) = (
            change.old_path.as_deref().unwrap_or(shown),
            change.new_path.as_deref().unwrap_or(shown),
        );
        out.push_str(&format!("diff --git a/{old} b/{new}\n"));
        if let (Some(old), Some(new)) = (&change.old_path, &change.new_path)
            && old != new
        {
            out.push_str(&format!("rename from {old}\nrename to {new}\n"));
        }
        if change.old_text != change.new_text {
            let old_name = change.old_path.as_ref().map_or_else(|| "/dev/null".to_owned(), |path| format!("a/{path}"));
            let new_name = change.new_path.as_ref().map_or_else(|| "/dev/null".to_owned(), |path| format!("b/{path}"));
            let diff = TextDiff::from_lines(&change.old_text, &change.new_text);
            out.push_str(&diff.unified_diff().header(&old_name, &new_name).to_string());
        }
    }
    out
}

/// The version a diff of `thread` starts from (ADR 0011's diff bases).
fn diff_base(family: &Family, thread: ThreadId, base: Option<&str>) -> Result<VersionVector, CoreError> {
    let graph = family.graph();
    let head = family.thread_head(thread);
    let in_thread = graph.version_vector(&head);
    // The version at each commit; the files are imported after genesis.
    let commits = || {
        family.log().iter().filter(|delta| in_thread.contains(delta.span.first())).filter_map(|delta| match &delta.op {
            Op::Git(GitOp::Genesis { commit, .. }) => Some((imported_version(family, delta.thread), *commit)),
            Op::Git(GitOp::Committed { commit, .. }) => Some((Frontier::single(delta.span.last()), *commit)),
            _ => None,
        })
    };
    let version = match base {
        None | Some("start") => start_version(family, thread),
        Some("merge-base") => match family.thread(thread).and_then(|state| state.forked_from) {
            Some(source) => return Ok(in_thread.intersection(&graph.version_vector(&family.thread_head(source)))),
            None => start_version(family, thread),
        },
        Some("last-commit") => commits()
            .next_back()
            .map(|(version, _)| version)
            .ok_or_else(|| CoreError::Invalid("the thread has no commits".into()))?,
        Some("last-turn") => prompt_marks(family, thread)
            .pop()
            .ok_or_else(|| CoreError::Invalid("the thread has no turns".into()))?,
        Some(other) => {
            let sha = other.strip_prefix("commit:").ok_or_else(|| {
                CoreError::Invalid(format!(
                    "unknown diff base {other:?}; use merge-base, last-commit, last-turn or commit:<sha>"
                ))
            })?;
            commits()
                .rfind(|(_, commit)| commit.to_string().starts_with(sha))
                .map(|(version, _)| version)
                .ok_or_else(|| CoreError::Invalid(format!("the thread never was at a commit {sha}")))?
        }
    };
    Ok(graph.version_vector(&version))
}

/// Where the thread's own work starts: its fork point, or the import of
/// its files.
fn start_version(family: &Family, thread: ThreadId) -> Frontier {
    family
        .thread(thread)
        .and_then(|state| state.fork_point.clone())
        .unwrap_or_else(|| imported_version(family, thread))
}

/// The version right after the thread's files were imported.
fn imported_version(family: &Family, thread: ThreadId) -> Frontier {
    family
        .log()
        .iter()
        .rev()
        .find(|delta| {
            delta.thread == thread
                && delta.author == Author::Import
                && matches!(
                    delta.op,
                    Op::Create { .. } | Op::SetContent { .. } | Op::Text { .. } | Op::Move { .. } | Op::Delete { .. }
                )
        })
        .map_or_else(Frontier::root, |delta| Frontier::single(delta.span.last()))
}

/// The version at the start of each prompt.
fn prompt_marks(family: &Family, thread: ThreadId) -> Vec<Frontier> {
    family
        .conversation(thread)
        .filter(|delta| {
            matches!(
                &delta.op,
                Op::Conv(ConvOp::Turn {
                    mark: TurnMark::PromptSubmitted,
                    ..
                })
            )
        })
        .map(|delta| Frontier::single(delta.span.first()))
        .collect()
}

fn short_id(thread: ThreadId) -> String {
    let id = thread.to_string();
    id[id.len().saturating_sub(12)..].to_owned()
}
