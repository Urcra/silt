//! Who sees what (ADR 0010, ADR 0013). Visibility is per thread: the
//! thread's own setting, else its source's for a fork, else its project's,
//! else the default of the org the project belongs to. Only members of
//! that org see anything of it, and only the owner sees a private thread.
//! A delta of a hidden thread still reaches a user when a thread they see
//! builds on it (a pull or a merge) or when it is offered to such a thread
//! in a merge request, since that is how private work gets shared.

use std::collections::{HashMap, HashSet};

use silt_crdt::VersionVector;
use silt_log::{MrOp, Op};
use silt_proto::Visibility;
use silt_sync::Room;
use silt_types::{Frontier, ThreadId};
use sqlx::Row;
use sqlx::sqlite::SqlitePool;

use crate::id_from;

struct ThreadRow {
    owner: i64,
    family: u128,
    setting: Option<Visibility>,
    forked_from: Option<ThreadId>,
    repo: String,
}

struct Project {
    org: Option<i64>,
    setting: Option<Visibility>,
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

fn level(text: Option<String>) -> Option<Visibility> {
    text.and_then(|text| text.parse().ok())
}

impl Viewer {
    /// `forks` maps each fork to its source.
    pub(crate) async fn load(
        db: &SqlitePool,
        user: i64,
        forks: &HashMap<ThreadId, ThreadId>,
    ) -> Result<Self, sqlx::Error> {
        let projects: HashMap<String, Project> = sqlx::query("SELECT repo, org_id, visibility FROM projects")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.get("repo"),
                    Project {
                        org: row.get("org_id"),
                        setting: level(row.get("visibility")),
                    },
                )
            })
            .collect();
        let defaults: HashMap<i64, Visibility> = sqlx::query("SELECT id, default_visibility FROM orgs")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|row| (row.get("id"), level(row.get("default_visibility")).unwrap_or(Visibility::Org)))
            .collect();
        let orgs: HashSet<i64> = sqlx::query("SELECT org_id FROM org_members WHERE user_id = ?")
            .bind(user)
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|row| row.get("org_id"))
            .collect();
        let teams: HashSet<(i64, String)> = sqlx::query(
            "SELECT t.org_id, t.name FROM team_members m JOIN teams t ON t.id = m.team_id WHERE m.user_id = ?",
        )
        .bind(user)
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|row| (row.get("org_id"), row.get("name")))
        .collect();
        let threads: HashMap<ThreadId, ThreadRow> =
            sqlx::query("SELECT id, family_id, owner_id, visibility, repo FROM threads WHERE deleted_ms IS NULL")
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
                            setting: level(row.get("visibility")),
                            forked_from: forks.get(&id).copied(),
                            repo: row.get("repo"),
                        },
                    )
                })
                .collect();
        let effective = |mut thread: ThreadId| -> (Option<i64>, Visibility) {
            let org = threads.get(&thread).and_then(|row| projects.get(&row.repo)).and_then(|project| project.org);
            let fallback = |repo: &str| {
                let project = projects.get(repo);
                project
                    .and_then(|project| project.setting.clone())
                    .or_else(|| project.and_then(|project| project.org).and_then(|org| defaults.get(&org).cloned()))
                    .unwrap_or(Visibility::Org)
            };
            // Fork chains are short and acyclic, but stay safe against bad data.
            for _ in 0..64 {
                let Some(row) = threads.get(&thread) else {
                    break;
                };
                if let Some(setting) = &row.setting {
                    return (org, setting.clone());
                }
                match row.forked_from {
                    Some(source) if threads.contains_key(&source) => thread = source,
                    _ => return (org, fallback(&row.repo)),
                }
            }
            (org, Visibility::Private)
        };
        let mut viewer = Self {
            owned: sqlx::query("SELECT id FROM families WHERE owner_id = ?")
                .bind(user)
                .fetch_all(db)
                .await?
                .iter()
                .map(|row| id_from(row.get("id")))
                .collect(),
            // Deleted threads are known too: an unknown thread counts as its
            // family owner's, which would hand them someone's deleted
            // private thread.
            known_threads: sqlx::query("SELECT id FROM threads")
                .fetch_all(db)
                .await?
                .iter()
                .map(|row| ThreadId(id_from(row.get("id"))))
                .collect(),
            ..Self::default()
        };
        for (id, row) in &threads {
            let visible = row.owner == user
                || match effective(*id) {
                    (Some(org), visibility) if orgs.contains(&org) => match visibility {
                        Visibility::Private => false,
                        Visibility::Org => true,
                        Visibility::Team(team) => teams.contains(&(org, team)),
                    },
                    _ => false,
                };
            if visible {
                viewer.threads.insert(*id);
                viewer.families.insert(row.family);
            }
        }
        Ok(viewer)
    }

    pub(crate) fn sees_thread(&self, thread: ThreadId) -> bool {
        self.threads.contains(&thread)
    }

    pub(crate) fn visible_threads(&self) -> impl Iterator<Item = ThreadId> + '_ {
        self.threads.iter().copied()
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

    /// Every id of `room` this user may read: their threads' deltas and what
    /// those build on.
    pub(crate) fn readable_history(&self, family: u128, room: &Room) -> VersionVector {
        let readable = self.readable(family, room);
        let tips: Frontier = room
            .deltas()
            .iter()
            .zip(readable)
            .filter(|(_, readable)| *readable)
            .map(|(delta, _)| delta.span.last())
            .collect();
        room.graph().version_vector(&tips)
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
