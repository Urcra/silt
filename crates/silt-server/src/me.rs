//! What each person keeps for themselves about the threads they see
//! (ADR 0024). Only they read or write it.

use std::str::FromStr;

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::StatusCode;
use silt_proto::{MrSeen, SeenMark, ThreadState};
use silt_types::{DeltaId, ThreadId};

use crate::{ApiError, Caller, Server, bad_request, id_bytes, not_found};

/// Whether the caller sees the merge request: its target, whose readers
/// review it, or its fork.
async fn sees_request(server: &Server, caller: Caller, mr: DeltaId) -> Result<bool, ApiError> {
    let (fork, target) = {
        let index = server.shared.index.lock().await;
        (index.request_fork(mr), index.request_target(mr))
    };
    let viewer = server.viewer(caller.user).await?;
    Ok([fork, target].into_iter().flatten().any(|thread| viewer.sees_thread(thread)))
}

/// The caller's seen marks on every merge request they still see.
pub(crate) async fn seen(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Vec<MrSeen>>, ApiError> {
    let rows = sqlx::query!("SELECT mr_id, path, hash, seen, at_ms FROM seen WHERE user_id = ?", caller.user)
        .fetch_all(server.db())
        .await?;
    let mut by_mr: BTreeMap<String, Vec<SeenMark>> = BTreeMap::new();
    for row in rows {
        by_mr.entry(row.mr_id).or_default().push(SeenMark {
            path: row.path,
            hash: row.hash,
            seen: row.seen != 0,
            at_ms: row.at_ms,
        });
    }
    let mut out = Vec::new();
    for (mr, marks) in by_mr {
        let Ok(mr) = DeltaId::from_str(&mr) else {
            continue;
        };
        if sees_request(&server, caller, mr).await? {
            out.push(MrSeen { mr, marks });
        }
    }
    Ok(Json(out))
}

/// Keeps a seen mark unless the one stored is newer.
pub(crate) async fn mark_seen(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(mr): UrlPath<String>,
    Json(mark): Json<SeenMark>,
) -> Result<StatusCode, ApiError> {
    let id = DeltaId::from_str(&mr).map_err(|_| bad_request("invalid merge request id"))?;
    if !sees_request(&server, caller, id).await? {
        return Err(not_found());
    }
    let (mr, seen) = (id.to_string(), i64::from(mark.seen));
    sqlx::query!(
        "INSERT INTO seen (user_id, mr_id, path, hash, seen, at_ms) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (user_id, mr_id, path) DO UPDATE SET hash = excluded.hash, seen = excluded.seen, at_ms = excluded.at_ms
         WHERE excluded.at_ms > seen.at_ms OR (excluded.at_ms = seen.at_ms AND excluded.seen > seen.seen)",
        caller.user,
        mr,
        mark.path,
        mark.hash,
        seen,
        mark.at_ms,
    )
    .execute(server.db())
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn threads(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Vec<ThreadState>>, ApiError> {
    let viewer = server.viewer(caller.user).await?;
    let rows =
        sqlx::query_scalar!("SELECT state FROM thread_state WHERE user_id = ?", caller.user).fetch_all(server.db()).await?;
    Ok(Json(
        rows.iter()
            .filter_map(|state| serde_json::from_str::<ThreadState>(state).ok())
            .filter(|state| viewer.sees_thread(state.thread))
            .collect(),
    ))
}

/// Merges a change into the caller's state for the thread, and answers
/// with the state as it now stands.
pub(crate) async fn change_thread(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(thread): UrlPath<String>,
    Json(change): Json<ThreadState>,
) -> Result<Json<ThreadState>, ApiError> {
    let thread = ThreadId::from_str(&thread).map_err(|_| bad_request("invalid thread id"))?;
    if change.thread != thread {
        return Err(bad_request("the change is for another thread"));
    }
    if !server.viewer(caller.user).await?.sees_thread(thread) {
        return Err(not_found());
    }
    let key = id_bytes(thread.0);
    let mut tx = server.db().begin().await?;
    let stored = sqlx::query_scalar!("SELECT state FROM thread_state WHERE user_id = ? AND thread_id = ?", caller.user, key)
        .fetch_optional(&mut *tx)
        .await?;
    let mut state =
        stored.and_then(|state| serde_json::from_str::<ThreadState>(&state).ok()).unwrap_or_else(|| ThreadState::new(thread));
    state.merge(&change);
    let json = serde_json::to_string(&state).map_err(|_| bad_request("the change can't be stored"))?;
    sqlx::query!(
        "INSERT INTO thread_state (user_id, thread_id, state) VALUES (?, ?, ?)
         ON CONFLICT (user_id, thread_id) DO UPDATE SET state = excluded.state",
        caller.user,
        key,
        json,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(state))
}
