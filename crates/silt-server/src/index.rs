//! What the server derives from deltas to answer search and in-flight
//! queries (ADR 0011). It has no git blobs, so it can't project file text;
//! paths come from tree operations alone, later stamps winning.

use std::collections::{BTreeMap, HashMap, HashSet};

use silt_agents::Entry;
use silt_agents::history::HistoryReader;
use silt_crdt::{InodeId, InodeKind, Parent};
use silt_log::{CommentOp, ConvOp, Delta, GitOp, MrOp, Op, TurnMark};
use silt_types::{ActorId, BlobHash, DeltaId, FamilyId, Lamport, ThreadId};
use sqlx::{QueryBuilder, Sqlite, Transaction};

/// One inode's latest placement.
#[derive(Debug, Clone)]
struct Placement {
    parent: Parent,
    name: String,
    stamp: (Lamport, ActorId),
    dir: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Index {
    placements: HashMap<FamilyId, HashMap<InodeId, Placement>>,
    /// Each merge request's fork, by the request's id.
    requests: HashMap<DeltaId, ThreadId>,
    /// Each merge request's target, by the request's id.
    targets: HashMap<DeltaId, ThreadId>,
    /// Each fork's source.
    forks: HashMap<ThreadId, ThreadId>,
    /// Counts changes to `forks`, which visibility is worked out from;
    /// carried over when the index is rebuilt.
    pub(crate) forks_version: u64,
    /// The deltas that refer to each blob: whoever may read one of them
    /// may fetch the blob.
    blobs: HashMap<BlobHash, Vec<(FamilyId, DeltaId)>>,
    transcripts: HistoryReader,
    /// Who took part in each comment, its author first, by the comment's id.
    comment_people: HashMap<DeltaId, Vec<String>>,
    /// Each reply's comment.
    reply_roots: HashMap<DeltaId, DeltaId>,
    /// What a thread listing shows of each thread's own conversation.
    conversations: HashMap<ThreadId, Conversation>,
    /// Each thread's own history, as the hosted app pages through it.
    histories: HashMap<ThreadId, History>,
}

/// A thread's own history entries, its turns and what reverts rewound,
/// kept as its deltas come in rather than read from all of them for each
/// page the hosted app shows (ADR 0034).
#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    /// Each entry with its delta, in order; its index and whether it was
    /// rewound are set as a page is read.
    pub(crate) entries: Vec<(DeltaId, silt_proto::view::HistoryEntry)>,
    /// When each of the thread's turns began.
    pub(crate) prompts: Vec<i64>,
    /// The thread's own conversation, in order, where reverts find the
    /// turn they go back to.
    conversation: Vec<DeltaId>,
    positions: HashMap<DeltaId, usize>,
    pub(crate) rewound: HashSet<DeltaId>,
    /// It pulled from another thread, so its history reaches past its own
    /// deltas.
    pub(crate) pulled: bool,
}

impl History {
    fn take(&mut self, delta: &Delta, conv: &ConvOp) {
        let id = delta.span.first();
        self.positions.insert(id, self.conversation.len());
        self.conversation.push(id);
        match conv {
            ConvOp::Merged { from, title, summary, .. } => {
                let shown = crate::browse::merged_text(*from, title, summary);
                self.entries.push((id, crate::browse::history_entry(shown, delta.thread, delta.time_ms)));
            }
            ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. } => self.prompts.push(delta.time_ms),
            // As `silt_log::rewound_in` finds them.
            ConvOp::Rewound { to } => {
                if let Some(from) = self.positions.get(to).copied() {
                    let at = self.conversation.len() - 1;
                    self.rewound.extend(self.conversation[from + 1..at].iter().copied());
                }
            }
            _ => {}
        }
    }
}

/// What a thread listing shows of a thread's conversation, kept as its
/// lines come in rather than read from them for each listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Conversation {
    /// Prompts and replies.
    pub(crate) messages: usize,
    /// The text of the first prompt.
    pub(crate) first_prompt: Option<String>,
}

