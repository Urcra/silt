//! What the hosted web app reads (ADR 0026), from the server's own copy of
//! each family and only what the person may read, shaped as a daemon
//! answers it (`silt_proto::view`). Code comments and diffs need a checkout
//! and stay with the daemons.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

use axum::extract::{Extension, Path, Query, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use silt_agents::history::HistoryReader;
use silt_log::{
    CommentTarget, ConvOp, Delta, GraphEvent, MergeRequest, MrStatus, Op, SessionFile, TurnMark, comments_in, merge_requests_in,
};
use silt_proto::view::{
    ArtifactContent, CommentLocation, CommentView, GraphQuery, GraphRow, HistoryEntry, HistoryPage, InboxEntry, Mine, MrDetails,
    MrGuide, MrSummary, Project, ReplyView, SearchResult, ShowLine, StateChange, ThreadSummary, TurnInfo, opens_turn,
};
use silt_proto::{AgentReport, InboxRead, SearchQuery, ThreadState, Visibility};
use silt_types::{DeltaId, DeviceId, FamilyId, ThreadId};

use crate::app::Browser;
use crate::index::Conversation;
use crate::{ApiError, Caller, Server, UserId, Viewer, bad_request, id_from, not_found, now_ms};

pub(crate) fn routes() -> Router<Server> {
    Router::new()
        .route("/app/api/threads", get(threads))
        .route("/app/api/threads/{id}/history", get(history))
        .route("/app/api/threads/{id}/artifacts/{blob}", get(artifact_content))
        .route("/app/api/threads/{id}/turns", get(turns))
        .route("/app/api/threads/{id}/commits", get(commits))
        .route("/app/api/graph", get(graph))
        .route("/app/api/threads/{id}/comments", get(comments))
        .route("/app/api/threads/{id}/state", patch(change_state))
        .route("/app/api/mrs", get(merge_requests))
        .route("/app/api/search", get(search))
        .route("/app/api/inbox", get(inbox))
        .route("/app/api/inbox/read", post(inbox_read))
        .route("/app/api/me/notifications", get(notification_settings).put(change_notification_setting))
        .route("/app/api/orgs", get(orgs))
        .route("/app/api/projects", get(projects))
}

impl Browser {
    /// The person, as the server's own handlers take them; a browser has no
    /// device of theirs.
    fn caller(self) -> Caller {
        Caller { device: DeviceId(0), user: self.user }
    }
}

/// A thread the person sees, with what the listing knows of it.
struct Row {
    id: ThreadId,
    family: FamilyId,
    repo: String,
    created_ms: i64,
    owner: String,
    visibility: Option<Visibility>,
}

/// What the person may read: their threads, and each family's readable deltas.
struct Readable {
    rows: Vec<Row>,
    deltas: HashMap<FamilyId, Arc<Browsed>>,
    collaborators: HashMap<ThreadId, Vec<String>>,
    me: String,
}

/// The deltas of a family that a reader may read, and what is worked out
/// from them. Its room keeps it until the room changes, for every reader
/// who may read the same deltas: each page of the app would otherwise copy
/// and read the whole family again.
pub(crate) struct Browsed {
    log: Vec<Delta>,
    requests: OnceLock<Vec<MergeRequest>>,
    entries: std::sync::Mutex<HashMap<ThreadId, ThreadEntries>>,
}

type ThreadEntries = Arc<Vec<(DeltaId, HistoryEntry)>>;

impl Browsed {
    pub(crate) fn new(log: Vec<Delta>) -> Self {
        Self { log, requests: OnceLock::new(), entries: std::sync::Mutex::default() }
    }

    fn requests(&self) -> &[MergeRequest] {
        self.requests.get_or_init(|| merge_requests_in(&self.log))
    }

    /// `thread`'s own history entries, each with the delta it came from.
    fn entries(&self, thread: ThreadId) -> ThreadEntries {
        let mut kept = self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(kept.entry(thread).or_insert_with(|| Arc::new(entries(&self.log, thread))))
    }
}

impl Readable {
    async fn load(server: &Server, user: UserId) -> Result<Self, ApiError> {
        let viewer = server.viewer(user).await?;
        let mut readable = Self::rows(server, user, &viewer).await?;
        for family in readable.rows.iter().map(|row| row.family).collect::<BTreeSet<_>>() {
            let Some(mut slot) = server.room(family).await else {
                continue;
            };
            readable.deltas.insert(family, slot.browsed(family, &viewer));
        }
        Ok(readable)
    }

    /// The threads the person sees, without their families' deltas, for
    /// what the index answers.
    async fn rows(server: &Server, user: UserId, viewer: &Viewer) -> Result<Self, ApiError> {
        let rows = sqlx::query!(
            r#"SELECT t.id, t.family_id, t.repo, t.created_ms, t.visibility, u.name AS owner
             FROM threads t JOIN users u ON u.id = t.owner_id
             WHERE t.deleted_ms IS NULL ORDER BY t.created_ms"#
        )
        .fetch_all(server.db())
        .await?;
        let rows: Vec<Row> = rows
            .into_iter()
            .map(|row| Row {
                id: ThreadId(id_from(&row.id)),
                family: FamilyId(id_from(&row.family_id)),
                repo: row.repo,
                created_ms: row.created_ms,
                owner: row.owner,
                visibility: row.visibility.and_then(|visibility| visibility.parse().ok()),
            })
            .filter(|row| viewer.sees_thread(row.id))
            .collect();
        let collaborators = crate::collab::collaborators(server.db()).await?;
        let me = sqlx::query_scalar!("SELECT name FROM users WHERE id = ?", user).fetch_one(server.db()).await?;
        Ok(Self { rows, deltas: HashMap::new(), collaborators, me })
    }

    fn row(&self, selector: &str) -> Result<&Row, ApiError> {
        let found: Vec<&Row> = self.rows.iter().filter(|row| row.id.to_string().ends_with(selector)).collect();
        match found.as_slice() {
            [row] => Ok(row),
            [] => Err(not_found()),
            _ => Err(bad_request("that names several threads")),
        }
    }

    fn of(&self, row: &Row) -> &[Delta] {
        self.deltas.get(&row.family).map_or(&[], |browsed| browsed.log.as_slice())
    }

    fn requests(&self) -> Vec<(FamilyId, MergeRequest)> {
        self.deltas
            .iter()
            .flat_map(|(family, browsed)| browsed.requests().iter().map(|request| (*family, request.clone())))
            .collect()
    }
}

/// A thread's own history entries, each with the delta it came from.
pub(crate) fn entries(log: &[Delta], thread: ThreadId) -> Vec<(DeltaId, HistoryEntry)> {
    let mut reader = HistoryReader::default();
    let rewound = silt_log::rewound_in(log, thread);
    let mut out = Vec::new();
    for delta in log.iter().filter(|delta| delta.thread == thread) {
        let shown: Vec<silt_agents::tool::Shown> = match &delta.op {
            Op::Conv(ConvOp::Native { session, file, line }) => {
                let side = match file {
                    SessionFile::Main => None,
                    SessionFile::Subagent(name) => Some(name.as_str()),
                };
                reader.line(&session.agent, &session.id, side, line)
            }
            Op::Conv(ConvOp::Merged { from, title, summary, .. }) => vec![merged_text(*from, title, summary)],
            _ => Vec::new(),
        };
        let time_ms = entry_time(delta);
        for shown in shown {
            let mut entry = history_entry(shown, thread, time_ms);
            entry.index = out.len();
            entry.rewound = rewound.contains(&delta.span.first());
            out.push((delta.span.first(), entry));
        }
    }
    out
}

/// The kind and text a history shows for one entry of a transcript, if any.
/// A history entry of `thread` from what its history shows; its index
/// and whether a revert rewound it are the caller's to set.
/// When a delta's entries happened: as its agent stamped the line, else
/// when it was recorded.
fn entry_time(delta: &silt_log::Delta) -> i64 {
    match &delta.op {
        Op::Conv(ConvOp::Native { line, .. }) => silt_agents::time::line_time(line).unwrap_or(delta.time_ms),
        _ => delta.time_ms,
    }
}

pub(crate) fn history_entry(shown: silt_agents::tool::Shown, thread: ThreadId, time_ms: i64) -> HistoryEntry {
    shown.into_entry(0, thread.to_string(), "own", time_ms, false)
}

/// The entry a history shows for a merge into the thread.
pub(crate) fn merged_text(from: ThreadId, title: &str, summary: &str) -> silt_agents::tool::Shown {
    silt_agents::tool::Shown::plain("merge", format!("merged \"{title}\" from fork {from}: {summary}"))
}

/// The title it was given, or else the first line of its first prompt; a
/// fork with neither says what it's a fork of.
pub(crate) fn title(own: &[&Delta], first_prompt: Option<&str>) -> Option<String> {
    let given = own.iter().rev().find_map(|delta| match &delta.op {
        Op::Conv(ConvOp::Title(title)) => Some(title.clone()),
        _ => None,
    });
    let forked_from = || {
        own.iter().find_map(|delta| match delta.op {
            Op::Fork { from } => Some(format!("Fork of {}", from.short())),
            _ => None,
        })
    };
    given
        .or_else(|| first_prompt.map(|prompt| prompt.lines().next().unwrap_or_default().chars().take(80).collect()))
        .or_else(forked_from)
}

/// The text of the first prompt among `history`'s entries.
pub(crate) fn first_prompt(history: &[(DeltaId, HistoryEntry)]) -> Option<&str> {
    history.iter().find(|(_, entry)| entry.kind == "user" && entry.side.is_none()).map(|(_, entry)| entry.text.as_str())
}

/// The titles of the threads the person sees, as their thread list shows them.
pub(crate) async fn titles(server: &Server, user: UserId) -> Result<HashMap<ThreadId, Option<String>>, ApiError> {
    let readable = Readable::load(server, user).await?;
    let conversations = conversations(server, &readable).await;
    Ok(readable
        .rows
        .iter()
        .map(|row| {
            let log = readable.of(row);
            let own: Vec<&Delta> = log.iter().filter(|delta| delta.thread == row.id).collect();
            (row.id, title(&own, conversations[&row.id].first_prompt.as_deref()))
        })
        .collect())
}

fn summary(
    readable: &Readable,
    row: &Row,
    conversation: &Conversation,
    requests: &[(FamilyId, MergeRequest)],
    states: &HashMap<ThreadId, ThreadState>,
    running: &HashMap<ThreadId, (DeviceId, AgentReport)>,
) -> ThreadSummary {
    let log = readable.of(row);
    let own: Vec<&Delta> = log.iter().filter(|delta| delta.thread == row.id).collect();
    let collaborators = readable.collaborators.get(&row.id).cloned().unwrap_or_default();
    ThreadSummary {
        id: row.id.to_string(),
        repo: row.repo.clone(),
        title: title(&own, conversation.first_prompt.as_deref()),
        agent: agent_of(&own),
        created_ms: row.created_ms,
        messages: conversation.messages,
        edits: own.iter().filter(|delta| matches!(delta.op, Op::Text { .. })).count(),
        location: "remote".into(),
        forked_from: own.iter().find_map(|delta| match delta.op {
            Op::Fork { from } => Some(from.to_string()),
            _ => None,
        }),
        writable: row.owner == readable.me || collaborators.contains(&readable.me),
        owner: row.owner.clone(),
        collaborators,
        agent_state: running.get(&row.id).map(|(_, agent)| agent.state),
        background: running.get(&row.id).map_or(0, |(_, agent)| agent.background),
        last_ms: own.iter().map(|delta| delta.time_ms).max().unwrap_or(row.created_ms),
        merged: requests.iter().any(|(_, request)| request.fork == row.id && request.status == MrStatus::Approved),
        review: row.owner == readable.me
            && requests.iter().any(|(_, request)| request.target == row.id && request.status == MrStatus::Open),
        mine: Mine::of(states.get(&row.id)),
        visibility: row.visibility.clone(),
        server: None,
        local_only: false,
        workspace: None,
        cloud: silt_proto::view::cloud_session(own.iter().copied()),
    }
}

async fn states(server: &Server, browser: Browser) -> Result<HashMap<ThreadId, ThreadState>, ApiError> {
    let Json(states) = crate::me::threads(State(server.clone()), Extension(browser.caller())).await?;
    Ok(states.into_iter().map(|state| (state.thread, state)).collect())
}

async fn threads(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
) -> Result<Json<Vec<ThreadSummary>>, ApiError> {
    let readable = Readable::load(&server, browser.user).await?;
    let requests = readable.requests();
    let states = states(&server, browser).await?;
    let running = server.shared.control.running().await;
    let conversations = conversations(&server, &readable).await;
    Ok(Json(
        readable.rows.iter().map(|row| summary(&readable, row, &conversations[&row.id], &requests, &states, &running)).collect(),
    ))
}

/// What the listing shows of each thread's conversation, as the index keeps it.
async fn conversations(server: &Server, readable: &Readable) -> HashMap<ThreadId, Conversation> {
    let index = server.shared.index.lock().await;
    readable.rows.iter().map(|row| (row.id, index.conversation(row.id))).collect()
}

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

async fn history(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<HistoryPage>, ApiError> {
    let viewer = server.viewer(browser.user).await?;
    let readable = Readable::rows(&server, browser.user, &viewer).await?;
    let row = readable.row(&id)?;
    let index = server.shared.index.lock().await;
    let Some(history) = index.history(row.id) else {
        return Ok(Json(HistoryPage { thread: row.id.to_string(), total: 0, entries: Vec::new() }));
    };
    let total = history.entries.len();
    let offset = page.offset.unwrap_or(0).min(total);
    let limit = page.limit.unwrap_or(total);
    let entries = (offset..)
        .zip(history.entries.iter().skip(offset).take(limit))
        .map(|(index, (delta, entry))| HistoryEntry { index, rewound: history.rewound.contains(delta), ..entry.clone() })
        .collect();
    Ok(Json(HistoryPage { thread: row.id.to_string(), total, entries }))
}

/// An artifact's content, if an artifact of the thread names the blob
/// (ADR 0066).
async fn artifact_content(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, blob)): Path<(String, String)>,
) -> Result<Json<ArtifactContent>, ApiError> {
    use base64::Engine as _;
    let viewer = server.viewer(browser.user).await?;
    let readable = Readable::rows(&server, browser.user, &viewer).await?;
    let row = readable.row(&id)?;
    let media_type = server
        .shared
        .index
        .lock()
        .await
        .history(row.id)
        .and_then(|history| {
            history.entries.iter().find_map(|(_, entry)| entry.artifact.as_ref().filter(|artifact| artifact.blob == blob))
        })
        .map(|artifact| artifact.media_type.clone())
        .ok_or_else(not_found)?;
    let hash: silt_types::BlobHash = blob.parse().map_err(|_| bad_request("invalid blob hash"))?;
    let bytes = server.shared.blobs.get(hash).await.map_err(|_| not_found())?;
    Ok(Json(ArtifactContent { media_type, data: base64::engine::general_purpose::STANDARD.encode(bytes) }))
}

