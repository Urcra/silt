//! Who may write into a thread (ADR 0014): its owner and the collaborators
//! the owner shares it with. Anyone who sees a thread may comment on it.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use axum::Json;
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::StatusCode;
use silt_log::{Delta, Op};
use silt_proto::ShareRequest;
use silt_types::ThreadId;
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
    let user = crate::accounts::user_by_name(db, &request.user)
        .await?
        .ok_or_else(|| bad_request(format!("no user named {}", request.user)))?;
    if request.remove {
        sqlx::query("DELETE FROM thread_collaborators WHERE thread_id = ? AND user_id = ?")
            .bind(id_bytes(thread.0))
            .bind(user.id)
            .execute(db)
            .await?;
    } else {
        if !server.viewer(user.id).await?.sees_thread(thread) {
            return Err(bad_request(format!(
                "{} can't see this thread; change its visibility first",
                request.user
            )));
        }
        sqlx::query("INSERT OR IGNORE INTO thread_collaborators (thread_id, user_id) VALUES (?, ?)")
            .bind(id_bytes(thread.0))
            .bind(user.id)
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
