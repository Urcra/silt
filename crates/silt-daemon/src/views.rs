use std::collections::BTreeMap;

use silt_agents::{Entry, claude};
use silt_crdt::InodeId;
use silt_log::{Author, ConvOp, Delta, Family, Op, TurnMark};
use silt_types::{Frontier, ThreadId};
use similar::TextDiff;

use crate::core::{Core, CoreError, touched_inode};
use crate::protocol::{FileEdit, SearchResult, ShowLine, ThreadSummary};

impl Core {
    pub(crate) fn thread_summaries(&self) -> Vec<ThreadSummary> {
        let device = self.remote.as_ref().map(|remote| remote.config.device);
        let mut summaries: Vec<ThreadSummary> = self
            .thread_records()
            .values()
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
                    title: title(&conversation),
                    created_ms: record.created_ms,
                    messages: conversation
                        .iter()
                        .flat_map(|delta| entries(delta))
                        .filter(|entry| matches!(entry, Entry::User { .. } | Entry::Assistant { .. }))
                        .count(),
                    edits: deltas.iter().filter(|delta| matches!(delta.op, Op::Text { .. })).count(),
                    location: location.into(),
                    forked_from: deltas.iter().find_map(|delta| match delta.op {
                        Op::Fork { from } => Some(from.to_string()),
                        _ => None,
                    }),
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
        let edits = self
            .families()
            .get(&family_id)
            .map(|family| edits_by_tool(family, thread))
            .unwrap_or_default();
        let conversation: Vec<&Delta> = self
            .local_log(family_id)
            .iter()
            .filter(|delta| delta.thread == thread && matches!(delta.op, Op::Conv(_)))
            .collect();
        let mut lines = Vec::new();
        for delta in conversation {
            match &delta.op {
                Op::Conv(ConvOp::Turn { mark, .. }) => {
                    if let TurnMark::PromptSubmitted | TurnMark::Stopped = mark {
                        lines.push(ShowLine::Turn {
                            mark: format!("{mark:?}"),
                        });
                    }
                }
                Op::Conv(ConvOp::Native { .. }) => {
                    for entry in entries(delta) {
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

    /// A unified diff of the thread's changes, or of one turn's (1-based).
    pub(crate) fn diff(&self, thread: ThreadId, turn: Option<usize>) -> Result<String, CoreError> {
        let family = self
            .families()
            .get(&self.thread_records()[&thread].family)
            .ok_or_else(|| CoreError::Invalid("the thread isn't on this machine yet; use `silt resume`".into()))?;
        let (from, to) = match turn {
            None => (imported_version(family, thread), family.thread_head(thread)),
            Some(turn) => {
                let marks = prompt_marks(family, thread);
                let start = turn
                    .checked_sub(1)
                    .and_then(|index| marks.get(index))
                    .ok_or_else(|| CoreError::Invalid(format!("thread has {} turns", marks.len())))?;
                let end = marks.get(turn).cloned().unwrap_or_else(|| family.thread_head(thread));
                (start.clone(), end)
            }
        };
        let mut out = String::new();
        for (path, inode) in family.thread_view(thread).paths() {
            let Some(doc) = family.text_doc(inode) else {
                continue;
            };
            let before = doc.text_at(family.graph(), &from);
            let after = doc.text_at(family.graph(), &to);
            if before == after {
                continue;
            }
            let diff = TextDiff::from_lines(&before, &after);
            out.push_str(
                &diff
                    .unified_diff()
                    .header(&format!("a/{path}"), &format!("b/{path}"))
                    .to_string(),
            );
        }
        Ok(out)
    }

    pub(crate) async fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchResult>, CoreError> {
        Ok(self
            .store()
            .search(query, limit)
            .await?
            .into_iter()
            .map(|hit| SearchResult {
                thread: hit.thread.to_string(),
                kind: hit.kind,
                snippet: hit.snippet,
            })
            .collect())
    }
}

fn entries(delta: &Delta) -> Vec<Entry> {
    match &delta.op {
        Op::Conv(ConvOp::Native { line, .. }) => claude::normalize(line),
        _ => Vec::new(),
    }
}

fn title(conversation: &[&Delta]) -> Option<String> {
    conversation.iter().rev().find_map(|delta| match &delta.op {
        Op::Conv(ConvOp::Native { line, .. }) => {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            (value.get("type")?.as_str()? == "ai-title")
                .then(|| value.get("aiTitle")?.as_str().map(str::to_owned))
                .flatten()
        }
        Op::Conv(ConvOp::Title(title)) => Some(title.clone()),
        _ => None,
    })
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
    counts
        .into_iter()
        .map(|(tool, files)| {
            let files = files
                .into_iter()
                .filter(|(inode, _)| family.tree().kind(*inode) != Some(silt_crdt::InodeKind::Dir))
                .map(|(inode, (inserted, deleted))| FileEdit {
                    path: family.tree().path_of(inode).unwrap_or_else(|| "(deleted)".into()),
                    inserted,
                    deleted,
                })
                .collect();
            (tool, files)
        })
        .collect()
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
