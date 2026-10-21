//! What each person keeps for themselves about the threads they see
//! (ADR 0024). Only they read or write it.

use std::str::FromStr;

use axum::Json;
use axum::extract::{Extension, Path as UrlPath, State};
use silt_proto::ThreadState;
use silt_types::ThreadId;

use crate::{ApiError, Caller, Server, bad_request, id_bytes, not_found};

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
