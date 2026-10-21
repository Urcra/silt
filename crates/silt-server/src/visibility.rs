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

use crate::{UserId, id_from};

struct ThreadRow {
    owner: UserId,
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
    /// Unique to this snapshot, for caching what is worked out from it.
    serial: u64,
    threads: HashSet<ThreadId>,
    /// Families with at least one visible thread.
    families: HashSet<u128>,
    /// Families this user started.
    owned: HashSet<u128>,
    known_threads: HashSet<ThreadId>,
}

/// What one user's visibility is decided from.
#[derive(Default)]
struct Facts {
    projects: HashMap<String, Project>,
    /// Each org's default visibility.
    defaults: HashMap<i64, Visibility>,
    /// The user's orgs.
    orgs: HashSet<i64>,
    /// The user's teams, by org.
    teams: HashSet<(i64, String)>,
    /// Threads that aren't deleted.
    threads: HashMap<ThreadId, ThreadRow>,
    /// Families the user started.
    owned: HashSet<u128>,
    /// Every thread, deleted ones too.
    known_threads: HashSet<ThreadId>,
}

fn level(text: Option<String>) -> Option<Visibility> {
    text.and_then(|text| text.parse().ok())
}

impl Facts {
    /// `forks` maps each fork to its source.
    async fn load(db: &SqlitePool, user: UserId, forks: &HashMap<ThreadId, ThreadId>) -> Result<Self, sqlx::Error> {
        let projects: HashMap<String, Project> = sqlx::query("SELECT repo, org_id, visibility FROM projects")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|row| (row.get("repo"), Project { org: row.get("org_id"), setting: level(row.get("visibility")) }))
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
        let teams: HashSet<(i64, String)> =
            sqlx::query("SELECT t.org_id, t.name FROM team_members m JOIN teams t ON t.id = m.team_id WHERE m.user_id = ?")
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
        let owned = sqlx::query("SELECT id FROM families WHERE owner_id = ?")
            .bind(user)
            .fetch_all(db)
            .await?
            .iter()
            .map(|row| id_from(row.get("id")))
            .collect();
        let known_threads = sqlx::query("SELECT id FROM threads")
            .fetch_all(db)
            .await?
            .iter()
            .map(|row| ThreadId(id_from(row.get("id"))))
            .collect();
        Ok(Self { projects, defaults, orgs, teams, threads, owned, known_threads })
    }

    /// The thread's org and the visibility it takes, following forks to
    /// their source and then the project and its org.
    fn effective(&self, mut thread: ThreadId) -> (Option<i64>, Visibility) {
        let org = self.threads.get(&thread).and_then(|row| self.projects.get(&row.repo)).and_then(|project| project.org);
        let fallback = |repo: &str| {
            let project = self.projects.get(repo);
            project
                .and_then(|project| project.setting.clone())
                .or_else(|| project.and_then(|project| project.org).and_then(|org| self.defaults.get(&org).cloned()))
                .unwrap_or(Visibility::Org)
        };
        // Fork chains are short and acyclic, but stay safe against bad data.
        for _ in 0..64 {
            let Some(row) = self.threads.get(&thread) else {
                break;
            };
            if let Some(setting) = &row.setting {
                return (org, setting.clone());
            }
            match row.forked_from {
                Some(source) if self.threads.contains_key(&source) => thread = source,
                _ => return (org, fallback(&row.repo)),
            }
        }
        (org, Visibility::Private)
    }

    fn sees(&self, user: UserId, thread: ThreadId, row: &ThreadRow) -> bool {
        row.owner == user
            || match self.effective(thread) {
                (Some(org), visibility) if self.orgs.contains(&org) => match visibility {
                    Visibility::Private => false,
                    Visibility::Org => true,
                    Visibility::Team(team) => self.teams.contains(&(org, team)),
                },
                _ => false,
            }
    }
}

impl Viewer {
    /// `forks` maps each fork to its source.
    pub(crate) async fn load(
        db: &SqlitePool,
        user: UserId,
        forks: &HashMap<ThreadId, ThreadId>,
        serial: u64,
    ) -> Result<Self, sqlx::Error> {
        Ok(Self::decide(user, Facts::load(db, user, forks).await?, serial))
    }