async fn turns(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TurnInfo>>, ApiError> {
    let viewer = server.viewer(browser.user).await?;
    let readable = Readable::rows(&server, browser.user, &viewer).await?;
    let row = readable.row(&id)?;
    let index = server.shared.index.lock().await;
    Ok(Json(index.history(row.id).map(indexed_turns).unwrap_or_default()))
}

/// [`turn_infos`], from what the index keeps of the thread.
fn indexed_turns(history: &crate::index::History) -> Vec<TurnInfo> {
    let entries = &history.entries;
    (1..)
        .zip(&history.prompts)
        .map(|(number, &began)| {
            // The latest prompt said at or before the mark.
            let prompt =
                entries.iter().rposition(|(_, entry)| opens_turn(&entry.kind) && entry.side.is_none() && entry.time_ms <= began);
            TurnInfo { number, entry: prompt, prompt: prompt.map(|at| entries[at].1.text.clone()).unwrap_or_default() }
        })
        .collect()
}

fn turn_infos(log: &[Delta], thread: ThreadId, history: &[(DeltaId, HistoryEntry)]) -> Vec<TurnInfo> {
    let mut turns = Vec::new();
    for delta in log.iter().filter(|delta| delta.thread == thread) {
        if let Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. }) = delta.op {
            // The latest prompt said at or before the mark.
            let prompt = history
                .iter()
                .rev()
                .find(|(_, entry)| opens_turn(&entry.kind) && entry.side.is_none() && entry.time_ms <= delta.time_ms);
            turns.push(TurnInfo {
                number: turns.len() + 1,
                entry: prompt.map(|(_, entry)| entry.index),
                prompt: prompt.map(|(_, entry)| entry.text.clone()).unwrap_or_default(),
            });
        }
    }
    turns
}

