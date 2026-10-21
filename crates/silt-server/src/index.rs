//! What the server derives from deltas to answer search and in-flight
//! queries (ADR 0011). It has no git blobs, so it can't project file text;
//! paths come from tree operations alone, later stamps winning.

use std::collections::HashMap;

use silt_agents::Transcripts;
use silt_crdt::{InodeId, InodeKind, Parent};
use silt_log::{CommentOp, ConvOp, Delta, GitOp, MrOp, Op};
use silt_types::{ActorId, BlobHash, DeltaId, FamilyId, Lamport, ThreadId};
use sqlx::{Sqlite, Transaction};

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
    /// Each fork's source.
    forks: HashMap<ThreadId, ThreadId>,
    /// Counts changes to `forks`, which visibility is worked out from;
    /// carried over when the index is rebuilt.
    pub(crate) forks_version: u64,
    /// The deltas that refer to each blob: whoever may read one of them
    /// may fetch the blob.
    blobs: HashMap<BlobHash, Vec<(FamilyId, DeltaId)>>,
    transcripts: Transcripts,
    /// Who took part in each comment, its author first, by the comment's id.
    comment_people: HashMap<DeltaId, Vec<String>>,
    /// Each reply's comment.
    reply_roots: HashMap<DeltaId, DeltaId>,
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
                let file = match file {
                    silt_log::SessionFile::Main => "main".to_owned(),
                    silt_log::SessionFile::Subagent(name) => format!("subagent:{name}"),
                };
                for entry in self.transcripts.entries(&session.agent, &session.id, &file, line) {
                    if let Some(body) = entry.searchable_text().filter(|body| !body.trim().is_empty()) {
                        rows.push(Row::Text { thread, kind: entry.kind().to_owned(), body, time_ms });
                    }
                }
                None
            }
            Op::Conv(ConvOp::Merged { title, summary, .. }) | Op::Mr(MrOp::Opened { title, summary, .. }) => {
                rows.push(Row::Text { thread, kind: "merge".into(), body: format!("{title}\n{summary}"), time_ms });
                if let Op::Mr(_) = delta.op {
                    self.requests.insert(delta.span.first(), thread);
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

    /// The fork a merge request came from.
    pub(crate) fn request_fork(&self, mr: DeltaId) -> Option<ThreadId> {
        self.requests.get(&mr).copied()
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

pub(crate) async fn write(tx: &mut Transaction<'_, Sqlite>, rows: &[Row]) -> Result<(), sqlx::Error> {
    for row in rows {
        match row {
            Row::Text { thread, kind, body, time_ms } => {
                let thread = crate::id_bytes(thread.0);
                sqlx::query!(
                    "INSERT INTO search (body, thread_id, kind, time_ms) VALUES (?, ?, ?, ?)",
                    body,
                    thread,
                    kind,
                    time_ms
                )
                .execute(&mut **tx)
                .await?;
            }
            Row::Touch { thread, path, time_ms } => {
                let thread = crate::id_bytes(thread.0);
                sqlx::query!(
                    "INSERT INTO touches (thread_id, path, last_ms) VALUES (?, ?, ?)
                     ON CONFLICT (thread_id, path) DO UPDATE SET last_ms = MAX(last_ms, excluded.last_ms)",
                    thread,
                    path,
                    time_ms,
                )
                .execute(&mut **tx)
                .await?;
            }
            Row::Activity { thread, time_ms, branch } => {
                let thread = crate::id_bytes(thread.0);
                sqlx::query!(
                    "INSERT INTO thread_activity (thread_id, branch, last_ms) VALUES (?, ?, ?)
                     ON CONFLICT (thread_id) DO UPDATE SET
                         last_ms = MAX(last_ms, excluded.last_ms),
                         branch = COALESCE(excluded.branch, thread_activity.branch)",
                    thread,
                    branch,
                    time_ms,
                )
                .execute(&mut **tx)
                .await?;
            }
            Row::Merged { thread } => {
                let thread = crate::id_bytes(thread.0);
                sqlx::query!(
                    "INSERT INTO thread_activity (thread_id, merged) VALUES (?, 1)
                     ON CONFLICT (thread_id) DO UPDATE SET merged = 1",
                    thread,
                )
                .execute(&mut **tx)
                .await?;
            }
        }
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
