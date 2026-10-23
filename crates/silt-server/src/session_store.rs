//! The server as a session store for Agent SDK apps (ADR 0042). Each
//! session is a private thread of the person whose device token calls,
//! its entries the thread's Claude Code transcript lines.

use axum::extract::{DefaultBodyLimit, Extension, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use silt_log::{Author, ConvOp, Delta, Op, SessionFile, SessionRef, TurnMark};
use silt_proto::{PushRequest, RegisterThread, ThreadInfo, Visibility};
use silt_types::{DeltaSpan, DeviceId, FamilyId, Frontier, ThreadId};

use crate::{ApiError, Caller, DELTAS_BODY, Server, bad_request, id_bytes, id_from, now_ms};

const AGENT: &str = "claude-code";
const SUBAGENTS: &str = "subagents/";

pub(crate) fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/session-store/append", post(append).layer(DefaultBodyLimit::max(DELTAS_BODY)))
        .route("/v1/session-store/load", post(load))
        .route("/v1/session-store/sessions", get(sessions))
        .route("/v1/session-store/subkeys", post(subkeys))
        .route("/v1/session-store/delete", post(delete))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionKey {
    project_key: String,
    session_id: String,
    #[serde(default)]
    subpath: Option<String>,
}

impl SessionKey {
    /// Checks what becomes a session id, a file name and a database key.
    fn checked(&self) -> Result<SessionFile, ApiError> {
        let id_ok = !self.session_id.is_empty()
            && self.session_id.len() <= 100
            && self.session_id.chars().all(|ch| ch.is_ascii_alphanumeric() || "-_".contains(ch));
        if !id_ok {
            return Err(bad_request("a session id is letters, digits, - and _"));
        }
        if self.project_key.is_empty() || self.project_key.len() > 1000 || self.project_key.chars().any(char::is_control) {
            return Err(bad_request("the project key is missing or too long"));
        }
        match self.subpath.as_deref() {
            None => Ok(SessionFile::Main),
            Some(subpath) => {
                let name = subpath.strip_prefix(SUBAGENTS).filter(|name| {
                    !name.is_empty()
                        && name.len() <= 200
                        && !name.starts_with('.')
                        && name.chars().all(|ch| ch.is_ascii_alphanumeric() || "-_.".contains(ch))
                });
                name.map(|name| SessionFile::Subagent(name.to_owned()))
                    .ok_or_else(|| bad_request("only subagent transcripts (subagents/<name>) are kept beside a session"))
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct AppendRequest {
    key: SessionKey,
    entries: Vec<Value>,
    /// The project the session belongs to, as a normalized remote, if any.
    #[serde(default)]
    repo: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KeyRequest {
    key: SessionKey,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubkeysRequest {
    project_key: String,
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct SessionsQuery {
    project_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListedSession {
    session_id: String,
    mtime: i64,
}

struct Found {
    family: FamilyId,
    thread: ThreadId,
}

async fn find(server: &Server, user: crate::UserId, project_key: &str, session_id: &str) -> Result<Option<Found>, ApiError> {
    let row = sqlx::query!(
        "SELECT s.family_id, s.thread_id FROM sdk_sessions s JOIN threads t ON t.id = s.thread_id
         WHERE s.user_id = ? AND s.project_key = ? AND s.session_id = ? AND t.deleted_ms IS NULL",
        user,
        project_key,
        session_id,
    )
    .fetch_optional(server.db())
    .await?;
    Ok(row.map(|row| Found { family: FamilyId(id_from(&row.family_id)), thread: ThreadId(id_from(&row.thread_id)) }))
}

/// The session's thread, made and registered on its first append.
async fn find_or_make(
    server: &Server,
    caller: Caller,
    request: &AppendRequest,
    first: Option<&Value>,
) -> Result<Found, ApiError> {
    let key = &request.key;
    if let Some(found) = find(server, caller.user, &key.project_key, &key.session_id).await? {
        return Ok(found);
    }
    let (family, thread) = (FamilyId(uuid::Uuid::new_v4().as_u128()), ThreadId(uuid::Uuid::new_v4().as_u128()));
    let repo = request.repo.clone().unwrap_or_default();
    let info = ThreadInfo {
        id: thread,
        family,
        repo,
        created_ms: now_ms(),
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };
    let register = RegisterThread { thread: info, visibility: Some(Visibility::Private), origin_url: None, recovery_of: None };
    crate::register_thread(State(server.clone()), Extension(caller), Json(register)).await?;
    let (family_id, thread_id, now) = (id_bytes(family.0), id_bytes(thread.0), now_ms());
    sqlx::query!(
        "INSERT INTO sdk_sessions (user_id, project_key, session_id, family_id, thread_id, mtime_ms) VALUES (?, ?, ?, ?, ?, ?)",
        caller.user,
        key.project_key,
        key.session_id,
        family_id,
        thread_id,
        now,
    )
    .execute(server.db())
    .await?;
    let cwd = first.and_then(|entry| entry.get("cwd")).and_then(Value::as_str).unwrap_or_default().to_owned();
    let started = Op::Conv(ConvOp::SessionStarted { session: session(&key.session_id), cwd: cwd.clone(), checkout_root: cwd });
    write(server, caller, family, thread, vec![started]).await?;
    Ok(Found { family, thread })
}

/// How the tables name a transcript: empty for the main one.
fn file_name(file: &SessionFile) -> &str {
    match file {
        SessionFile::Main => "",
        SessionFile::Subagent(name) => name,
    }
}

/// How many of the transcript's first entries a delete hid.
async fn hidden(server: &Server, thread: ThreadId, file: &SessionFile) -> Result<usize, sqlx::Error> {
    let (thread_id, name) = (id_bytes(thread.0), file_name(file));
    let hidden = sqlx::query_scalar!("SELECT hidden FROM sdk_cleared WHERE thread_id = ? AND file = ?", thread_id, name)
        .fetch_optional(server.db())
        .await?;
    Ok(hidden.map_or(0, |hidden| usize::try_from(hidden).unwrap_or(0)))
}

/// The thread's entries of `file`, in the order they were appended.
fn entries_of(deltas: &[Delta], thread: ThreadId, file: &SessionFile) -> Vec<Value> {
    deltas
        .iter()
        .filter(|delta| delta.thread == thread)
        .filter_map(|delta| match &delta.op {
            Op::Conv(ConvOp::Native { file: of, line, .. }) if of == file => serde_json::from_str(line).ok(),
            _ => None,
        })
        .collect()
}

fn session(id: &str) -> SessionRef {
    SessionRef { agent: AGENT.into(), id: id.to_owned() }
}

/// Whether a main transcript entry is a prompt someone typed, which starts a
/// turn, rather than a tool's result coming back.
fn is_prompt(entry: &Value) -> bool {
    if entry.get("type").and_then(Value::as_str) != Some("user")
        || entry.get("isSidechain").and_then(Value::as_bool) == Some(true)
    {
        return false;
    }
    match entry.get("message").and_then(|message| message.get("content")) {
        Some(Value::String(_)) => true,
        Some(Value::Array(blocks)) => blocks.iter().any(|block| block.get("type").and_then(Value::as_str) == Some("text")),
        _ => false,
    }
}

async fn append(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(request): Json<AppendRequest>,
) -> Result<StatusCode, ApiError> {
    let file = request.key.checked()?;
    let found = find_or_make(&server, caller, &request, request.entries.first()).await?;
    let (thread_id, name) = (id_bytes(found.thread.0), file_name(&file).to_owned());
    let mut ops = Vec::new();
    let mut uuids = Vec::new();
    for entry in &request.entries {
        if let Some(uuid) = entry.get("uuid").and_then(Value::as_str) {
            let known = sqlx::query_scalar!(
                "SELECT 1 FROM sdk_entries WHERE thread_id = ? AND file = ? AND uuid = ?",
                thread_id,
                name,
                uuid
            )
            .fetch_optional(server.db())
            .await?
            .is_some();
            if known || uuids.iter().any(|seen| seen == uuid) {
                continue;
            }
            uuids.push(uuid.to_owned());
        }
        let line = serde_json::to_string(entry).map_err(|_| bad_request("an entry isn't JSON"))?;
        ops.push(Op::Conv(ConvOp::Native { session: session(&request.key.session_id), file: file.clone(), line }));
        if file == SessionFile::Main && is_prompt(entry) {
            ops.push(Op::Conv(ConvOp::Turn { session: session(&request.key.session_id), mark: TurnMark::PromptSubmitted }));
        }
    }
    if ops.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }
    write(&server, caller, found.family, found.thread, ops).await?;
    let mut tx = server.db().begin().await?;
    for uuid in &uuids {
        sqlx::query!("INSERT OR IGNORE INTO sdk_entries (thread_id, file, uuid) VALUES (?, ?, ?)", thread_id, name, uuid)
            .execute(&mut *tx)
            .await?;
    }
    let now = now_ms();
    sqlx::query!("UPDATE sdk_sessions SET mtime_ms = ? WHERE thread_id = ?", now, thread_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Writes `ops` on the thread as the person's own server actor, redacted as
/// a device redacts its pushes, through the same checks as a push.
async fn write(server: &Server, caller: Caller, family: FamilyId, thread: ThreadId, ops: Vec<Op>) -> Result<(), ApiError> {
    let _one = server.shared.writes.lock().await;
    let actor = crate::writes::actor_of(server, caller.user).await?;
    let redactor = silt_redact::Redactor::new(Vec::new());
    let deltas = {
        let slot = server.room_or_new(family).await;
        let mut parents = silt_log::thread_head_in(slot.room.deltas(), slot.room.graph(), thread);
        let mut lamport = slot.room.graph().next_lamport();
        let mut deltas = Vec::with_capacity(ops.len());
        for op in ops {
            let span = DeltaSpan::new(actor, lamport, op.units() as u64);
            lamport = span.end();
            let delta = Delta { span, parents: parents.clone(), thread, author: Author::User, time_ms: now_ms(), op };
            parents = Frontier::single(span.last());
            deltas.push(redactor.redact_delta(&delta).delta);
        }
        deltas
    };
    let as_person = Caller { device: DeviceId(0), user: caller.user };
    let Json(pushed) = crate::push(
        State(server.clone()),
        Extension(as_person),
        axum::extract::Path(family.to_string()),
        Json(PushRequest { deltas }),
    )
    .await?;
    if pushed.accepted == 0 {
        return Err(crate::forbidden("you may not write in this session"));
    }
    Ok(())
}

async fn load(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(KeyRequest { key }): Json<KeyRequest>,
) -> Result<Json<Option<Vec<Value>>>, ApiError> {
    // Nothing is kept under a subpath append refuses.
    let Ok(file) = key.checked() else {
        return Ok(Json(None));
    };
    let Some(found) = find(&server, caller.user, &key.project_key, &key.session_id).await? else {
        return Ok(Json(None));
    };
    let hidden = hidden(&server, found.thread, &file).await?;
    let Some(slot) = server.room(found.family).await else {
        return Ok(Json(None));
    };
    let entries: Vec<Value> = entries_of(slot.room.deltas(), found.thread, &file).into_iter().skip(hidden).collect();
    Ok(Json(Some(entries).filter(|entries| !entries.is_empty() || file == SessionFile::Main)))
}

async fn sessions(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Query(SessionsQuery { project_key }): Query<SessionsQuery>,
) -> Result<Json<Vec<ListedSession>>, ApiError> {
    let rows = sqlx::query!(
        "SELECT s.session_id, s.mtime_ms FROM sdk_sessions s JOIN threads t ON t.id = s.thread_id
         WHERE s.user_id = ? AND s.project_key = ? AND t.deleted_ms IS NULL ORDER BY s.mtime_ms DESC",
        caller.user,
        project_key,
    )
    .fetch_all(server.db())
    .await?;
    Ok(Json(rows.into_iter().map(|row| ListedSession { session_id: row.session_id, mtime: row.mtime_ms }).collect()))
}

async fn subkeys(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(request): Json<SubkeysRequest>,
) -> Result<Json<Vec<String>>, ApiError> {
    let Some(found) = find(&server, caller.user, &request.project_key, &request.session_id).await? else {
        return Ok(Json(Vec::new()));
    };
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    if let Some(slot) = server.room(found.family).await {
        for delta in slot.room.deltas().iter().filter(|delta| delta.thread == found.thread) {
            if let Op::Conv(ConvOp::Native { file: SessionFile::Subagent(name), .. }) = &delta.op {
                *counts.entry(name.clone()).or_default() += 1;
            }
        }
    }
    let mut names = Vec::new();
    for (name, count) in counts {
        if count > hidden(&server, found.thread, &SessionFile::Subagent(name.clone())).await? {
            names.push(format!("{SUBAGENTS}{name}"));
        }
    }
    Ok(Json(names))
}

/// Moves the session's thread to the trash, or hides a subagent transcript's
/// entries so far: a thread's history only grows.
async fn delete(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(KeyRequest { key }): Json<KeyRequest>,
) -> Result<StatusCode, ApiError> {
    let Ok(file) = key.checked() else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let Some(found) = find(&server, caller.user, &key.project_key, &key.session_id).await? else {
        return Ok(StatusCode::NO_CONTENT);
    };
    if let SessionFile::Subagent(name) = &file {
        let count = match server.room(found.family).await {
            Some(slot) => entries_of(slot.room.deltas(), found.thread, &file).len(),
            None => 0,
        };
        let (thread_id, count) = (id_bytes(found.thread.0), i64::try_from(count).unwrap_or(i64::MAX));
        let mut tx = server.db().begin().await?;
        sqlx::query!(
            "INSERT INTO sdk_cleared (thread_id, file, hidden) VALUES (?, ?, ?)
             ON CONFLICT (thread_id, file) DO UPDATE SET hidden = excluded.hidden",
            thread_id,
            name,
            count,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("DELETE FROM sdk_entries WHERE thread_id = ? AND file = ?", thread_id, name).execute(&mut *tx).await?;
        tx.commit().await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    crate::lifecycle::delete(State(server.clone()), Extension(caller), axum::extract::Path(found.thread.to_string())).await?;
    // The thread stays in the trash; a later append starts a new one.
    let thread_id = id_bytes(found.thread.0);
    sqlx::query!("DELETE FROM sdk_sessions WHERE thread_id = ?", thread_id).execute(server.db()).await?;
    Ok(StatusCode::NO_CONTENT)
}