/// The command of the agent that recorded the thread's latest session.
fn agent_of(own: &[&Delta]) -> Option<String> {
    own.iter().rev().find_map(|delta| match &delta.op {
        Op::Conv(ConvOp::Native { session, .. }) => {
            Some(silt_agents::agent(&session.agent).map_or_else(|| session.agent.clone(), |agent| agent.command().to_owned()))
        }
        _ => None,
    })
}

/// The history graph over what the person may read (ADR 0029, 0030).
async fn graph(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Query(query): Query<GraphQuery>,
) -> Result<Json<Vec<GraphRow>>, ApiError> {
    let readable = Readable::load(&server, browser.user).await?;
    let states = states(&server, browser).await?;
    let mut rows = Vec::new();
    for (family, browsed) in &readable.deltas {
        if query.family.as_ref().is_some_and(|wanted| *wanted != family.to_string()) {
            continue;
        }
        let log = browsed.log.as_slice();
        let requests = browsed.requests();
        let threads: HashMap<ThreadId, &Row> =
            readable.rows.iter().filter(|row| row.family == *family).map(|row| (row.id, row)).collect();
        let mut turns: HashMap<ThreadId, Vec<TurnInfo>> = HashMap::new();
        let mut agents: HashMap<ThreadId, Option<String>> = HashMap::new();
        for item in silt_log::graph_of(log) {
            let thread = match &item.event {
                GraphEvent::Turn { thread, .. }
                | GraphEvent::Commit { thread, .. }
                | GraphEvent::Fork { thread, .. }
                | GraphEvent::Request { thread, .. }
                | GraphEvent::Merged { thread, .. } => *thread,
            };
            let Some(row) = threads.get(&thread) else {
                continue;
            };
            if query.project.as_ref().is_some_and(|project| *project != row.repo) || (query.mine && row.owner != readable.me) {
                continue;
            }
            let (kind, turn, text, other, status) = match &item.event {
                GraphEvent::Turn { number, .. } => {
                    let turns = turns.entry(thread).or_insert_with(|| turn_infos(log, thread, &browsed.entries(thread)));
                    let prompt = turns
                        .get(number - 1)
                        .map(|turn| turn.prompt.lines().next().unwrap_or_default().chars().take(120).collect());
                    ("turn", Some(*number), prompt.unwrap_or_default(), None, None)
                }
                GraphEvent::Commit { commit, turn, .. } => ("commit", *turn, commit.to_string(), None, None),
                GraphEvent::Fork { from, .. } => {
                    ("fork", None, format!("forked from {}", from.short()), Some(from.to_string()), None)
                }
                GraphEvent::Request { target, mr, title, .. } => {
                    let status = requests.iter().find(|request| request.id == *mr).map(|request| match request.status {
                        MrStatus::Open => "open",
                        MrStatus::Approved => "approved",
                        MrStatus::Declined { .. } => "declined",
                    });
                    ("request", None, title.clone(), Some(target.to_string()), status.map(str::to_owned))
                }
                GraphEvent::Merged { from, title, .. } => ("merge", None, title.clone(), Some(from.to_string()), None),
            };
            if !query.shows(kind) {
                continue;
            }
            let agent = agents
                .entry(thread)
                .or_insert_with(|| agent_of(&log.iter().filter(|delta| delta.thread == thread).collect::<Vec<_>>()))
                .clone();
            let commit = match &item.event {
                GraphEvent::Commit { commit, .. } => Some(commit.to_string()),
                _ => None,
            };
            rows.push(GraphRow {
                commit,
                id: item.delta.to_string(),
                kind: kind.to_owned(),
                thread: thread.to_string(),
                family: family.to_string(),
                time_ms: item.time_ms,
                turn,
                text,
                other,
                status,
                owner: row.owner.clone(),
                agent,
                settled: Mine::of(states.get(&thread)).settled,
            });
        }
    }
    let mut rows = query.page(rows);
    // Only the page's commits are looked up in the mirror.
    for row in rows.iter_mut().filter(|row| row.kind == "commit") {
        let mirror = server.git_root().join(format!("{}.git", row.family));
        row.text = mirror_subject(&mirror, &row.text).await.unwrap_or_else(|| row.text[..8].to_owned());
    }
    Ok(Json(rows))
}

