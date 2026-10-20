//! What the server derives from deltas to answer search and in-flight
//! queries (ADR 0011). It has no git blobs, so it can't project file text;
//! paths come from tree operations alone, later stamps winning.

use std::collections::HashMap;

use silt_agents::claude;
use silt_crdt::{InodeId, InodeKind, Parent};
use silt_log::{ConvOp, Delta, GitOp, MrOp, Op};
use silt_types::{ActorId, DeltaId, FamilyId, Lamport, ThreadId};
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
        let thread = delta.thread;
        let time_ms = delta.time_ms;
        let mut rows = vec![Row::Activity {
            thread,
            time_ms,
            branch: None,
        }];
        let stamp = (delta.span.first().lamport, delta.span.actor);
        let placements = self.placements.entry(family).or_default();
        let touched = match &delta.op {
            Op::Create { kind, parent, name, .. } => {
                let inode = InodeId(delta.span.first());
                placements.insert(
                    inode,
                    Placement {
                        parent: *parent,
                        name: name.clone(),
                        stamp,
                        dir: *kind == InodeKind::Dir,
                    },
                );
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
            Op::Conv(ConvOp::Native { line, .. }) => {
                for entry in claude::normalize(line) {
                    if let Some(body) = entry.searchable_text().filter(|body| !body.trim().is_empty()) {
                        rows.push(Row::Text {
                            thread,
                            kind: entry.kind().to_owned(),
                            body,
                            time_ms,
                        });
                    }
                }
                None
            }
            Op::Conv(ConvOp::Merged { title, summary, .. }) | Op::Mr(MrOp::Opened { title, summary, .. }) => {
                rows.push(Row::Text {
                    thread,
                    kind: "merge".into(),
                    body: format!("{title}\n{summary}"),
                    time_ms,
                });
                if let Op::Mr(_) = delta.op {
                    self.requests.insert(delta.span.first(), thread);
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
                rows[0] = Row::Activity {
                    thread,
                    time_ms,
                    branch: branch.clone(),
                };
                None
            }
            _ => None,
        };
        let file = touched.filter(|inode| {
            self.placements
                .get(&family)
                .and_then(|placements| placements.get(inode))
                .is_some_and(|placement| !placement.dir)
        });
        if let Some(path) = file.and_then(|inode| self.path(family, inode)) {
            rows.push(Row::Touch { thread, path, time_ms });
        }
        rows
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
            Row::Text {
                thread,
                kind,
                body,
                time_ms,
            } => {
                sqlx::query("INSERT INTO search (body, thread_id, kind, time_ms) VALUES (?, ?, ?, ?)")
                    .bind(body)
                    .bind(crate::id_bytes(thread.0))
                    .bind(kind)
                    .bind(time_ms)
                    .execute(&mut **tx)
                    .await?;
            }
            Row::Touch { thread, path, time_ms } => {
                sqlx::query(
                    "INSERT INTO touches (thread_id, path, last_ms) VALUES (?, ?, ?)
                     ON CONFLICT (thread_id, path) DO UPDATE SET last_ms = MAX(last_ms, excluded.last_ms)",
                )
                .bind(crate::id_bytes(thread.0))
                .bind(path)
                .bind(time_ms)
                .execute(&mut **tx)
                .await?;
            }
            Row::Activity { thread, time_ms, branch } => {
                sqlx::query(
                    "INSERT INTO thread_activity (thread_id, branch, last_ms) VALUES (?, ?, ?)
                     ON CONFLICT (thread_id) DO UPDATE SET
                         last_ms = MAX(last_ms, excluded.last_ms),
                         branch = COALESCE(excluded.branch, thread_activity.branch)",
                )
                .bind(crate::id_bytes(thread.0))
                .bind(branch)
                .bind(time_ms)
                .execute(&mut **tx)
                .await?;
            }
            Row::Merged { thread } => {
                sqlx::query(
                    "INSERT INTO thread_activity (thread_id, merged) VALUES (?, 1)
                     ON CONFLICT (thread_id) DO UPDATE SET merged = 1",
                )
                .bind(crate::id_bytes(thread.0))
                .execute(&mut **tx)
                .await?;
            }
        }
    }
    Ok(())
}

/// Empties the derived tables before they are rebuilt from the deltas.
pub(crate) async fn clear(tx: &mut Transaction<'_, Sqlite>) -> Result<(), sqlx::Error> {
    for table in ["DELETE FROM search", "DELETE FROM touches", "DELETE FROM thread_activity"] {
        sqlx::query(table).execute(&mut **tx).await?;
    }
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
        assert!(rows.contains(&Row::Touch {
            thread: ThreadId(1),
            path: "src/lexer.rs".into(),
            time_ms: 5
        }));
        let (_, rows) = add(
            &mut family,
            Op::Move {
                inode: InodeId(file),
                parent: Parent::Root,
                name: "lexer.rs".into(),
            },
        );
        assert!(rows.contains(&Row::Touch {
            thread: ThreadId(1),
            path: "lexer.rs".into(),
            time_ms: 5
        }));
    }
}