impl Conversation {
    fn take(&mut self, entry: &Entry) {
        match entry {
            Entry::User { .. } => {
                self.messages += 1;
                if self.first_prompt.is_none() {
                    self.first_prompt = Some(entry.searchable_text().unwrap_or_default());
                }
            }
            Entry::Assistant { .. } => self.messages += 1,
            _ => {}
        }
    }
}

/// A row to write for a delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Row {
    Text { thread: ThreadId, kind: String, body: String, time_ms: i64 },
    Touch { thread: ThreadId, path: String, time_ms: i64 },
    Activity { thread: ThreadId, time_ms: i64, branch: Option<String> },
    Merged { thread: ThreadId },
}

impl Index {
    /// Takes in a delta and returns the rows it adds to the index.
    pub(crate) fn observe(&mut self, family: FamilyId, delta: &Delta) -> Vec<Row> {
        if let Some(blob) = delta.blob() {
            self.blobs.entry(blob).or_default().push((family, delta.span.first()));
        }
        let thread = delta.thread;
        if let Op::Conv(conv) = &delta.op {
            self.histories.entry(thread).or_default().take(delta, conv);
        }
        if let Op::Merge { mr: None, .. } = delta.op {
            self.histories.entry(thread).or_default().pulled = true;
        }
        let time_ms = delta.time_ms;
        let mut rows = vec![Row::Activity { thread, time_ms, branch: None }];
        let stamp = (delta.span.first().lamport, delta.span.actor);
        let placements = self.placements.entry(family).or_default();
        let touched = match &delta.op {
            Op::Create { kind, parent, name, .. } => {
                let inode = InodeId(delta.span.first());
                placements.insert(inode, Placement { parent: *parent, name: name.clone(), stamp, dir: *kind == InodeKind::Dir });
                Some(inode)
            }
            Op::Move { inode, parent, name } => {
                if let Some(placement) = placements.get_mut(inode).filter(|placement| placement.stamp < stamp) {
                    placement.parent = *parent;
                    placement.name.clone_from(name);
                    placement.stamp = stamp;
                }
                Some(*inode)
            }
            Op::Delete { inode } | Op::Text { inode, .. } | Op::SetContent { inode, .. } | Op::SetExecutable { inode, .. } => {
                Some(*inode)
            }
            Op::Conv(ConvOp::Native { session, file, line }) => {
                let side = match file {
                    silt_log::SessionFile::Main => None,
                    silt_log::SessionFile::Subagent(name) => Some(name.as_str()),
                };
                let conversation = self.conversations.entry(thread).or_default();
                let shown = self.transcripts.line_with(&session.agent, &session.id, side, line, |entry| {
                    // A subagent's prompts and replies aren't the thread's own.
                    if side.is_none() {
                        conversation.take(entry);
                    }
                    if let Some(body) = entry.searchable_text().filter(|body| !body.trim().is_empty()) {
                        rows.push(Row::Text { thread, kind: entry.kind().to_owned(), body, time_ms });
                    }
                });
                let history = self.histories.entry(thread).or_default();
                for shown in shown {
                    history.entries.push((delta.span.first(), crate::browse::history_entry(shown, thread, time_ms)));
                }
                None
            }
            Op::Conv(ConvOp::Merged { title, summary, .. }) | Op::Mr(MrOp::Opened { title, summary, .. }) => {
                rows.push(Row::Text { thread, kind: "merge".into(), body: format!("{title}\n{summary}"), time_ms });
                if let Op::Mr(MrOp::Opened { target, .. }) = &delta.op {
                    self.requests.insert(delta.span.first(), thread);
                    self.targets.insert(delta.span.first(), *target);
                }
                None
            }
            Op::Fork { from } => {
                if self.forks.insert(thread, *from) != Some(*from) {
                    self.forks_version += 1;
                }
                None
            }
            Op::Comment(CommentOp::Added { by, .. }) => {
                self.comment_people.entry(delta.span.first()).or_default().insert(0, by.clone());
                None
            }
            Op::Comment(CommentOp::Replied { parent, by, .. }) => {
                let root = self.reply_roots.get(parent).copied().unwrap_or(*parent);
                self.reply_roots.insert(delta.span.first(), root);
                let people = self.comment_people.entry(root).or_default();
                if !people.contains(by) {
                    people.push(by.clone());
                }
                None
            }
            Op::Mr(MrOp::Approved { mr }) => {
                if let Some(fork) = self.requests.get(mr) {
                    rows.push(Row::Merged { thread: *fork });
                }
                None
            }
            Op::Git(GitOp::Genesis { branch, .. } | GitOp::Committed { branch, .. }) => {
                rows[0] = Row::Activity { thread, time_ms, branch: branch.clone() };
                None
            }
            _ => None,
        };
        let file = touched.filter(|inode| {
            self.placements.get(&family).and_then(|placements| placements.get(inode)).is_some_and(|placement| !placement.dir)
        });
        if let Some(path) = file.and_then(|inode| self.path(family, inode)) {
            rows.push(Row::Touch { thread, path, time_ms });
        }
        rows
    }

