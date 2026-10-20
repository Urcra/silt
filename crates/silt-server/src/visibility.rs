//! Who sees what (ADR 0010). Visibility is per thread: the thread's own
//! setting, else its source's for a fork, else its project's, else `team`.
//! A delta of a hidden thread still reaches a user when a thread they see
//! builds on it (a pull or a merge) or when it is offered to such a thread
//! in a merge request, since that is how private work gets shared.

use std::collections::{HashMap, HashSet};

use silt_log::{MrOp, Op};
use silt_sync::Room;
use silt_types::{Frontier, ThreadId};
use sqlx::Row;
use sqlx::sqlite::SqlitePool;

use crate::id_from;

struct ThreadRow {
    owner: i64,
    family: u128,
    setting: Option<String>,
    forked_from: Option<ThreadId>,
    repo: String,
}

/// What one user can see.
#[derive(Debug, Default)]
pub(crate) struct Viewer {
    threads: HashSet<ThreadId>,
    /// Families with at least one visible thread.
    families: HashSet<u128>,
    /// Families this user started.
    owned: HashSet<u128>,
    known_threads: HashSet<ThreadId>,
}

impl Viewer {
    /// `forks` maps each fork to its source.
    pub(crate) async fn load(
        db: &SqlitePool,
        user: i64,
        forks: &HashMap<ThreadId, ThreadId>,
    ) -> Result<Self, sqlx::Error> {
        let projects: HashMap<String, String> = sqlx::query("SELECT repo, visibility FROM projects")
            .fetch_all(db)
            .await?
            .iter()
            .map(|row| (row.get("repo"), row.get("visibility")))
            .collect();
        let threads: HashMap<ThreadId, ThreadRow> =
            sqlx::query("SELECT id, family_id, owner_id, visibility, repo FROM threads")
                .fetch_all(db)
                .await?
                .iter()
                .map(|row| {
                    let id = ThreadId(id_from(row.get("id")));
                    (
                        id,
                        ThreadRow {
                            owner: row.get("owner_id"),
                            family: id_from(row.get("family_id")),
                            setting: row.get("visibility"),
                            forked_from: forks.get(&id).copied(),
                            repo: row.get("repo"),
                        },
                    )
                })
                .collect();
        let effective = |mut thread: ThreadId| -> String {
            // Fork chains are short and acyclic, but stay safe against bad data.
            for _ in 0..64 {
                let Some(row) = threads.get(&thread) else {
                    break;
                };
                if let Some(setting) = &row.setting {
                    return setting.clone();
                }
                match row.forked_from {
                    Some(source) if threads.contains_key(&source) => thread = source,
                    _ => return projects.get(&row.repo).cloned().unwrap_or_else(|| "team".into()),
                }
            }
            "team".into()
        };
        let mut viewer = Self {
            owned: sqlx::query("SELECT id FROM families WHERE owner_id = ?")
                .bind(user)
                .fetch_all(db)
                .await?
                .iter()
                .map(|row| id_from(row.get("id")))
                .collect(),
            known_threads: threads.keys().copied().collect(),
            ..Self::default()
        };
        for (id, row) in &threads {
            if row.owner == user || effective(*id) == "team" {
                viewer.threads.insert(*id);
                viewer.families.insert(row.family);
            }
        }
        Ok(viewer)
    }

    pub(crate) fn sees_thread(&self, thread: ThreadId) -> bool {
        self.threads.contains(&thread)
    }

    pub(crate) fn sees_family(&self, family: u128) -> bool {
        self.families.contains(&family) || self.owned.contains(&family)
    }

    /// Whether the delta's own thread is visible. Threads the server hasn't
    /// been told about yet count as their family owner's.
    fn sees_thread_in(&self, family: u128, thread: ThreadId) -> bool {
        self.sees_thread(thread) || (!self.known_threads.contains(&thread) && self.owned.contains(&family))
    }

    /// A fingerprint of what this user sees in a family: the visible threads
    /// and the hidden deltas shared with them. It changes when something
    /// becomes readable that a client may have skipped, so the client then
    /// reads the family again from the start.
    pub(crate) fn view_of(&self, family: u128, room: &Room) -> u64 {
        let readable = self.readable(family, room);
        let mut threads: Vec<ThreadId> = Vec::new();
        let mut shared = Vec::new();
        for (delta, readable) in room.deltas().iter().zip(readable) {
            if self.sees_thread_in(family, delta.thread) {
                threads.push(delta.thread);
            } else if readable {
                shared.push(delta.span.first());
            }
        }
        threads.sort();
        threads.dedup();
        let mut hasher = blake3::Hasher::new();
        for thread in threads {
            hasher.update(&thread.0.to_be_bytes());
        }
        for id in shared {
            hasher.update(&id.actor.0.to_be_bytes());
            hasher.update(&id.lamport.0.to_be_bytes());
        }
        u64::from_le_bytes(hasher.finalize().as_bytes()[..8].try_into().expect("8 bytes"))
    }

    /// Positions in `room` of the deltas this user may read.
    pub(crate) fn readable(&self, family: u128, room: &Room) -> Vec<bool> {
        let deltas = room.deltas();
        if deltas.iter().all(|delta| self.sees_thread_in(family, delta.thread)) {
            return vec![true; deltas.len()];
        }
        let mut shared: Vec<_> = deltas
            .iter()
            .filter(|delta| match &delta.op {
                Op::Mr(MrOp::Opened { target, .. }) => self.sees_thread(*target),
                _ => self.sees_thread_in(family, delta.thread),
            })
            .map(|delta| delta.span.last())
            .collect();
        shared.sort();
        shared.dedup();
        let reachable = room.graph().version_vector(&shared.into_iter().collect::<Frontier>());
        deltas
            .iter()
            .map(|delta| self.sees_thread_in(family, delta.thread) || reachable.contains(delta.span.first()))
            .collect()
    }
}
