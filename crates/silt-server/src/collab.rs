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
use sqlx::Row;
use sqlx::sqlite::SqlitePool;

use crate::{ApiError, Caller, Server, bad_request, forbidden, id_bytes, id_from, not_found};

pub(crate) async fn share(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(thread): UrlPath<String>,
    Json(request): Json<ShareRequest>,
) -> Result<StatusCode, ApiError> {
    let thread = ThreadId::from_str(&thread).map_err(|_| bad_request("invalid thread id"))?;
    let db = server.db();
    let owner: i64 = sqlx::query("SELECT owner_id FROM threads WHERE id = ?")
        .bind(id_bytes(thread.0))
        .fetch_optional(db)
        .await?
        .ok_or_else(not_found)?
        .get("owner_id");
    if owner != caller.user {
        return Err(forbidden("only the thread's owner shares it"));
    }
    let user = crate::orgs::named_user(&server, caller.user, &request.user).await?;
    if request.remove {
        sqlx::query("DELETE FROM thread_collaborators WHERE thread_id = ? AND user_id = ?")
            .bind(id_bytes(thread.0))
            .bind(user)
            .execute(db)
            .await?;
    } else {
        if !server.viewer(user).await?.sees_thread(thread) {
            return Err(bad_request(format!(
                "{} can't see this thread; change its visibility first",
                request.user
            )));
        }
        sqlx::query("INSERT OR IGNORE INTO thread_collaborators (thread_id, user_id) VALUES (?, ?)")
            .bind(id_bytes(thread.0))
            .bind(user)
            .execute(db)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Each thread's collaborators by name.
pub(crate) async fn collaborators(db: &SqlitePool) -> Result<HashMap<ThreadId, Vec<String>>, sqlx::Error> {
    let mut found: HashMap<ThreadId, Vec<String>> = HashMap::new();
    for row in sqlx::query(
        "SELECT c.thread_id, u.name FROM thread_collaborators c JOIN users u ON u.id = c.user_id ORDER BY u.name",
    )
    .fetch_all(db)
    .await?
    {
        found.entry(ThreadId(id_from(row.get("thread_id")))).or_default().push(row.get("name"));
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
///   may write it (`check_writes`).
pub(crate) async fn admit(
    server: &Server,
    user: i64,
    family: FamilyId,
    deltas: Vec<&Delta>,
) -> Result<(Vec<Delta>, Vec<ActorId>), ApiError> {
    let db = server.db();
    let mut owners: HashMap<ActorId, Option<i64>> = HashMap::new();
    for actor in deltas.iter().map(|delta| delta.span.actor).collect::<HashSet<_>>() {
        let owner = sqlx::query("SELECT user_id FROM actor_owners WHERE actor = ?")
            .bind(id_bytes(actor.0))
            .fetch_optional(db)
            .await?
            .map(|row| row.get::<i64, _>("user_id"));
        owners.insert(actor, owner);
    }
    let admitted: Vec<&Delta> = deltas
        .into_iter()
        .filter(|delta| owners[&delta.span.actor].is_none_or(|owner| owner == user))
        .collect();
    for thread in admitted.iter().map(|delta| delta.thread).collect::<HashSet<_>>() {
        let registered = sqlx::query("SELECT family_id FROM threads WHERE id = ?")
            .bind(id_bytes(thread.0))
            .fetch_optional(db)
            .await?;
        if registered.is_some_and(|row| id_from(row.get("family_id")) != family.0) {
            return Err(forbidden(format!("thread {thread} is not part of family {family}")));
        }
    }
    if admitted.iter().any(|delta| matches!(delta.op, Op::Comment(_))) {
        let viewer = server.viewer(user).await?;
        if let Some(delta) = admitted.iter().find(|delta| matches!(delta.op, Op::Comment(_)) && !viewer.sees_thread(delta.thread)) {
            return Err(forbidden(format!("thread {} isn't one you can see", delta.thread)));
        }
    }
    check_writes(db, user, &admitted).await?;
    let claims = owners
        .into_iter()
        .filter(|(actor, owner)| owner.is_none() && admitted.iter().any(|delta| delta.span.actor == *actor))
        .map(|(actor, _)| actor)
        .collect();
    Ok((admitted.into_iter().cloned().collect(), claims))
}

/// Records `actors` as written by `user`'s devices.
pub(crate) async fn claim_actors(tx: &mut sqlx::SqliteConnection, user: i64, actors: &[ActorId]) -> Result<(), sqlx::Error> {
    for actor in actors {
        sqlx::query("INSERT INTO actor_owners (actor, user_id) VALUES (?, ?) ON CONFLICT DO NOTHING")
            .bind(id_bytes(actor.0))
            .bind(user)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

/// Fails unless `user` may write every one of `deltas`, which are new to
/// the server. Threads nobody registered yet belong to whoever writes them.
pub(crate) async fn check_writes(db: &SqlitePool, user: i64, deltas: &[&Delta]) -> Result<(), ApiError> {
    let threads: HashSet<ThreadId> = deltas
        .iter()
        .filter(|delta| !matches!(delta.op, Op::Comment(_)))
        .map(|delta| delta.thread)
        .collect();
    for thread in threads {
        let row = sqlx::query(
            "SELECT t.owner_id, EXISTS (SELECT 1 FROM thread_collaborators c WHERE c.thread_id = t.id AND c.user_id = ?) AS shared
             FROM threads t WHERE t.id = ?",
        )
        .bind(user)
        .bind(id_bytes(thread.0))
        .fetch_optional(db)
        .await?;
        let Some(row) = row else {
            continue;
        };
        if row.get::<i64, _>("owner_id") != user && row.get::<i64, _>("shared") == 0 {
            return Err(forbidden(format!(
                "thread {thread} belongs to someone else; fork it, or ask its owner to share it with you"
            )));
        }
    }
    Ok(())
}