    /// Who took part in the comment a reply to `parent` joins, by name.
    pub(crate) fn comment_people(&self, parent: DeltaId) -> Vec<String> {
        let root = self.reply_roots.get(&parent).copied().unwrap_or(parent);
        self.comment_people.get(&root).cloned().unwrap_or_default()
    }

    pub(crate) fn history(&self, thread: ThreadId) -> Option<&History> {
        self.histories.get(&thread)
    }

    /// The capabilities the latest version of artifact `id` declares, in
    /// `thread` or what it was forked from (ADR 0072).
    pub(crate) fn artifact_capabilities(&self, thread: ThreadId, id: &str) -> Option<silt_proto::artifact::Capabilities> {
        let mut at = thread;
        for _ in 0..64 {
            let latest = self.histories.get(&at).and_then(|history| {
                history
                    .entries
                    .iter()
                    .filter_map(|(_, entry)| entry.artifact.as_ref())
                    .filter(|artifact| artifact.id == id)
                    .max_by_key(|artifact| artifact.version)
            });
            if let Some(latest) = latest {
                return latest.capabilities.clone();
            }
            at = *self.forks.get(&at)?;
        }
        None
    }

    /// Whether `thread`'s history reaches past its own deltas: it's a fork,
    /// or it pulled.
    pub(crate) fn has_lineage(&self, thread: ThreadId) -> bool {
        self.forks.contains_key(&thread) || self.histories.get(&thread).is_some_and(|history| history.pulled)
    }

    pub(crate) fn conversation(&self, thread: ThreadId) -> Conversation {
        self.conversations.get(&thread).cloned().unwrap_or_default()
    }

    /// The fork a merge request came from.
    pub(crate) fn request_fork(&self, mr: DeltaId) -> Option<ThreadId> {
        self.requests.get(&mr).copied()
    }

    /// The thread a merge request offers its fork's work to.
    pub(crate) fn request_target(&self, mr: DeltaId) -> Option<ThreadId> {
        self.targets.get(&mr).copied()
    }

    pub(crate) fn forks(&self) -> &HashMap<ThreadId, ThreadId> {
        &self.forks
    }

    /// The deltas that refer to `blob`.
    pub(crate) fn blob_references(&self, blob: BlobHash) -> Vec<(FamilyId, DeltaId)> {
        self.blobs.get(&blob).cloned().unwrap_or_default()
    }

    fn path(&self, family: FamilyId, inode: InodeId) -> Option<String> {
        let placements = self.placements.get(&family)?;
        let mut names = Vec::new();
        let mut at = inode;
        // A move cycle can't happen in the real tree, but the server only
        // keeps the latest placement of each inode, so bound the walk.
        for _ in 0..256 {
            let placement = placements.get(&at)?;
            names.push(placement.name.as_str());
            match placement.parent {
                Parent::Root => {
                    names.reverse();
                    return Some(names.join("/"));
                }
                Parent::Trash => return None,
                Parent::Dir(parent) => at = parent,
            }
        }
        None
    }
}

