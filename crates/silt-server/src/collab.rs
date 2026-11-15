//! Who may write into a thread (ADR 0014): its owner and the collaborators
//! the owner shares it with. Anyone who sees a thread may comment on it.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use axum::Json;
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::StatusCode;
use silt_log::{Delta, Op};
use silt_proto::ShareRequest;
use silt_types::{ActorId, FamilyId, ThreadId};
use sqlx::sqlite::SqlitePool;

use crate::visibility::Viewer;
use crate::{ApiError, Caller, Server, UserId, bad_request, forbidden, id_bytes, id_from, not_found};

pub(crate) async fn share(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(thread): UrlPath<String>,
    Json(request): Json<ShareRequest>,
) -> Result<StatusCode, ApiError> {
    let thread = ThreadId::from_str(&thread).map_err(|_| bad_request("invalid thread id"))?;
    let db = server.db();
    let thread_id = id_bytes(thread.0);
    let owner = sqlx::query_scalar!(r#"SELECT owner_id AS "owner_id: UserId" FROM threads WHERE id = ?"#, thread_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(not_found)?;
    if owner != caller.user {
        return Err(forbidden("only the thread's owner shares it"));
    }
    let user = crate::orgs::named_user(&server, caller.user, &request.user).await?;
    if request.remove {
        sqlx::query!("DELETE FROM thread_collaborators WHERE thread_id = ? AND user_id = ?", thread_id, user).execute(db).await?;
    } else {
        if !server.viewer(user).await?.sees_thread(thread) {
            return Err(bad_request(format!("{} can't see this thread; change its visibility first", request.user)));
        }
        sqlx::query!("INSERT OR IGNORE INTO thread_collaborators (thread_id, user_id) VALUES (?, ?)", thread_id, user)
            .execute(db)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Each thread's collaborators by name.
pub(crate) async fn collaborators(db: &SqlitePool) -> Result<HashMap<ThreadId, Vec<String>>, sqlx::Error> {
    let mut found: HashMap<ThreadId, Vec<String>> = HashMap::new();
    for row in
        sqlx::query!("SELECT c.thread_id, u.name FROM thread_collaborators c JOIN users u ON u.id = c.user_id ORDER BY u.name")
            .fetch_all(db)
            .await?
    {
        found.entry(ThreadId(id_from(&row.thread_id))).or_default().push(row.name);
    }
    Ok(found)
}

/// The pushed deltas new to the server that `user` may add (ADR 0016),
/// and the actors to record as theirs.
/// - A delta written as an actor another person's device owns is dropped:
///   a client re-pushing after a server restore legitimately sends others'
///   deltas, but taking an actor's ids would wedge its owner's sync.
/// - Every delta's thread must belong to `family`.
/// - Comments need only that `user` sees the thread; other deltas that they
///   own it or that it is shared with them. Threads nobody registered yet
///   belong to whoever writes them.
pub(crate) async fn admit(
    server: &Server,
    user: UserId,
    family: FamilyId,
    deltas: Vec<&Delta>,
) -> Result<(Vec<Delta>, Vec<ActorId>), ApiError> {
    let mut ledger = Ledger::load(server.db(), user, &deltas).await?;
    let pages: HashSet<(ThreadId, silt_types::ArtifactId)> = deltas
        .iter()
        .filter_map(|delta| match &delta.op {
            Op::Page(op) => Some((delta.thread, op.artifact())),
            _ => None,
        })
        .collect();
    if !pages.is_empty() {
        let name = sqlx::query_scalar!("SELECT name FROM users WHERE id = ?", user).fetch_one(server.db()).await?;
        ledger.me = silt_proto::page::person_id(&name);
        let index = server.shared.index.lock().await;
        for (thread, artifact) in pages {
            // A version recorded in this same push declares them too.
            let declared = pushed_capabilities(&deltas, thread, artifact)
                .or_else(|| index.artifact_capabilities(thread, &artifact.to_string()));
            ledger.pages.insert((thread, artifact), declared);
        }
    }
    let viewer = server.viewer(user).await?;
    let (admitted, claims) = admission(user, family, deltas, &ledger, &viewer)?;
    Ok((admitted.into_iter().cloned().collect(), claims))
}

/// The capabilities the latest version of `artifact` recorded in `deltas`
/// declares, when the push records one.
fn pushed_capabilities(
    deltas: &[&Delta],
    thread: ThreadId,
    artifact: silt_types::ArtifactId,
) -> Option<silt_proto::artifact::Capabilities> {
    let id = artifact.to_string();
    deltas
        .iter()
        .filter(|delta| delta.thread == thread)
        .filter_map(|delta| match &delta.op {
            Op::Conv(silt_log::ConvOp::Native { line, .. }) if line.contains("\"artifact\"") => {
                serde_json::from_str::<serde_json::Value>(line).ok()
            }
            _ => None,
        })
        .filter(|record| record["t"] == "artifact" && record["id"] == id.as_str())
        .max_by_key(|record| record["version"].as_u64().unwrap_or(0))
        .and_then(|record| serde_json::from_value(record.get("capabilities")?.clone()).ok())
}

/// What the server knows of the actors and threads in a push.
#[derive(Default)]
struct Ledger {
    /// The user whose devices own each actor, when one does.
    actors: HashMap<ActorId, Option<UserId>>,
    threads: HashMap<ThreadId, Registered>,
    /// What each artifact a push writes page data of declares (ADR 0072).
    pages: HashMap<(ThreadId, silt_types::ArtifactId), Option<silt_proto::artifact::Capabilities>>,
    /// The pushing person's id in page data.
    me: String,
}

struct Registered {
    family: u128,
    owner: UserId,
    /// Whether its owner shares it with the pushing user.
    shared: bool,
}

impl Ledger {
    async fn load(db: &SqlitePool, user: UserId, deltas: &[&Delta]) -> Result<Self, sqlx::Error> {
        let mut ledger = Self::default();
        for actor in deltas.iter().map(|delta| delta.span.actor).collect::<HashSet<_>>() {
            let actor_id = id_bytes(actor.0);
            let owner = sqlx::query_scalar!(r#"SELECT user_id AS "user_id: UserId" FROM actor_owners WHERE actor = ?"#, actor_id)
                .fetch_optional(db)
                .await?;
            ledger.actors.insert(actor, owner);
        }
        for thread in deltas.iter().map(|delta| delta.thread).collect::<HashSet<_>>() {
            let thread_id = id_bytes(thread.0);
            let row = sqlx::query!(
                r#"SELECT t.family_id, t.owner_id AS "owner_id: UserId",
                        EXISTS (SELECT 1 FROM thread_collaborators c WHERE c.thread_id = t.id AND c.user_id = ?) AS "shared!: bool"
                 FROM threads t WHERE t.id = ?"#,
                user,
                thread_id,
            )
            .fetch_optional(db)
            .await?;
            if let Some(row) = row {
                let registered = Registered { family: id_from(&row.family_id), owner: row.owner_id, shared: row.shared };
                ledger.threads.insert(thread, registered);
            }
        }
        Ok(ledger)
    }
}

/// The deltas of a push that `user` may add, and the actors to record as
/// theirs.
fn admission<'d>(
    user: UserId,
    family: FamilyId,
    deltas: Vec<&'d Delta>,
    ledger: &Ledger,
    viewer: &Viewer,
) -> Result<(Vec<&'d Delta>, Vec<ActorId>), ApiError> {
    let theirs = |actor: &ActorId| ledger.actors.get(actor).copied().flatten().is_none_or(|owner| owner == user);
    let admitted: Vec<&Delta> = deltas.into_iter().filter(|delta| theirs(&delta.span.actor)).collect();
    for delta in &admitted {
        if ledger.threads.get(&delta.thread).is_some_and(|registered| registered.family != family.0) {
            return Err(forbidden(format!("thread {} is not part of family {family}", delta.thread)));
        }
    }
    if let Some(delta) =
        admitted.iter().find(|delta| matches!(delta.op, Op::Comment(_) | Op::Page(_)) && !viewer.sees_thread(delta.thread))
    {
        return Err(forbidden(format!("thread {} isn't one you can see", delta.thread)));
    }
    for delta in &admitted {
        if let Op::Page(op) = &delta.op {
            page_allowed(user, delta.thread, op, ledger, viewer)?;
        }
    }
    for delta in admitted.iter().filter(|delta| !matches!(delta.op, Op::Comment(_) | Op::Page(_))) {
        let Some(registered) = ledger.threads.get(&delta.thread) else {
            continue;
        };
        // A collaborator the thread was hidden from since sharing writes no
        // more than anyone else who can't see it.
        let collaborator = registered.shared && viewer.sees_thread(delta.thread);
        if registered.owner != user && !collaborator {
            return Err(forbidden(format!(
                "thread {} belongs to someone else; fork it, or ask its owner to share it with you",
                delta.thread
            )));
        }
    }
    let mut claims: Vec<ActorId> = admitted
        .iter()
        .map(|delta| delta.span.actor)
        .filter(|actor| ledger.actors.get(actor).copied().flatten().is_none())
        .collect();
    claims.sort();
    claims.dedup();
    Ok((admitted, claims))
}

/// Whether `user` may write `op` in `thread`, under the rules its
/// artifact's latest version declares (ADR 0072): anyone who sees the
/// thread comments, its owner and collaborators more.
fn page_allowed(user: UserId, thread: ThreadId, op: &silt_log::PageOp, ledger: &Ledger, viewer: &Viewer) -> Result<(), ApiError> {
    use silt_log::PageOp;
    use silt_proto::page::{Level, Rules};
    let level = match ledger.threads.get(&thread) {
        None => Level::Owner,
        Some(registered) if registered.owner == user => Level::Owner,
        Some(registered) if registered.shared && viewer.sees_thread(thread) => Level::Collaborator,
        Some(_) => Level::Comment,
    };
    let artifact = op.artifact();
    let refused = |what: &str| forbidden(format!("you may not {what} artifact {artifact}'s page data in thread {thread}"));
    let declared = ledger.pages.get(&(thread, artifact)).cloned().flatten();
    let Some(declared) = declared else {
        return Err(refused("write"));
    };
    match op {
        PageOp::Deleted { path, .. } if path.starts_with("_assets/") => {
            if !declared.contains_key("assets") || level < Level::Collaborator {
                return Err(refused("delete the files of"));
            }
        }
        PageOp::Set { path, .. } | PageOp::Patched { path, .. } | PageOp::Deleted { path, .. } => {
            let rules = Rules::of(declared.get("db")).map_err(|_| refused("write"))?;
            if !declared.contains_key("db") || !rules.may_write(path, &ledger.me, level) {
                return Err(refused(&format!("write {path} of")));
            }
        }
        PageOp::Asset { .. } => {
            let config = declared.get("assets").ok_or_else(|| refused("add assets to"))?;
            let needs = match config.get("upload").and_then(serde_json::Value::as_str) {
                Some("comment") => Level::Comment,
                _ => Level::Collaborator,
            };
            if level < needs {
                return Err(refused("add assets to"));
            }
        }
        PageOp::Text { .. } | PageOp::Lines { .. } | PageOp::Cell { .. } => {
            let needs = match declared.get("edit").and_then(|edit| edit.get("level")).and_then(serde_json::Value::as_str) {
                Some("collaborator") => Level::Collaborator,
                _ => Level::Comment,
            };
            if level < needs {
                return Err(refused("edit"));
            }
        }
    }
    Ok(())
}

/// Records `actors` as written by `user`'s devices.
pub(crate) async fn claim_actors(tx: &mut sqlx::SqliteConnection, user: UserId, actors: &[ActorId]) -> Result<(), sqlx::Error> {
    for actor in actors {
        let actor_id = id_bytes(actor.0);
        sqlx::query!("INSERT INTO actor_owners (actor, user_id) VALUES (?, ?) ON CONFLICT DO NOTHING", actor_id, user)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use silt_log::{Author, CommentOp, CommentTarget};
    use silt_types::{DeltaId, DeltaSpan, Frontier, Lamport};

    use super::*;

    const ME: UserId = UserId(1);
    const OTHER: UserId = UserId(2);
    const FAMILY: FamilyId = FamilyId(7);

    fn delta(actor: u128, thread: u128, op: Op) -> Delta {
        let span = DeltaSpan::new(ActorId(actor), Lamport(0), 1);
        Delta { span, parents: Frontier::default(), thread: ThreadId(thread), author: Author::User, time_ms: 0, op }
    }

    fn write(actor: u128, thread: u128) -> Delta {
        delta(actor, thread, Op::Fork { from: ThreadId(99) })
    }

    fn comment(actor: u128, thread: u128) -> Delta {
        let target = CommentTarget::Message { delta: DeltaId::new(ActorId(actor), Lamport(0)) };
        delta(actor, thread, Op::Comment(CommentOp::Added { target, text: "nice".to_owned(), by: "me".to_owned() }))
    }

    fn ledger(threads: Vec<(u128, u128, UserId, bool)>) -> Ledger {
        Ledger {
            actors: HashMap::from([(ActorId(1), Some(ME)), (ActorId(2), Some(OTHER)), (ActorId(3), None)]),
            threads: threads
                .into_iter()
                .map(|(thread, family, owner, shared)| (ThreadId(thread), Registered { family, owner, shared }))
                .collect(),
            ..Ledger::default()
        }
    }

    #[test]
    fn deltas_of_other_people_s_actors_are_dropped_and_new_actors_claimed() {
        let deltas = [write(1, 10), write(2, 10), write(3, 10)];
        let Ok((admitted, claims)) = admission(ME, FAMILY, deltas.iter().collect(), &ledger(vec![]), &Viewer::default()) else {
            panic!("refused");
        };
        assert_eq!(admitted.iter().map(|delta| delta.span.actor).collect::<Vec<_>>(), [ActorId(1), ActorId(3)]);
        assert_eq!(claims, [ActorId(3)]);
    }

    #[test]
    fn only_owners_and_collaborators_who_still_see_it_write_a_thread() {
        let threads = ledger(vec![(10, 7, ME, false), (11, 7, OTHER, false), (12, 7, OTHER, true)]);
        let sees_shared = Viewer::seeing([ThreadId(12)]);
        let admit = |thread, viewer: &Viewer| admission(ME, FAMILY, vec![&write(1, thread)], &threads, viewer).is_ok();
        assert!(admit(10, &Viewer::default()));
        assert!(!admit(11, &Viewer::seeing([ThreadId(11)])));
        assert!(admit(12, &sees_shared));
        assert!(!admit(12, &Viewer::default()), "hidden since it was shared");
        assert!(admit(13, &Viewer::default()), "unregistered threads belong to their writer");
    }

    #[test]
    fn comments_need_only_that_the_thread_is_seen() {
        let threads = ledger(vec![(11, 7, OTHER, false)]);
        let admit = |viewer: &Viewer| admission(ME, FAMILY, vec![&comment(1, 11)], &threads, viewer).is_ok();
        assert!(admit(&Viewer::seeing([ThreadId(11)])));
        assert!(!admit(&Viewer::default()));
    }

    #[test]
    fn threads_of_another_family_are_refused() {
        let threads = ledger(vec![(10, 8, ME, false)]);
        assert!(admission(ME, FAMILY, vec![&write(1, 10)], &threads, &Viewer::default()).is_err());
    }
}