/// The thread's commits with the turns they came in (ADR 0030), and their
/// subjects from the family's git mirror, which has those origin lacked.
async fn commits(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<silt_proto::view::CommitView>>, ApiError> {
    let readable = Readable::load(&server, browser.user).await?;
    let row = readable.row(&id)?;
    let mirror = server.git_root().join(format!("{}.git", row.family));
    let mut views = Vec::new();
    for commit in silt_log::commits_in(readable.of(row), row.id) {
        let id = commit.commit.to_string();
        views.push(silt_proto::view::CommitView {
            subject: mirror_subject(&mirror, &id).await,
            commit: id,
            branch: commit.branch,
            turn: commit.turn,
            time_ms: commit.time_ms,
        });
    }
    Ok(Json(views))
}

async fn mirror_subject(mirror: &std::path::Path, commit: &str) -> Option<String> {
    if !mirror.exists() {
        return None;
    }
    let out = tokio::process::Command::new("git")
        .arg("--git-dir")
        .arg(mirror)
        .args(["log", "-1", "--format=%s", commit, "--"])
        .output()
        .await
        .ok()?;
    let subject = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !subject.is_empty()).then_some(subject)
}

/// How a daemon shortens an id it shows, so links from either work.
pub(crate) fn short(id: DeltaId) -> String {
    let actor = id.actor.to_string();
    format!("{}.{}", &actor[actor.len() - 6..], id.lamport.0)
}

