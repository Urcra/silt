//! What each person keeps for themselves about the threads they see
//! (ADR 0024). Only they read or write it.

use std::str::FromStr;

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::StatusCode;
use silt_proto::{DesignSystemCopy, MrSeen, SeenMark, ThreadSeen, ThreadState};
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
            version: None,
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

/// The longest path, hash or version a mark may carry, which a person's
/// device sends and the server keeps.
const MARK_FIELD_MAX: usize = 4096;

/// The caller's seen marks on the files of every thread they still see
/// (ADR 0067).
pub(crate) async fn thread_seen(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Vec<ThreadSeen>>, ApiError> {
    let viewer = server.viewer(caller.user).await?;
    let rows = sqlx::query!(
        "SELECT thread_id, path, hash, version, seen, at_ms FROM thread_seen WHERE user_id = ? ORDER BY thread_id",
        caller.user
    )
    .fetch_all(server.db())
    .await?;
    let mut by_thread: BTreeMap<u128, Vec<SeenMark>> = BTreeMap::new();
    for row in rows {
        let Ok(bytes) = <[u8; 16]>::try_from(row.thread_id.as_slice()) else {
            continue;
        };
        by_thread.entry(u128::from_be_bytes(bytes)).or_default().push(SeenMark {
            path: row.path,
            hash: row.hash,
            seen: row.seen != 0,
            at_ms: row.at_ms,
            version: row.version,
        });
    }
    Ok(Json(
        by_thread
            .into_iter()
            .map(|(thread, marks)| ThreadSeen { thread: ThreadId(thread), marks })
            .filter(|seen| viewer.sees_thread(seen.thread))
            .collect(),
    ))
}

/// Keeps a mark on a thread's file unless the one stored is newer.
pub(crate) async fn mark_thread_seen(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(thread): UrlPath<String>,
    Json(mark): Json<SeenMark>,
) -> Result<StatusCode, ApiError> {
    let thread = ThreadId::from_str(&thread).map_err(|_| bad_request("invalid thread id"))?;
    let long = |field: &str| field.len() > MARK_FIELD_MAX;
    if long(&mark.path) || long(&mark.hash) || mark.version.as_deref().is_some_and(long) {
        return Err(bad_request("the mark is too long"));
    }
    if !server.viewer(caller.user).await?.sees_thread(thread) {
        return Err(not_found());
    }
    let (key, seen) = (id_bytes(thread.0), i64::from(mark.seen));
    sqlx::query!(
        "INSERT INTO thread_seen (user_id, thread_id, path, hash, version, seen, at_ms) VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (user_id, thread_id, path) DO UPDATE SET hash = excluded.hash, version = excluded.version,
             seen = excluded.seen, at_ms = excluded.at_ms
         WHERE excluded.at_ms > thread_seen.at_ms OR (excluded.at_ms = thread_seen.at_ms AND excluded.seen > thread_seen.seen)",
        caller.user,
        key,
        mark.path,
        mark.hash,
        mark.version,
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
    let mut tx = crate::write_transaction(server.db()).await?;
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

/// Where the server keeps a person's own design systems, each a file
/// (ADR 0071).
fn design_systems_dir(server: &Server, user: crate::UserId) -> std::path::PathBuf {
    server.shared.data_dir.join("design-systems").join(user.0.to_string())
}

/// The caller's own design systems, removed ones as tombstones, as their
/// devices last sent them.
pub(crate) async fn design_systems(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Vec<DesignSystemCopy>>, ApiError> {
    let dir = design_systems_dir(&server, caller.user);
    let mut out = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return Ok(Json(out));
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.path().extension().is_some_and(|extension| extension == "json")
            && let Ok(bytes) = tokio::fs::read(entry.path()).await
            && let Ok(copy) = serde_json::from_slice::<DesignSystemCopy>(&bytes)
            && copy.valid()
        {
            out.push(copy);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(out))
}

/// Keeps a copy of one of the caller's design systems unless the one kept
/// changed later.
pub(crate) async fn put_design_system(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(name): UrlPath<String>,
    Json(copy): Json<DesignSystemCopy>,
) -> Result<StatusCode, ApiError> {
    if copy.name != name || !copy.valid() {
        return Err(bad_request("that isn't a design system a device can take"));
    }
    let dir = design_systems_dir(&server, caller.user);
    let path = dir.join(format!("{name}.json"));
    if let Ok(bytes) = tokio::fs::read(&path).await
        && let Ok(kept) = serde_json::from_slice::<DesignSystemCopy>(&bytes)
        && kept.at_ms >= copy.at_ms
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    let json = serde_json::to_vec(&copy).map_err(|_| bad_request("the copy can't be stored"))?;
    let stored = async {
        tokio::fs::create_dir_all(&dir).await?;
        let staging = dir.join(format!("{name}.json.tmp"));
        tokio::fs::write(&staging, json).await?;
        tokio::fs::rename(&staging, &path).await
    };
    if let Err(error) = stored.await {
        tracing::error!(%error, "could not keep a design system");
        return Err(crate::ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            silt_proto::ErrorReply::Invalid { message: "internal error".into() },
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// A person's own themes, removed ones as tombstones, as their devices last
/// sent them (ADR 0099).
pub(crate) async fn stored_themes(server: &Server, user: crate::UserId) -> Result<Vec<silt_proto::ThemeCopy>, ApiError> {
    let rows = sqlx::query!("SELECT body FROM themes WHERE user_id = ? ORDER BY name", user).fetch_all(server.db()).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| serde_json::from_slice::<silt_proto::ThemeCopy>(&row.body).ok())
        .filter(silt_proto::ThemeCopy::valid)
        .collect())
}

pub(crate) async fn themes(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Vec<silt_proto::ThemeCopy>>, ApiError> {
    Ok(Json(stored_themes(&server, caller.user).await?))
}

/// Keeps a copy of one of the caller's themes unless the one kept changed
/// later, in one statement, so two devices sending at once can't race.
pub(crate) async fn put_theme(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(name): UrlPath<String>,
    Json(copy): Json<silt_proto::ThemeCopy>,
) -> Result<StatusCode, ApiError> {
    if copy.name != name || !copy.valid() {
        return Err(bad_request("that isn't a theme a device can take"));
    }
    let body = serde_json::to_vec(&copy).map_err(|_| bad_request("the copy can't be stored"))?;
    let deleted = i64::from(copy.deleted);
    sqlx::query!(
        "INSERT INTO themes (user_id, name, at_ms, deleted, body) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (user_id, name) DO UPDATE SET at_ms = excluded.at_ms, deleted = excluded.deleted, body = excluded.body
         WHERE excluded.at_ms > themes.at_ms",
        caller.user,
        name,
        copy.at_ms,
        deleted,
        body,
    )
    .execute(server.db())
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