    fn decide(user: UserId, facts: Facts, serial: u64) -> Self {
        let (threads, families) =
            facts.threads.iter().filter(|(id, row)| facts.sees(user, **id, row)).map(|(id, row)| (*id, row.family)).unzip();
        // Deleted threads are known too: an unknown thread counts as its
        // family owner's, which would hand them someone's deleted private
        // thread.
        Self { serial, threads, families, owned: facts.owned, known_threads: facts.known_threads }
    }

    pub(crate) fn serial(&self) -> u64 {
        self.serial
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
        let tips: Frontier =
            room.deltas().iter().zip(readable).filter(|(_, readable)| *readable).map(|(delta, _)| delta.span.last()).collect();
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
        deltas.iter().map(|delta| self.sees_thread_in(family, delta.thread) || reachable.contains(delta.span.first())).collect()
    }

    #[cfg(test)]
    pub(crate) fn seeing(threads: impl IntoIterator<Item = ThreadId>) -> Self {
        Self { threads: threads.into_iter().collect(), ..Self::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: UserId = UserId(1);
    const OTHER: UserId = UserId(2);
    const ORG: i64 = 10;

    fn thread(owner: UserId, repo: &str, setting: Option<Visibility>, forked_from: Option<ThreadId>) -> ThreadRow {
        ThreadRow { owner, family: 7, setting, forked_from, repo: repo.to_owned() }
    }

    fn facts(threads: Vec<(u128, ThreadRow)>) -> Facts {
        Facts {
            projects: HashMap::from([
                ("org/repo".to_owned(), Project { org: Some(ORG), setting: None }),
                ("org/secret".to_owned(), Project { org: Some(ORG), setting: Some(Visibility::Private) }),
                ("elsewhere/repo".to_owned(), Project { org: Some(99), setting: None }),
            ]),
            defaults: HashMap::from([(ORG, Visibility::Org)]),
            orgs: HashSet::from([ORG]),
            teams: HashSet::from([(ORG, "core".to_owned())]),
            threads: threads.into_iter().map(|(id, row)| (ThreadId(id), row)).collect(),
            ..Facts::default()
        }
    }

    fn sees(facts: Facts, id: u128) -> bool {
        Viewer::decide(ME, facts, 0).sees_thread(ThreadId(id))
    }

    #[test]
    fn org_members_see_threads_their_project_does_not_hide() {
        assert!(sees(facts(vec![(1, thread(OTHER, "org/repo", None, None))]), 1));
        assert!(!sees(facts(vec![(1, thread(OTHER, "org/secret", None, None))]), 1));
        assert!(!sees(facts(vec![(1, thread(OTHER, "elsewhere/repo", None, None))]), 1), "not a member of that org");
    }

    #[test]
    fn owners_always_see_their_threads() {
        assert!(sees(facts(vec![(1, thread(ME, "org/secret", Some(Visibility::Private), None))]), 1));
        assert!(sees(facts(vec![(1, thread(ME, "unknown/repo", None, None))]), 1));
    }

    #[test]
    fn teams_see_only_their_own_threads() {
        let core = thread(OTHER, "org/repo", Some(Visibility::Team("core".to_owned())), None);
        let design = thread(OTHER, "org/repo", Some(Visibility::Team("design".to_owned())), None);
        let both = facts(vec![(1, core), (2, design)]);
        let viewer = Viewer::decide(ME, both, 0);
        assert!(viewer.sees_thread(ThreadId(1)));
        assert!(!viewer.sees_thread(ThreadId(2)));
        assert!(viewer.sees_family(7));
    }

    #[test]
    fn forks_take_their_source_s_setting_unless_they_have_one() {
        let source = thread(OTHER, "org/repo", Some(Visibility::Private), None);
        let fork = thread(OTHER, "org/repo", None, Some(ThreadId(1)));
        let shown = thread(OTHER, "org/repo", Some(Visibility::Org), Some(ThreadId(1)));
        let viewer = Viewer::decide(ME, facts(vec![(1, source), (2, fork), (3, shown)]), 0);
        assert!(!viewer.sees_thread(ThreadId(2)));
        assert!(viewer.sees_thread(ThreadId(3)));
    }

    #[test]
    fn a_fork_cycle_hides_the_thread() {
        let one = thread(OTHER, "org/repo", None, Some(ThreadId(2)));
        let two = thread(OTHER, "org/repo", None, Some(ThreadId(1)));
        assert!(!sees(facts(vec![(1, one), (2, two)]), 1));
    }
}