/// A thread the person sees, with its history entries and the comments on
/// it as they may read them, and the person's name.
pub(crate) struct SeenThread {
    pub(crate) thread: ThreadId,
    pub(crate) family: FamilyId,
    pub(crate) entries: Vec<(DeltaId, HistoryEntry)>,
    pub(crate) comments: Vec<silt_log::Comment>,
    pub(crate) me: String,
}

pub(crate) async fn seen_thread(server: &Server, user: UserId, selector: &str) -> Result<SeenThread, ApiError> {
    let readable = Readable::load(server, user).await?;
    let row = readable.row(selector)?;
    let log = readable.of(row);
    Ok(SeenThread {
        thread: row.id,
        family: row.family,
        entries: entries(log, row.id),
        comments: comments_in(log, row.id),
        me: readable.me.clone(),
    })
}

/// A merge request the person sees, by its id or a daemon's short one, and
/// the person's name.
pub(crate) async fn seen_request(
    server: &Server,
    user: UserId,
    selector: &str,
) -> Result<(FamilyId, MergeRequest, String), ApiError> {
    let readable = Readable::load(server, user).await?;
    let (family, request) = readable
        .requests()
        .into_iter()
        .find(|(_, request)| short(request.id) == selector || request.id.to_string() == selector)
        .ok_or_else(not_found)?;
    Ok((family, request, readable.me))
}