/// Rows written in one statement, well under SQLite's bound on variables.
const ROWS_PER_INSERT: usize = 500;

/// What writing a batch of rows in order leaves: its texts, the latest time
/// of each path touched, and one activity row for each thread.
#[derive(Debug, Default, PartialEq, Eq)]
struct Folded<'a> {
    texts: Vec<(ThreadId, &'a str, &'a str, i64)>,
    touches: BTreeMap<(ThreadId, &'a str), i64>,
    activity: BTreeMap<ThreadId, Activity<'a>>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Activity<'a> {
    last_ms: i64,
    branch: Option<&'a str>,
    merged: bool,
}

fn fold(rows: &[Row]) -> Folded<'_> {
    let mut folded = Folded::default();
    for row in rows {
        match row {
            Row::Text { thread, kind, body, time_ms } => folded.texts.push((*thread, kind, body, *time_ms)),
            Row::Touch { thread, path, time_ms } => {
                let last = folded.touches.entry((*thread, path)).or_insert(*time_ms);
                *last = (*last).max(*time_ms);
            }
            Row::Activity { thread, time_ms, branch } => {
                let activity = folded.activity.entry(*thread).or_default();
                activity.last_ms = activity.last_ms.max(*time_ms);
                activity.branch = branch.as_deref().or(activity.branch);
            }
            Row::Merged { thread } => folded.activity.entry(*thread).or_default().merged = true,
        }
    }
    folded
}

pub(crate) async fn write(tx: &mut Transaction<'_, Sqlite>, rows: &[Row]) -> Result<(), sqlx::Error> {
    let folded = fold(rows);
    for texts in folded.texts.chunks(ROWS_PER_INSERT) {
        let mut insert = QueryBuilder::<Sqlite>::new("INSERT INTO search (body, thread_id, kind, time_ms) ");
        insert.push_values(texts, |mut row, (thread, kind, body, time_ms)| {
            row.push_bind(*body).push_bind(crate::id_bytes(thread.0)).push_bind(*kind).push_bind(*time_ms);
        });
        insert.build().execute(&mut **tx).await?;
    }
    let touches: Vec<_> = folded.touches.into_iter().collect();
    for touches in touches.chunks(ROWS_PER_INSERT) {
        let mut insert = QueryBuilder::<Sqlite>::new("INSERT INTO touches (thread_id, path, last_ms) ");
        insert.push_values(touches, |mut row, ((thread, path), last_ms)| {
            row.push_bind(crate::id_bytes(thread.0)).push_bind(*path).push_bind(*last_ms);
        });
        insert.push(" ON CONFLICT (thread_id, path) DO UPDATE SET last_ms = MAX(last_ms, excluded.last_ms)");
        insert.build().execute(&mut **tx).await?;
    }
    let activity: Vec<_> = folded.activity.into_iter().collect();
    for activity in activity.chunks(ROWS_PER_INSERT) {
        let mut insert = QueryBuilder::<Sqlite>::new("INSERT INTO thread_activity (thread_id, branch, last_ms, merged) ");
        insert.push_values(activity, |mut row, (thread, activity)| {
            row.push_bind(crate::id_bytes(thread.0))
                .push_bind(activity.branch)
                .push_bind(activity.last_ms)
                .push_bind(activity.merged);
        });
        insert.push(
            " ON CONFLICT (thread_id) DO UPDATE SET
                 last_ms = MAX(last_ms, excluded.last_ms),
                 branch = COALESCE(excluded.branch, thread_activity.branch),
                 merged = MAX(merged, excluded.merged)",
        );
        insert.build().execute(&mut **tx).await?;
    }
    Ok(())
}