pub(crate) async fn comments(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<CommentView>>, ApiError> {
    let readable = Readable::load(&server, browser.user).await?;
    let row = readable.row(&id)?;
    let log = readable.of(row);
    let history = entries(log, row.id);
    Ok(Json(
        comments_in(log, row.id)
            .into_iter()
            .map(|comment| CommentView {
                id: short(comment.id),
                by: comment.by,
                time_ms: comment.time_ms,
                text: comment.text,
                resolved: comment.resolved,
                sent: comment.sent,
                location: match comment.target {
                    CommentTarget::Message { delta } => {
                        history.iter().find(|(from, _)| *from == delta).map_or(CommentLocation::Unknown, |(_, entry)| {
                            CommentLocation::Entry { index: entry.index, excerpt: entry.text.chars().take(120).collect() }
                        })
                    }
                    CommentTarget::Code { .. } => CommentLocation::Unknown,
                },
                replies: comment
                    .replies
                    .into_iter()
                    .map(|reply| ReplyView { id: short(reply.id), by: reply.by, time_ms: reply.time_ms, text: reply.text })
                    .collect(),
            })
            .collect(),
    ))
}

async fn change_state(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(change): Json<StateChange>,
) -> Result<Json<Mine>, ApiError> {
    let thread = Readable::load(&server, browser.user).await?.row(&id)?.id;
    let Json(state) = crate::me::change_thread(
        State(server),
        Extension(browser.caller()),
        Path(thread.to_string()),
        Json(change.stamped(thread, now_ms())),
    )
    .await?;
    Ok(Json(Mine::of(Some(&state))))
}

fn status_text(status: &MrStatus) -> String {
    match status {
        MrStatus::Open => "open".into(),
        MrStatus::Approved => "approved".into(),
        MrStatus::Declined { reason } if reason.is_empty() => "declined".into(),
        MrStatus::Declined { reason } => format!("declined: {reason}"),
    }
}

fn mr_summary(log: &[Delta], request: &MergeRequest) -> MrSummary {
    let open = request.status == MrStatus::Open;
    let asked = request.changes_requested.as_ref().filter(|_| open);
    let moved = log.iter().filter(|delta| delta.thread == request.fork).map(|delta| delta.time_ms).max();
    MrSummary {
        id: short(request.id),
        fork: request.fork.to_string(),
        target: request.target.to_string(),
        title: request.title.clone(),
        summary: request.summary.clone(),
        status: status_text(&request.status),
        opened_ms: request.opened_ms,
        decided_ms: request.decided_ms,
        changes_requested: asked.map(|asked| asked.reason.clone()),
        updated_since: asked.is_some_and(|asked| moved.is_some_and(|moved| moved > asked.at_ms)),
    }
}

async fn merge_requests(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
) -> Result<Json<Vec<MrSummary>>, ApiError> {
    let readable = Readable::load(&server, browser.user).await?;
    let mut requests = readable.requests();
    requests.sort_by_key(|(_, request)| request.opened_ms);
    Ok(Json(requests.iter().map(|(family, request)| mr_summary(&readable.deltas[family].log, request)).collect()))
}

pub(crate) async fn merge_request(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<MrDetails>, ApiError> {
    let readable = Readable::load(&server, browser.user).await?;
    let (family, request) = readable
        .requests()
        .into_iter()
        .find(|(_, request)| short(request.id) == id || request.id.to_string() == id)
        .ok_or_else(not_found)?;
    let browsed = &readable.deltas[&family];
    let conversation = browsed
        .entries(request.fork)
        .iter()
        .map(|(_, entry)| ShowLine::Entry { kind: entry.kind.clone(), text: entry.text.clone() })
        .collect();
    Ok(Json(MrDetails {
        mr: mr_summary(&browsed.log, &request),
        diff: String::new(),
        conflicts: Vec::new(),
        conversation,
        files: Vec::new(),
        guide: request.guide.clone().map(|(text, by)| MrGuide { text, by }),
    }))
}

async fn search(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<SearchResult>>, ApiError> {
    let Json(reply) = crate::search(State(server), Extension(browser.caller()), Query(query)).await?;
    Ok(Json(
        reply
            .hits
            .into_iter()
            .map(|hit| SearchResult {
                thread: hit.thread.to_string(),
                kind: hit.kind,
                snippet: hit.snippet,
                owner: Some(hit.owner),
                status: Some(hit.status),
                repo: Some(hit.repo),
                time_ms: Some(hit.time_ms),
                turn: None,
                before_compaction: false,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct InboxAll {
    #[serde(default)]
    all: bool,
}

async fn inbox(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Query(query): Query<InboxAll>,
) -> Result<Json<Vec<InboxEntry>>, ApiError> {
    let query = crate::inbox::InboxQuery { all: query.all };
    let Json(reply) = crate::inbox::list(State(server), Extension(browser.caller()), Query(query)).await?;
    Ok(Json(
        reply
            .notifications
            .into_iter()
            .map(|note| InboxEntry {
                id: note.id,
                kind: note.kind,
                thread: note.thread.to_string(),
                actor: note.actor,
                text: note.text,
                created_ms: note.created_ms,
                read: note.read,
                action: note.action,
                notify: note.notify,
                server: None,
            })
            .collect(),
    ))
}

async fn inbox_read(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Json(read): Json<InboxRead>,
) -> Result<Json<&'static str>, ApiError> {
    crate::inbox::read(State(server), Extension(browser.caller()), Json(read)).await?;
    Ok(Json("read"))
}

async fn notification_settings(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
) -> Result<Json<Vec<silt_proto::KindSetting>>, ApiError> {
    crate::inbox::settings(State(server), Extension(browser.caller())).await
}

async fn change_notification_setting(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Json(setting): Json<silt_proto::KindSetting>,
) -> Result<Json<Vec<silt_proto::KindSetting>>, ApiError> {
    crate::inbox::change_setting(State(server), Extension(browser.caller()), Json(setting)).await
}

async fn orgs(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
) -> Result<Json<Vec<silt_proto::OrgInfo>>, ApiError> {
    let Json(reply) = crate::orgs::list(State(server), Extension(browser.caller())).await?;
    Ok(Json(reply.orgs))
}

async fn projects(State(server): State<Server>, Extension(browser): Extension<Browser>) -> Result<Json<Vec<Project>>, ApiError> {
    let readable = Readable::load(&server, browser.user).await?;
    let requests = readable.requests();
    let Json(reply) = crate::projects::list(State(server), Extension(browser.caller())).await?;
    Ok(Json(
        reply
            .projects
            .into_iter()
            .map(|info| {
                let open = requests
                    .iter()
                    .filter(|(_, request)| request.status == MrStatus::Open)
                    .filter(|(_, request)| readable.rows.iter().any(|row| row.id == request.target && row.repo == info.repo))
                    .count();
                Project {
                    open_mrs: u32::try_from(open).unwrap_or(u32::MAX),
                    repo: info.repo,
                    name: info.name,
                    org: info.org,
                    origin_url: info.origin_url,
                    visibility: info.visibility,
                    default_agent: info.default_agent,
                    default_preset: info.default_preset,
                    git_notes: info.git_notes,
                    forge_comments: info.forge_comments,
                    threads: info.threads,
                    clone: None,
                    home: None,
                    server: None,
                    workspace: None,
                    workspace_why: None,
                }
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_keeps_a_threads_history_and_turns_as_reading_its_log_finds_them() {
        let thread = ThreadId(1);
        let mut family = silt_log::Family::new();
        let session = silt_log::SessionRef { agent: "claude".into(), id: "s".into() };
        let mut log = Vec::new();
        let mut add = |family: &mut silt_log::Family, time_ms: i64, op: ConvOp| {
            let delta = family.delta(
                silt_types::ActorId(1),
                thread,
                silt_log::Author::User,
                time_ms,
                family.thread_head(thread),
                Op::Conv(op),
            );
            family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
            log.push(delta.clone());
            delta
        };
        let line = |text: &str| ConvOp::Native { session: session.clone(), file: SessionFile::Main, line: text.into() };
        let mark = |mark| ConvOp::Turn { session: session.clone(), mark };
        add(&mut family, 1, mark(TurnMark::PromptSubmitted));
        let first = add(&mut family, 2, line(r#"{"type":"user","message":{"role":"user","content":"first"}}"#));
        add(
            &mut family,
            3,
            line(r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}"#),
        );
        add(&mut family, 4, mark(TurnMark::PromptSubmitted));
        add(&mut family, 5, line(r#"{"type":"user","message":{"role":"user","content":"second"}}"#));
        add(&mut family, 6, ConvOp::Rewound { to: first.span.first() });

        let mut index = crate::index::Index::default();
        for delta in &log {
            index.observe(FamilyId(1), delta);
        }
        let history = index.history(thread).expect("kept");
        let read = entries(&log, thread);
        let kept: Vec<_> = history
            .entries
            .iter()
            .map(|(delta, entry)| {
                (*delta, entry.kind.clone(), entry.text.clone(), entry.time_ms, history.rewound.contains(delta))
            })
            .collect();
        let expected: Vec<_> = read
            .iter()
            .map(|(delta, entry)| (*delta, entry.kind.clone(), entry.text.clone(), entry.time_ms, entry.rewound))
            .collect();
        assert_eq!(kept, expected);
        assert!(expected.iter().any(|(.., rewound)| *rewound), "the revert rewound what came after the first prompt");
        assert_eq!(indexed_turns(history), turn_infos(&log, thread, &read));
    }

    #[test]
    fn ids_are_shortened_as_a_daemon_shortens_them() {
        let id = DeltaId::new(silt_types::ActorId(0x0123_4567_89ab_cdef), silt_types::Lamport(42));
        let actor = id.actor.to_string();
        assert_eq!(short(id), format!("{}.42", &actor[actor.len() - 6..]));
    }
}