/// Empties the derived tables before they are rebuilt from the deltas.
pub(crate) async fn clear(tx: &mut Transaction<'_, Sqlite>) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM search").execute(&mut **tx).await?;
    sqlx::query!("DELETE FROM touches").execute(&mut **tx).await?;
    sqlx::query!("DELETE FROM thread_activity").execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use silt_log::{Author, Content, Family};

    use super::*;

    #[test]
    fn a_batch_folds_into_what_writing_its_rows_in_order_leaves() {
        let (one, two) = (ThreadId(1), ThreadId(2));
        let rows = [
            Row::Activity { thread: one, time_ms: 5, branch: Some("main".into()) },
            Row::Text { thread: one, kind: "user".into(), body: "hello".into(), time_ms: 5 },
            Row::Touch { thread: one, path: "src/lib.rs".into(), time_ms: 7 },
            Row::Activity { thread: one, time_ms: 9, branch: None },
            Row::Touch { thread: one, path: "src/lib.rs".into(), time_ms: 6 },
            Row::Activity { thread: one, time_ms: 3, branch: Some("topic".into()) },
            Row::Merged { thread: two },
        ];
        let folded = fold(&rows);
        assert_eq!(folded.texts, vec![(one, "user", "hello", 5)]);
        assert_eq!(folded.touches, BTreeMap::from([((one, "src/lib.rs"), 7)]));
        assert_eq!(
            folded.activity,
            BTreeMap::from([
                (one, Activity { last_ms: 9, branch: Some("topic"), merged: false }),
                (two, Activity { last_ms: 0, branch: None, merged: true }),
            ])
        );
    }

    #[test]
    fn a_threads_conversation_is_counted_as_its_lines_come_in() {
        let family = Family::new();
        let mut index = Index::default();
        let session = silt_log::SessionRef { agent: "claude".into(), id: "s".into() };
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"fix the parser\nthen test it"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"Read","input":{}}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":"thanks"}}"#,
        ];
        for line in lines {
            let op = Op::Conv(ConvOp::Native { session: session.clone(), file: silt_log::SessionFile::Main, line: line.into() });
            let delta = family.delta(ActorId(1), ThreadId(1), Author::User, 5, family.thread_head(ThreadId(1)), op);
            index.observe(FamilyId(1), &delta);
        }
        let conversation = index.conversation(ThreadId(1));
        assert_eq!(conversation.messages, 3, "two prompts and a reply; the tool call isn't a message");
        assert_eq!(conversation.first_prompt.as_deref(), Some("fix the parser\nthen test it"));
        assert_eq!(index.conversation(ThreadId(2)), Conversation::default());
    }

    #[test]
    fn paths_follow_creates_and_moves() {
        let mut family = Family::new();
        let mut index = Index::default();
        let mut add = |family: &mut Family, op: Op| {
            let delta = family.delta(ActorId(1), ThreadId(1), Author::User, 5, family.thread_head(ThreadId(1)), op);
            family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
            (delta.span.first(), index.observe(FamilyId(1), &delta))
        };
        let (src, _) = add(
            &mut family,
            Op::Create {
                kind: InodeKind::Dir,
                parent: Parent::Root,
                name: "src".into(),
                content: Content::Empty,
                executable: false,
            },
        );
        let (file, rows) = add(
            &mut family,
            Op::Create {
                kind: InodeKind::File,
                parent: Parent::Dir(InodeId(src)),
                name: "lexer.rs".into(),
                content: Content::Empty,
                executable: false,
            },
        );
        assert!(rows.contains(&Row::Touch { thread: ThreadId(1), path: "src/lexer.rs".into(), time_ms: 5 }));
        let (_, rows) = add(&mut family, Op::Move { inode: InodeId(file), parent: Parent::Root, name: "lexer.rs".into() });
        assert!(rows.contains(&Row::Touch { thread: ThreadId(1), path: "lexer.rs".into(), time_ms: 5 }));
    }
}
