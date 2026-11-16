//! What the hosted web app reads (ADR 0026), from the server's own copy of
//! each family and only what the person may read, shaped as a daemon
//! answers it (`silt_proto::view`). Code comments and diffs need a checkout
//! and stay with the daemons.

use std::collections::{BTreeSet, HashMap, HashSet};
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
    ArtifactContent, ArtifactView, CommentLocation, CommentView, GraphQuery, GraphRow, HistoryEntry, HistoryPage, InboxEntry,
    Mine, MrDetails, MrGuide, MrSummary, Project, ReplyView, SearchResult, ShowLine, StateChange, ThreadSummary, TurnInfo,
    opens_turn,
};
use silt_proto::{AgentReport, InboxRead, SearchQuery, ThreadState, Visibility};
use silt_types::{DeltaId, DeviceId, FamilyId, ThreadId};

use crate::app::Browser;
use crate::index::Conversation;
use crate::{ApiError, Caller, Server, UserId, Viewer, bad_request, id_from, not_found, now_ms};

/// The most a page's save or added file may send: the largest asset, as base64.
const PAGE_FILE_BODY: usize = 28 << 20;

pub(crate) fn routes() -> Router<Server> {
    Router::new()
        .route("/app/api/threads", get(threads))
        .route("/app/api/threads/{id}/history", get(history))
        .route("/app/api/threads/{id}/artifacts/{blob}", get(artifact_content))
        .route("/app/api/threads/{id}/artifacts/{artifact}/frame", post(make_frame))
        .route("/app/api/threads/{id}/artifacts/{artifact}/grants", get(artifact_grants).put(set_artifact_grant))
        .route("/app/api/threads/{id}/artifacts/{artifact}/data", post(page_data))
        .route(
            "/app/api/threads/{id}/artifacts/{artifact}/publish",
            post(publish_artifact).layer(axum::extract::DefaultBodyLimit::max(PAGE_FILE_BODY)),
        )
        .route(
            "/app/api/threads/{id}/artifacts/{artifact}/assets",
            post(page_assets).layer(axum::extract::DefaultBodyLimit::max(PAGE_FILE_BODY)),
        )
        .route("/app/api/threads/{id}/artifacts/{artifact}/versions", post(save_edited_version))
        .route("/app/api/threads/{id}/artifacts/{artifact}/assets/{asset}", get(page_asset_content))
        .route("/app/api/threads/{id}/viewer", get(page_viewer))
        .route("/app/api/artifacts", get(artifact_listing))
        .route("/app/api/threads/{id}/artifacts/{artifact}/thumbnail", get(artifact_thumbnail))
        .route("/app/api/threads/{id}/artifacts/{artifact}/doc", post(doc_data))
        .route("/app/api/threads/{id}/artifacts/{artifact}/sheet", post(sheet_data))
        .route("/app/api/threads/{id}/artifacts/{artifact}/sheet.xlsx", get(sheet_xlsx))
        .route("/app/api/threads/{id}/artifacts/{artifact}/pin", axum::routing::put(pin_artifact))
        .route("/app/api/threads/{id}/artifacts/{artifact}/archive", axum::routing::put(archive_artifact))
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
    graph: OnceLock<silt_crdt::CausalGraph>,
    /// Docs' and sheets' text for search, worked out once per change.
    texts: std::sync::Mutex<HashMap<(ThreadId, silt_types::ArtifactId), Arc<str>>>,
}

type ThreadEntries = Arc<Vec<(DeltaId, HistoryEntry)>>;

impl Browsed {
    pub(crate) fn new(log: Vec<Delta>) -> Self {
        Self {
            log,
            requests: OnceLock::new(),
            entries: std::sync::Mutex::default(),
            graph: OnceLock::new(),
            texts: std::sync::Mutex::default(),
        }
    }

    fn requests(&self) -> &[MergeRequest] {
        self.requests.get_or_init(|| merge_requests_in(&self.log))
    }

    fn graph(&self) -> &silt_crdt::CausalGraph {
        self.graph.get_or_init(|| {
            let mut graph = silt_crdt::CausalGraph::new();
            for delta in &self.log {
                let _ = graph.add(delta.span, &delta.parents);
            }
            graph
        })
    }

    /// What a doc or a sheet says now, for search.
    fn edited_text(&self, thread: ThreadId, artifact: silt_types::ArtifactId, kind: &str) -> Arc<str> {
        if let Some(text) = self.texts.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&(thread, artifact)) {
            return Arc::clone(text);
        }
        let text: Arc<str> = match kind {
            "doc" => silt_log::doc_text_in(&self.log, self.graph(), thread, artifact).into(),
            _ => (0..silt_log::sheet_count_in(&self.log, self.graph(), thread, artifact))
                .map(|sheet| {
                    silt_proto::edited::sheet_text(&silt_log::sheet_cells_in(&self.log, self.graph(), thread, artifact, sheet))
                })
                .collect::<Vec<_>>()
                .join("\n")
                .into(),
        };
        self.texts.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert((thread, artifact), Arc::clone(&text));
        text
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

/// A thread's history entries, each with the delta it came from: its own,
/// and for a fork or a thread that pulled, what it inherited and what
/// pulls brought in, as the daemon tells them.
pub(crate) fn entries(log: &[Delta], thread: ThreadId) -> Vec<(DeltaId, HistoryEntry)> {
    let items: Vec<(&Delta, &'static str, bool)> = if silt_log::has_lineage_in(log, thread) {
        let mut graph = silt_crdt::CausalGraph::new();
        for delta in log {
            // A delta whose parents this log lacks can't be in a lineage it tells.
            let _ = graph.add(delta.span, &delta.parents);
        }
        silt_log::history_in(log, &graph, thread)
            .into_iter()
            .map(|item| {
                let origin = match item.source {
                    silt_log::HistorySource::Own => "own",
                    silt_log::HistorySource::Inherited => "inherited",
                    silt_log::HistorySource::Upstream => "upstream",
                };
                (item.delta, origin, item.rewound)
            })
            .collect()
    } else {
        let rewound = silt_log::rewound_in(log, thread);
        log.iter()
            .filter(|delta| delta.thread == thread)
            .map(|delta| (delta, "own", rewound.contains(&delta.span.first())))
            .collect()
    };
    let mut reader = HistoryReader::default();
    let mut out = Vec::new();
    for (delta, origin, rewound) in items {
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
            let index = out.len();
            out.push((delta.span.first(), shown.into_entry(index, delta.thread.to_string(), origin, time_ms, rewound)));
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
    // A fork's inherited prompts are its source's, not its own.
    history
        .iter()
        .find(|(_, entry)| entry.kind == "user" && entry.side.is_none() && entry.origin == "own")
        .map(|(_, entry)| entry.text.as_str())
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
    let total_and_entries = |all: &[(DeltaId, HistoryEntry)], rewound: Option<&HashSet<DeltaId>>| {
        let total = all.len();
        let offset = page.offset.unwrap_or(0).min(total);
        let limit = page.limit.unwrap_or(total);
        let entries = (offset..)
            .zip(all.iter().skip(offset).take(limit))
            .map(|(index, (delta, entry))| HistoryEntry {
                index,
                rewound: rewound.map_or(entry.rewound, |rewound| rewound.contains(delta)),
                ..entry.clone()
            })
            .collect();
        (total, entries)
    };
    if let Some(entries) = lineage_entries(&server, &viewer, row).await {
        let (total, entries) = total_and_entries(&entries, None);
        return Ok(Json(HistoryPage { thread: row.id.to_string(), total, entries }));
    }
    let index = server.shared.index.lock().await;
    let Some(history) = index.history(row.id) else {
        return Ok(Json(HistoryPage { thread: row.id.to_string(), total: 0, entries: Vec::new() }));
    };
    let (total, entries) = total_and_entries(&history.entries, Some(&history.rewound));
    Ok(Json(HistoryPage { thread: row.id.to_string(), total, entries }))
}

/// A fork's or a thread that pulled's whole history, worked out from what
/// the person may read of its family; `None` for a thread whose own deltas
/// are its history, which the index pages through faster.
async fn lineage_entries(server: &Server, viewer: &Viewer, row: &Row) -> Option<ThreadEntries> {
    if !server.shared.index.lock().await.has_lineage(row.id) {
        return None;
    }
    let mut slot = server.room(row.family).await?;
    Some(slot.browsed(row.family, viewer).entries(row.id))
}

/// An artifact's content, or the CSS its design system compiled to, if an
/// artifact of the thread names the blob (ADR 0068, ADR 0071).
async fn artifact_content(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, blob)): Path<(String, String)>,
) -> Result<Json<ArtifactContent>, ApiError> {
    use base64::Engine as _;
    let viewer = server.viewer(browser.user).await?;
    let readable = Readable::rows(&server, browser.user, &viewer).await?;
    let row = readable.row(&id)?;
    let named = |entries: &[(DeltaId, HistoryEntry)]| {
        entries.iter().filter_map(|(_, entry)| entry.artifact.as_ref()).find_map(|artifact| artifact.media_type_of(&blob))
    };
    let media_type = match lineage_entries(&server, &viewer, row).await {
        Some(entries) => named(&entries),
        None => server.shared.index.lock().await.history(row.id).and_then(|history| named(&history.entries)),
    }
    .ok_or_else(not_found)?;
    let hash: silt_types::BlobHash = blob.parse().map_err(|_| bad_request("invalid blob hash"))?;
    let bytes = server.shared.blobs.get(hash).await.map_err(|_| not_found())?;
    Ok(Json(ArtifactContent { media_type, data: base64::engine::general_purpose::STANDARD.encode(bytes) }))
}

/// A read or write of an artifact's page data, as its page asks through the
/// bridge (ADR 0072): read along the thread's lineage, written as the
/// person, under the rules the latest version declares.
async fn page_data(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(request): Json<silt_proto::page::PageRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use silt_proto::page::Rules;
    let artifact_id: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
    let page = page_of(&server, browser.user, &id, artifact_id).await?;
    let config = page.declared.get("db").ok_or_else(|| bad_request("this artifact doesn't declare db"))?;
    let rules = Rules::of(Some(config)).map_err(bad_request)?;
    if let Some(read) = silt_proto::page::read(&page.docs, &request, &rules, &page.me, page.level) {
        return Ok(Json(read));
    }
    // The writes are checked against the page as it stands under the write
    // lock, so another write can't land between checking and writing.
    let one = server.shared.writes.lock().await;
    let page = page_of(&server, browser.user, &id, artifact_id).await?;
    let writes = request.writes(&page.docs).map_err(bad_request)?;
    let ops =
        silt_proto::page::page_ops(artifact_id, &page.docs, writes, &rules, &page.me, page.level).map_err(crate::forbidden)?;
    let written = ops.len();
    let ops = ops.into_iter().map(Op::Page).collect();
    crate::writes::push_all(&server, &one, browser, page.family, page.thread, None, ops).await?;
    Ok(Json(serde_json::json!({ "written": written })))
}

/// An artifact's page as a person sees it: what its latest version
/// declares, its documents along the thread's lineage, and the person's id
/// and level there (ADR 0072).
pub(crate) struct PageOf {
    pub(crate) family: silt_types::FamilyId,
    pub(crate) thread: silt_types::ThreadId,
    pub(crate) declared: silt_proto::artifact::Capabilities,
    pub(crate) docs: silt_log::page::Documents,
    pub(crate) me: String,
    pub(crate) level: silt_proto::page::Level,
}

pub(crate) async fn page_of(
    server: &Server,
    user: UserId,
    id: &str,
    artifact: silt_types::ArtifactId,
) -> Result<PageOf, ApiError> {
    use silt_proto::page::Level;
    let viewer = server.viewer(user).await?;
    let readable = Readable::rows(server, user, &viewer).await?;
    let row = readable.row(id)?;
    let declared = readable_artifact(server, user, id, &artifact.to_string(), None).await?.capabilities.unwrap_or_default();
    let level = if row.owner == readable.me {
        Level::Owner
    } else if readable.collaborators.get(&row.id).is_some_and(|names| names.contains(&readable.me)) {
        Level::Collaborator
    } else {
        Level::Comment
    };
    let docs = {
        let mut slot = server.room(row.family).await.ok_or_else(not_found)?;
        let browsed = slot.browsed(row.family, &viewer);
        silt_log::page_documents_in(&browsed.log, browsed.graph(), row.id, artifact)
    };
    Ok(PageOf { family: row.family, thread: row.id, declared, docs, me: silt_proto::page::person_id(&readable.me), level })
}

/// Adds, lists or deletes the files people added to a page (ADR 0074).
async fn page_assets(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(asked): Json<silt_proto::page::AssetRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use base64::Engine as _;
    use silt_proto::page::{AssetRequest, AssetView, Level};
    let artifact: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
    let page = page_of(&server, browser.user, &id, artifact).await?;
    let config = page.declared.get("assets").ok_or_else(|| bad_request("this artifact doesn't declare assets"))?;
    match asked {
        AssetRequest::List => Ok(Json(serde_json::to_value(silt_proto::page::assets(&page.docs)).unwrap_or_default())),
        AssetRequest::Add { name, media_type, data } => {
            if page.level < silt_proto::page::asset_level(config) {
                return Err(crate::forbidden("you may not add files to this page"));
            }
            if !silt_proto::artifact::asset_media_type_ok(&media_type) {
                return Err(bad_request(format!("a page's files may not be {media_type}")));
            }
            let name: String = name.chars().filter(|char| !char.is_control()).take(200).collect();
            let mut bytes = base64::engine::general_purpose::STANDARD
                .decode(data.trim())
                .map_err(|_| bad_request("the file's base64 doesn't decode"))?;
            // An SVG is kept only as a picture, whatever it held.
            if media_type.starts_with("image/svg+xml") {
                bytes = silt_proto::svg::sanitized_svg(&bytes).map_err(bad_request)?;
            }
            if bytes.len() as u64 > silt_proto::artifact::ASSET_BYTES {
                return Err(bad_request("the file is larger than a page keeps"));
            }
            let limit = server.config().artifact_assets_mib.saturating_mul(1 << 20);
            let taken: u64 = silt_proto::page::assets(&page.docs).iter().map(|kept| kept.size).sum();
            if taken.saturating_add(bytes.len() as u64) > limit {
                return Err(bad_request(format!("this page's files would take more than this server's {} MiB", limit >> 20)));
            }
            let blob = server.shared.blobs.put(&bytes).await.map_err(|_| bad_request("the file couldn't be kept"))?;
            let asset = format!("{:032x}", uuid::Uuid::new_v4().as_u128())[..12].to_owned();
            let size = bytes.len() as u64;
            let op = silt_log::PageOp::Asset {
                artifact,
                asset: asset.clone(),
                blob,
                name: name.clone(),
                media_type: media_type.clone(),
                size,
            };
            crate::writes::write_as_with(&server, browser, page.family, page.thread, None, Op::Page(op)).await?;
            Ok(Json(
                serde_json::to_value(AssetView { url: format!("_assets/{asset}"), id: asset, name, media_type, size })
                    .unwrap_or_default(),
            ))
        }
        AssetRequest::Delete { id: asset } => {
            if page.level < Level::Collaborator {
                return Err(crate::forbidden("only the thread's owner and collaborators delete a page's files"));
            }
            let path = format!("_assets/{asset}");
            if !silt_proto::page::asset_id_ok(&asset) || !page.docs.contains_key(&path) {
                return Err(not_found());
            }
            let op = silt_log::PageOp::Deleted { artifact, path };
            crate::writes::write_as_with(&server, browser, page.family, page.thread, None, Op::Page(op)).await?;
            Ok(Json(serde_json::json!({ "deleted": asset })))
        }
    }
}

#[derive(Deserialize)]
struct Publish {
    html: String,
    #[serde(default)]
    base: Option<u32>,
}

/// A version an HTML artifact's page saves itself as, for the person
/// (ADR 0072): only its owner and collaborators save one, and only over the
/// version the page was opened at.
async fn publish_artifact(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(publish): Json<Publish>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let viewer = server.viewer(browser.user).await?;
    let readable = Readable::rows(&server, browser.user, &viewer).await?;
    let row = readable.row(&id)?;
    let latest = readable_artifact(&server, browser.user, &id, &artifact, None).await?;
    if latest.kind != "html" || !latest.capabilities.as_ref().is_some_and(|declared| declared.contains_key("artifact")) {
        return Err(bad_request("only an HTML artifact that declares artifact saves itself"));
    }
    if publish.base.is_some_and(|base| base != latest.version) {
        return Err(crate::conflict(format!("conflict: version {} is the latest", latest.version)));
    }
    let collaborator = readable.collaborators.get(&row.id).is_some_and(|names| names.contains(&readable.me));
    if row.owner != readable.me && !collaborator {
        return Err(crate::forbidden("only the thread's owner and collaborators save its artifacts' pages"));
    }
    let bytes = publish.html.into_bytes();
    if bytes.len() as u64 > silt_proto::artifact::ARTIFACT_BYTES {
        return Err(bad_request("the page is larger than an artifact keeps"));
    }
    let note = format!("Saved by the page, for {}", readable.me);
    let version =
        write_version(&server, browser, &viewer, row, latest, "text/html", "index.html", bytes, note, readable.me.clone())
            .await?;
    Ok(Json(serde_json::json!({ "id": artifact, "version": version })))
}

/// Writes `bytes` as artifact `latest`'s next version, recorded in the
/// session that made it, as the person; answers the new version's number.
#[expect(clippy::too_many_arguments, reason = "the parts of one version")]
async fn write_version(
    server: &Server,
    browser: Browser,
    viewer: &Viewer,
    row: &Row,
    latest: ArtifactView,
    media_type: &str,
    file: &str,
    bytes: Vec<u8>,
    note: String,
    by: String,
) -> Result<u32, ApiError> {
    let artifact = latest.id.clone();
    let session = {
        let mut slot = server.room(row.family).await.ok_or_else(not_found)?;
        let browsed = slot.browsed(row.family, viewer);
        let needle = format!("\"id\":\"{artifact}\"");
        browsed.log.iter().rev().find_map(|delta| match &delta.op {
            Op::Conv(silt_log::ConvOp::Native { session, line, .. })
                if line.contains("\"t\":\"artifact\"") && line.contains(&needle) =>
            {
                Some(session.clone())
            }
            _ => None,
        })
    }
    .ok_or_else(not_found)?;
    let version = latest.version + 1;
    let blob = server.shared.blobs.put(&bytes).await.map_err(|_| bad_request("the version couldn't be kept"))?;
    let name = format!("artifact/{artifact}/v{version}/{file}");
    let attachment = Op::Conv(silt_log::ConvOp::Attachment { session: session.clone(), name, blob });
    let view = ArtifactView {
        version,
        media_type: media_type.into(),
        blob: blob.to_string(),
        size: bytes.len() as u64,
        note: Some(note),
        by: Some(by),
        ..latest
    };
    let line = silt_agents::acp::artifact(&view);
    let native = Op::Conv(silt_log::ConvOp::Native { session, file: silt_log::SessionFile::Main, line });
    // The file and the version naming it are one push, so neither lands alone.
    crate::writes::write_all_as(server, browser, row.family, row.id, None, vec![attachment, native]).await?;
    Ok(version)
}

/// One of the files people added to a page, for a view Silt draws itself,
/// such as a whiteboard's images (ADR 0081).
async fn page_asset_content(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact, asset)): Path<(String, String, String)>,
) -> Result<Json<ArtifactContent>, ApiError> {
    use base64::Engine as _;
    if !silt_proto::page::asset_id_ok(&asset) {
        return Err(not_found());
    }
    let artifact: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
    let page = page_of(&server, browser.user, &id, artifact).await?;
    let doc = page.docs.get(&format!("_assets/{asset}")).ok_or_else(not_found)?;
    let blob: silt_types::BlobHash = doc["blob"].as_str().and_then(|blob| blob.parse().ok()).ok_or_else(not_found)?;
    let media_type = doc["media_type"].as_str().unwrap_or("application/octet-stream").to_owned();
    let bytes = server.shared.blobs.get(blob).await.map_err(|_| not_found())?;
    Ok(Json(ArtifactContent { media_type, data: base64::engine::general_purpose::STANDARD.encode(bytes) }))
}

#[derive(Deserialize)]
struct SaveVersion {
    #[serde(default)]
    note: Option<String>,
}

/// Keeps a doc, a sheet or a whiteboard as it stands as its next version,
/// from the person's Save version (ADR 0077).
async fn save_edited_version(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(save): Json<SaveVersion>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let viewer = server.viewer(browser.user).await?;
    let readable = Readable::rows(&server, browser.user, &viewer).await?;
    let row = readable.row(&id)?;
    let latest = readable_artifact(&server, browser.user, &id, &artifact, None).await?;
    if !silt_proto::edited::is_edited(&latest.kind) {
        return Err(bad_request("only a doc, a sheet or a whiteboard is saved as it stands"));
    }
    let collaborator = readable.collaborators.get(&row.id).is_some_and(|names| names.contains(&readable.me));
    if row.owner != readable.me && !collaborator {
        return Err(crate::forbidden("only the thread's owner and collaborators save its artifacts' versions"));
    }
    let artifact_id: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
    let snapshot = {
        let mut slot = server.room(row.family).await.ok_or_else(not_found)?;
        let browsed = slot.browsed(row.family, &viewer);
        match latest.kind.as_str() {
            "doc" => silt_log::doc_text_in(&browsed.log, browsed.graph(), row.id, artifact_id),
            "sheet" => {
                let book: Vec<_> = (0..silt_log::sheet_count_in(&browsed.log, browsed.graph(), row.id, artifact_id))
                    .map(|sheet| silt_log::sheet_cells_in(&browsed.log, browsed.graph(), row.id, artifact_id, sheet))
                    .collect();
                let book: Vec<&_> = book.iter().collect();
                let sheets = (0..book.len())
                    .map(|sheet| serde_json::json!({ "rows": silt_proto::edited::book_view(&book, sheet, 0.0).raw }))
                    .collect();
                silt_proto::edited::sheet_content(sheets)
            }
            _ => {
                let docs = silt_log::page_documents_in(&browsed.log, browsed.graph(), row.id, artifact_id);
                let items: Vec<&serde_json::Value> = docs
                    .iter()
                    .filter(|(path, _)| path.split('/').count() == 2 && path.starts_with("items/"))
                    .map(|(_, doc)| doc)
                    .collect();
                serde_json::json!({ "items": items }).to_string()
            }
        }
    };
    let (media_type, file) =
        if latest.kind == "doc" { (latest.media_type.clone(), "content") } else { ("application/json".into(), "content.json") };
    let note = save
        .note
        .map(|note| note.trim().chars().take(silt_proto::artifact::NOTE_CHARS).collect::<String>())
        .filter(|note| !note.is_empty());
    let note = note.unwrap_or_else(|| format!("Saved by {}", readable.me));
    let version = write_version(
        &server,
        browser,
        &viewer,
        row,
        latest,
        &media_type,
        file,
        snapshot.into_bytes(),
        note,
        readable.me.clone(),
    )
    .await?;
    Ok(Json(serde_json::json!({ "id": artifact, "version": version })))
}

/// A doc's text and a sheet's cells as the person reads them, folded from
/// the deltas they may read (ADR 0077).
async fn edited<T>(
    server: &Server,
    user: UserId,
    page: &PageOf,
    read: impl FnOnce(&[Delta], &silt_crdt::CausalGraph) -> T,
) -> Result<T, ApiError> {
    let viewer = server.viewer(user).await?;
    let mut slot = server.room(page.family).await.ok_or_else(not_found)?;
    let browsed = slot.browsed(page.family, &viewer);
    Ok(read(&browsed.log, browsed.graph()))
}

/// Whether the person may edit a doc or a sheet: anyone who may comment,
/// unless its version asks for a collaborator; the push checks it again.
fn may_edit(page: &PageOf) -> Result<(), ApiError> {
    use silt_proto::page::Level;
    let needs = match page.declared.get("edit").and_then(|edit| edit.get("level")).and_then(serde_json::Value::as_str) {
        Some("collaborator") => Level::Collaborator,
        _ => Level::Comment,
    };
    if page.level < needs { Err(crate::forbidden("you may not edit this")) } else { Ok(()) }
}

/// A doc's text, or the person's edits of it, from the hosted app (ADR 0077).
async fn doc_data(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(asked): Json<silt_proto::edited::DocRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let artifact: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
    let page = page_of(&server, browser.user, &id, artifact).await?;
    let read = |log: &[Delta], graph: &silt_crdt::CausalGraph| {
        let text = silt_log::doc_text_in(log, graph, page.thread, artifact);
        serde_json::json!({ "text": text, "base": silt_log::thread_head_in(log, graph, page.thread).to_string() })
    };
    match asked {
        silt_proto::edited::DocRequest::Read => Ok(Json(edited(&server, browser.user, &page, read).await?)),
        silt_proto::edited::DocRequest::Edit { edits, base } => {
            may_edit(&page)?;
            let base: Option<silt_types::Frontier> = match base.as_deref().filter(|base| !base.is_empty()) {
                Some(base) => Some(base.parse().map_err(|_| bad_request("that isn't a version"))?),
                None => None,
            };
            let ops = edited(&server, browser.user, &page, |log, graph| {
                let doc = silt_log::doc_crdt_in(log, graph, page.thread, artifact);
                // Counted against the text the person read, then placed by
                // character in the text now, so what others wrote since stays.
                let edits = match &base {
                    Some(base) => {
                        let seen = doc.attribution_at_vv(&graph.version_vector(base));
                        silt_proto::edited::rebased(&seen, &doc.attribution(), &edits)?
                    }
                    None => edits,
                };
                silt_proto::edited::doc_ops(&doc.text(), &edits)
            })
            .await?
            .map_err(bad_request)?;
            let ops = ops.into_iter().map(|op| Op::Page(silt_log::PageOp::Text { artifact, op })).collect();
            crate::writes::write_all_as(&server, browser, page.family, page.thread, None, ops).await?;
            Ok(Json(edited(&server, browser.user, &page, read).await?))
        }
    }
}

/// A sheet's cells and values, or the person's changes to it, from the
/// hosted app (ADR 0077).
async fn sheet_data(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(asked): Json<silt_proto::edited::SheetRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use silt_crdt::TextOp;
    use silt_proto::edited::{AxisName, SheetRequest};
    let artifact: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
    if asked.sheet() >= silt_proto::edited::SHEETS {
        return Err(bad_request(format!("an artifact has at most {} sheets", silt_proto::edited::SHEETS)));
    }
    let page = page_of(&server, browser.user, &id, artifact).await?;
    // Every sheet, through `sheet` at least, as one about to be written may be past the last.
    let book = |server: Server, sheet: u16| {
        let page = &page;
        async move {
            edited(&server, browser.user, page, |log, graph| {
                let count = silt_log::sheet_count_in(log, graph, page.thread, artifact).max(sheet + 1);
                (0..count).map(|at| silt_log::sheet_cells_in(log, graph, page.thread, artifact, at)).collect::<Vec<_>>()
            })
            .await
        }
    };
    let cells = |server: Server, sheet: u16| {
        let page = &page;
        async move {
            edited(&server, browser.user, page, |log, graph| silt_log::sheet_cells_in(log, graph, page.thread, artifact, sheet))
                .await
        }
    };
    let write = |op: silt_log::PageOp| crate::writes::write_as(&server, browser, page.family, page.thread, None, Op::Page(op));
    let write_all = |ops: Vec<silt_log::PageOp>| {
        crate::writes::write_all_as(&server, browser, page.family, page.thread, None, ops.into_iter().map(Op::Page).collect())
    };
    let lines = |sheet, axis: AxisName, op| silt_log::PageOp::Lines { artifact, sheet, axis: axis.into(), op };
    let today = (crate::now_ms() as f64 / 86_400_000.0).floor() + 25_569.0;
    match asked {
        SheetRequest::Read { sheet, range } => {
            let book = book(server.clone(), sheet).await?;
            let view = silt_proto::edited::book_view(&book.iter().collect::<Vec<_>>(), usize::from(sheet), today);
            match range {
                Some(range) => {
                    Ok(Json(serde_json::json!({ "values": silt_proto::edited::read_range(&view, &range).map_err(bad_request)? })))
                }
                None => Ok(Json(serde_json::to_value(view).unwrap_or_default())),
            }
        }
        SheetRequest::Set { sheet, cells: set } => {
            may_edit(&page)?;
            let now = cells(server.clone(), sheet).await?;
            let (rows, columns) = silt_proto::edited::sheet_growth(&now, &set).map_err(bad_request)?;
            let mut growth = Vec::new();
            if rows > 0 {
                growth.push(lines(sheet, AxisName::Rows, TextOp::Insert { pos: now.rows.len(), content: "r".repeat(rows) }));
            }
            if columns > 0 {
                growth.push(lines(
                    sheet,
                    AxisName::Columns,
                    TextOp::Insert { pos: now.columns.len(), content: "c".repeat(columns) },
                ));
            }
            write_all(growth).await?;
            let book = book(server.clone(), sheet).await?;
            let ops =
                silt_proto::edited::cell_ops_in(artifact, sheet, &book.iter().collect::<Vec<_>>(), usize::from(sheet), &set)
                    .map_err(bad_request)?;
            let written = ops.len();
            write_all(ops).await?;
            Ok(Json(serde_json::json!({ "written": written })))
        }
        SheetRequest::Insert { sheet, axis, at, count } => {
            may_edit(&page)?;
            let now = cells(server.clone(), sheet).await?;
            let len = if axis == AxisName::Rows { now.rows.len() } else { now.columns.len() };
            let most = if axis == AxisName::Rows { silt_sheet::ROWS } else { silt_sheet::COLUMNS };
            if count == 0 || len.checked_add(count).is_none_or(|grown| grown > most) {
                return Err(bad_request(format!("a sheet has at most {most} of them")));
            }
            let mark = if axis == AxisName::Rows { "r" } else { "c" };
            write(lines(sheet, axis, TextOp::Insert { pos: at.min(len), content: mark.repeat(count) })).await?;
            Ok(Json(serde_json::json!({ "inserted": count })))
        }
        SheetRequest::Remove { sheet, axis, at, count } => {
            may_edit(&page)?;
            let now = cells(server.clone(), sheet).await?;
            let len = if axis == AxisName::Rows { now.rows.len() } else { now.columns.len() };
            if count == 0 || at.checked_add(count).is_none_or(|end| end > len) {
                return Err(bad_request("those aren't in the sheet"));
            }
            write(lines(sheet, axis, TextOp::Delete { pos: at, len: count })).await?;
            Ok(Json(serde_json::json!({ "removed": count })))
        }
    }
}

/// A sheet as an `.xlsx` workbook, formulas included (ADR 0077).
async fn sheet_xlsx(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
) -> Result<axum::response::Response, ApiError> {
    use axum::http::{HeaderValue, header};
    use axum::response::IntoResponse;
    let mut sheets = Vec::new();
    let (mut sheet, mut count) = (0, 1);
    while sheet < count {
        let Json(view) = sheet_data(
            State(server.clone()),
            Extension(browser),
            Path((id.clone(), artifact.clone())),
            Json(silt_proto::edited::SheetRequest::Read { sheet, range: None }),
        )
        .await?;
        let view: silt_proto::edited::SheetView = serde_json::from_value(view).map_err(|_| not_found())?;
        count = view.sheets;
        sheets.push((format!("Sheet{}", sheet + 1), view));
        sheet += 1;
    }
    let bytes = silt_proto::edited::xlsx_book(&sheets).map_err(bad_request)?;
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    );
    headers.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("attachment; filename=\"sheet.xlsx\""));
    Ok(response)
}

/// The page document saying the thread's owner archived the artifact (ADR 0075).
const ARCHIVED: &str = "_meta/archived";

/// Every artifact in the threads the person can read, for the Artifacts
/// page (ADR 0075), with their pins.
async fn artifact_listing(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
) -> Result<Json<Vec<silt_proto::view::ArtifactListed>>, ApiError> {
    use silt_proto::view::{ArtifactListed, artifact_listing, latest_artifacts};
    let readable = Readable::load(&server, browser.user).await?;
    let titles = titles(&server, browser.user).await?;
    let pins: std::collections::HashSet<(Vec<u8>, String)> =
        sqlx::query!("SELECT thread_id, artifact_id FROM artifact_pins WHERE user_id = ?", browser.user)
            .fetch_all(server.db())
            .await?
            .into_iter()
            .map(|row| (row.thread_id, row.artifact_id))
            .collect();
    let mut rows = Vec::new();
    for row in &readable.rows {
        let Some(browsed) = readable.deltas.get(&row.family) else { continue };
        let entries = browsed.entries(row.id);
        let latest = latest_artifacts(entries.iter().map(|(_, entry)| entry));
        if latest.is_empty() {
            continue;
        }
        let graph = browsed.graph();
        for (artifact, time_ms) in latest {
            let archived = artifact.id.parse::<silt_types::ArtifactId>().is_ok_and(|id| {
                silt_log::page_documents_in(&browsed.log, graph, row.id, id).get(ARCHIVED) == Some(&serde_json::Value::Bool(true))
            });
            rows.push(ArtifactListed {
                pinned: pins.contains(&(crate::id_bytes(row.id.0).to_vec(), artifact.id.clone())),
                thread: row.id.to_string(),
                thread_title: titles.get(&row.id).cloned().flatten(),
                repo: row.repo.clone(),
                owner: row.owner.clone(),
                mine: row.owner == readable.me,
                artifact,
                time_ms,
                archived,
            });
        }
    }
    Ok(Json(artifact_listing(rows)))
}

/// The thumbnail a person's machine drew of an artifact's latest version
/// (ADR 0075); none until one has.
async fn artifact_thumbnail(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
) -> Result<Json<ArtifactContent>, ApiError> {
    use base64::Engine as _;
    let latest = readable_artifact(&server, browser.user, &id, &artifact, None).await?;
    let artifact_id: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
    let page = page_of(&server, browser.user, &id, artifact_id).await?;
    let doc = page
        .docs
        .get("_meta/thumbnail")
        .filter(|doc| doc["version"].as_u64() == Some(u64::from(latest.version)))
        .ok_or_else(not_found)?;
    let blob: silt_types::BlobHash = doc["blob"].as_str().and_then(|blob| blob.parse().ok()).ok_or_else(not_found)?;
    let bytes = server.shared.blobs.get(blob).await.map_err(|_| not_found())?;
    Ok(Json(ArtifactContent { media_type: "image/jpeg".into(), data: base64::engine::general_purpose::STANDARD.encode(bytes) }))
}

/// The artifacts the person pinned, for their machines (ADR 0075).
pub(crate) async fn device_pins(
    State(server): State<Server>,
    Extension(caller): Extension<crate::Caller>,
) -> Result<Json<Vec<silt_proto::view::ArtifactPin>>, ApiError> {
    let user = caller.user;
    let viewer = server.viewer(user).await?;
    let readable = Readable::rows(&server, user, &viewer).await?;
    let rows =
        sqlx::query!("SELECT thread_id, artifact_id FROM artifact_pins WHERE user_id = ?", user).fetch_all(server.db()).await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| silt_proto::view::ArtifactPin {
                thread: silt_types::ThreadId(crate::id_from(&row.thread_id)).to_string(),
                artifact: row.artifact_id,
                pinned: true,
            })
            // A thread the person can no longer read keeps its pin back.
            .filter(|pin| readable.row(&pin.thread).is_ok())
            .collect(),
    ))
}

/// A pin or unpin made on one of the person's machines.
pub(crate) async fn device_pin(
    State(server): State<Server>,
    Extension(caller): Extension<crate::Caller>,
    Json(pin): Json<silt_proto::view::ArtifactPin>,
) -> Result<axum::http::StatusCode, ApiError> {
    store_pin(&server, caller.user, &pin.thread, &pin.artifact, pin.pinned).await
}

async fn store_pin(
    server: &Server,
    user: UserId,
    id: &str,
    artifact: &str,
    pinned: bool,
) -> Result<axum::http::StatusCode, ApiError> {
    let _: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
    if pinned {
        readable_artifact(server, user, id, artifact, None).await?;
    }
    let viewer = server.viewer(user).await?;
    let readable = Readable::rows(server, user, &viewer).await?;
    let thread = crate::id_bytes(readable.row(id)?.id.0);
    if pinned {
        let now = crate::now_ms();
        sqlx::query!(
            "INSERT OR REPLACE INTO artifact_pins (user_id, thread_id, artifact_id, at_ms) VALUES (?, ?, ?, ?)",
            user,
            thread,
            artifact,
            now
        )
        .execute(server.db())
        .await?;
    } else {
        sqlx::query!("DELETE FROM artifact_pins WHERE user_id = ? AND thread_id = ? AND artifact_id = ?", user, thread, artifact)
            .execute(server.db())
            .await?;
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Pinned {
    pinned: bool,
}

/// Pins an artifact to the top of the person's Artifacts page, or unpins it.
async fn pin_artifact(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(pin): Json<Pinned>,
) -> Result<axum::http::StatusCode, ApiError> {
    store_pin(&server, browser.user, &id, &artifact, pin.pinned).await
}

#[derive(Deserialize)]
struct Archived {
    archived: bool,
}

/// Hides an artifact for everyone, or shows it again: the thread's owner's
/// to do, kept in the thread's page data (ADR 0075).
async fn archive_artifact(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(archive): Json<Archived>,
) -> Result<axum::http::StatusCode, ApiError> {
    let artifact: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
    let page = page_of(&server, browser.user, &id, artifact).await?;
    if page.level < silt_proto::page::Level::Owner {
        return Err(crate::forbidden("only the thread's owner archives its artifacts"));
    }
    let op = silt_log::PageOp::Set { artifact, path: ARCHIVED.into(), value: archive.archived.to_string() };
    crate::writes::write_as(&server, browser, page.family, page.thread, None, Op::Page(op)).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Who views a thread's pages, for their `user` capability.
async fn page_viewer(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<silt_proto::page::Viewer>, ApiError> {
    use silt_proto::page::Level;
    let viewer = server.viewer(browser.user).await?;
    let readable = Readable::rows(&server, browser.user, &viewer).await?;
    let row = readable.row(&id)?;
    let collaborators = readable.collaborators.get(&row.id).cloned().unwrap_or_default();
    let level = if row.owner == readable.me {
        Level::Owner
    } else if collaborators.contains(&readable.me) {
        Level::Collaborator
    } else {
        Level::Comment
    };
    let people = std::iter::once(row.owner.as_str()).chain(collaborators.iter().map(String::as_str));
    Ok(Json(silt_proto::page::Viewer::of(Some(&readable.me), level, people)))
}

#[derive(Deserialize)]
struct GrantChange {
    capability: String,
    /// `granted` or `refused`, or none to ask again.
    #[serde(default)]
    state: Option<String>,
    /// The version the person saw, whose declaration the grant is for; the
    /// latest when none.
    #[serde(default)]
    version: Option<u32>,
}

/// What the person allowed or refused an artifact's page, by capability,
/// where it was for what the latest version declares now (ADR 0072).
async fn artifact_grants(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
) -> Result<Json<std::collections::BTreeMap<String, String>>, ApiError> {
    Ok(Json(grants_held(&server, browser.user, &id, &artifact).await?))
}

/// The person's grants on an artifact that still hold.
pub(crate) async fn grants_held(
    server: &Server,
    user: UserId,
    id: &str,
    artifact: &str,
) -> Result<std::collections::BTreeMap<String, String>, ApiError> {
    let latest = readable_artifact(server, user, id, artifact, None).await?;
    let viewer = server.viewer(user).await?;
    let readable = Readable::rows(server, user, &viewer).await?;
    let thread = crate::id_bytes(readable.row(id)?.id.0);
    let rows = sqlx::query!(
        "SELECT capability, state, declaration FROM artifact_grants WHERE user_id = ? AND thread_id = ? AND artifact_id = ?",
        user,
        thread,
        artifact
    )
    .fetch_all(server.db())
    .await?;
    Ok(rows
        .into_iter()
        .filter(|row| silt_proto::artifact::grant_basis(Some(&latest), &row.capability).as_ref() == Some(&row.declaration))
        .map(|row| (row.capability, row.state))
        .collect())
}

async fn set_artifact_grant(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(change): Json<GrantChange>,
) -> Result<axum::http::StatusCode, ApiError> {
    if !silt_proto::artifact::CAPABILITIES.contains(&change.capability.as_str())
        || artifact.parse::<silt_types::ArtifactId>().is_err()
    {
        return Err(bad_request("no such capability or artifact"));
    }
    let seen = readable_artifact(&server, browser.user, &id, &artifact, change.version).await?;
    let viewer = server.viewer(browser.user).await?;
    let readable = Readable::rows(&server, browser.user, &viewer).await?;
    let thread = crate::id_bytes(readable.row(&id)?.id.0);
    match change.state.as_deref() {
        None => {
            sqlx::query!(
                "DELETE FROM artifact_grants WHERE user_id = ? AND thread_id = ? AND artifact_id = ? AND capability = ?",
                browser.user,
                thread,
                artifact,
                change.capability
            )
            .execute(server.db())
            .await?;
        }
        Some(state @ ("granted" | "refused")) => {
            let declaration = silt_proto::artifact::grant_basis(Some(&seen), &change.capability)
                .ok_or_else(|| bad_request(format!("artifact {artifact} doesn't declare {}", change.capability)))?;
            let at = now_ms();
            sqlx::query!(
                "INSERT INTO artifact_grants (user_id, thread_id, artifact_id, capability, state, declaration, at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (user_id, thread_id, artifact_id, capability)
                 DO UPDATE SET state = excluded.state, declaration = excluded.declaration, at_ms = excluded.at_ms",
                browser.user,
                thread,
                artifact,
                change.capability,
                state,
                declaration,
                at
            )
            .execute(server.db())
            .await?;
        }
        Some(_) => return Err(bad_request("a grant is granted or refused")),
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// What a frame key opens on the server: one version of one artifact, for
/// the person who asked, whose right to read the thread is checked again
/// each time the frame loads (ADR 0074).
#[derive(Debug, Clone)]
pub(crate) struct FrameGrant {
    user: UserId,
    thread: String,
    artifact: String,
    version: Option<u32>,
    theme: Option<silt_design::frame::Theme>,
    parent: Option<String>,
    boot: Option<String>,
}

#[derive(Deserialize)]
struct FrameAsk {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    boot: Option<serde_json::Value>,
}

/// The version of artifact `id` a person may read, the latest when
/// `version` is none, along the thread's lineage.
pub(crate) async fn readable_artifact(
    server: &Server,
    user: UserId,
    thread: &str,
    id: &str,
    version: Option<u32>,
) -> Result<ArtifactView, ApiError> {
    let viewer = server.viewer(user).await?;
    let readable = Readable::rows(server, user, &viewer).await?;
    let row = readable.row(thread)?;
    let pick = |entries: &[(DeltaId, HistoryEntry)]| {
        entries
            .iter()
            .filter_map(|(_, entry)| entry.artifact.as_ref())
            .filter(|artifact| !artifact.inline && artifact.id == id && version.is_none_or(|wanted| wanted == artifact.version))
            .max_by_key(|artifact| artifact.version)
            .cloned()
    };
    let found = match lineage_entries(server, &viewer, row).await {
        Some(entries) => pick(&entries),
        None => server.shared.index.lock().await.history(row.id).and_then(|history| pick(&history.entries)),
    };
    found.ok_or_else(not_found)
}

async fn make_frame(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(ask): Json<FrameAsk>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let view = readable_artifact(&server, browser.user, &id, &artifact, ask.version).await?;
    if view.kind != "html" {
        return Err(bad_request("only an HTML artifact opens in a frame"));
    }
    let grant = FrameGrant {
        user: browser.user,
        thread: id,
        artifact,
        version: ask.version,
        theme: ask.theme.as_deref().and_then(silt_design::frame::Theme::parse),
        parent: ask.parent.filter(|origin| silt_design::frame::is_origin(origin)),
        boot: ask
            .boot
            .filter(serde_json::Value::is_object)
            .map(|boot| boot.to_string())
            .filter(|text| text.len() <= silt_design::frame::BOOT_BYTES),
    };
    let key = crate::accounts::random_token();
    let owner = browser.user.to_string();
    server.shared.frames.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key.clone(), grant, &owner);
    Ok(Json(
        serde_json::json!({ "path": format!("/f/{key}/"), "expires_in_ms": silt_proto::frames::FRAME_TTL.as_millis() as u64 }),
    ))
}

/// The page a frame key opens, composed as every surface composes it, with
/// its policy as a header that also makes its origin opaque.
pub(crate) async fn serve_frame(State(server): State<Server>, Path(key): Path<String>) -> axum::response::Response {
    use axum::http::{HeaderValue, StatusCode, header};
    use axum::response::IntoResponse;
    let not_found = || (StatusCode::NOT_FOUND, "this frame's key expired or never existed").into_response();
    let grant = server.shared.frames.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key);
    let Some(grant) = grant else {
        return not_found();
    };
    let Ok(view) = readable_artifact(&server, grant.user, &grant.thread, &grant.artifact, grant.version).await else {
        return not_found();
    };
    let Ok(hash) = view.blob.parse::<silt_types::BlobHash>() else {
        return not_found();
    };
    let Ok(content) = server.shared.blobs.get(hash).await else {
        return not_found();
    };
    let style = match view.style.as_deref().and_then(|style| style.parse::<silt_types::BlobHash>().ok()) {
        Some(style) => server.shared.blobs.get(style).await.ok().map(|css| String::from_utf8_lossy(&css).into_owned()),
        None => None,
    };
    let vendor = server.config().artifact_proxy.then(|| format!("{}/vendor/", server.config().public_url()));
    let parts = silt_design::frame::FrameParts {
        style: style.as_deref(),
        theme: grant.theme,
        probe: true,
        vendor: vendor.as_deref(),
        parent: grant.parent.as_deref(),
        policy_meta: false,
        boot: grant.boot.as_deref(),
    };
    let page = silt_design::frame::compose(&String::from_utf8_lossy(&content), &parts);
    let files = format!("{}/f/{key}/", server.config().public_url());
    let policy = silt_design::frame::policy_with_files(vendor.as_deref(), &files);
    let mut response = (StatusCode::OK, page).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    if let Ok(value) = HeaderValue::from_str(&format!("sandbox allow-scripts; {policy}")) {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert("cross-origin-resource-policy", HeaderValue::from_static("same-site"));
    response
}

/// A file beside a frame's version, or an asset someone added to its page
/// (ADR 0074), for the person the key was made for.
pub(crate) async fn frame_file(
    State(server): State<Server>,
    Path((key, path)): Path<(String, String)>,
) -> axum::response::Response {
    use axum::http::{HeaderValue, StatusCode, header};
    use axum::response::IntoResponse;
    let not_found = || (StatusCode::NOT_FOUND, "no such file").into_response();
    let grant = server.shared.frames.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key);
    let Some(grant) = grant else {
        return not_found();
    };
    let found = match path.strip_prefix("_assets/") {
        Some(asset) if silt_proto::page::asset_id_ok(asset) => {
            let Ok(artifact) = grant.artifact.parse::<silt_types::ArtifactId>() else {
                return not_found();
            };
            let Ok(page) = page_of(&server, grant.user, &grant.thread, artifact).await else {
                return not_found();
            };
            page.docs.get(&format!("_assets/{asset}")).and_then(|doc| {
                Some((doc["blob"].as_str()?.parse::<silt_types::BlobHash>().ok()?, doc["media_type"].as_str()?.to_owned()))
            })
        }
        Some(_) => None,
        None => match readable_artifact(&server, grant.user, &grant.thread, &grant.artifact, grant.version).await {
            Ok(view) => view
                .files
                .and_then(|files| files.get(&path).cloned())
                .and_then(|file| Some((file.blob.parse::<silt_types::BlobHash>().ok()?, file.media_type))),
            Err(_) => None,
        },
    };
    let Some((blob, media_type)) = found else {
        return not_found();
    };
    let Ok(bytes) = server.shared.blobs.get(blob).await else {
        return not_found();
    };
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&media_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(silt_design::frame::FILE_POLICY));
    headers.insert("cross-origin-resource-policy", HeaderValue::from_static("cross-origin"));
    response
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

/// A doc's text CRDT in a thread the person sees, which comments on it
/// anchor in; `artifact` must be one of the thread's docs.
pub(crate) async fn seen_doc(
    server: &Server,
    user: UserId,
    selector: &str,
    artifact: &str,
) -> Result<(silt_types::ArtifactId, silt_crdt::TextDoc), ApiError> {
    let readable = Readable::load(server, user).await?;
    let row = readable.row(selector)?;
    let browsed = readable.deltas.get(&row.family).ok_or_else(not_found)?;
    let entries = browsed.entries(row.id);
    let kind = silt_proto::view::latest_artifacts(entries.iter().map(|(_, entry)| entry))
        .into_iter()
        .find(|(view, _)| view.id == artifact)
        .map(|(view, _)| view.kind)
        .ok_or_else(|| bad_request("this thread has no such artifact"))?;
    if kind != "doc" {
        return Err(bad_request("only a doc's text takes comments"));
    }
    let id: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
    Ok((id, silt_log::doc_crdt_in(&browsed.log, browsed.graph(), row.id, id)))
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
    let comments = comments_in(log, row.id);
    let commented: BTreeSet<silt_types::ArtifactId> = comments
        .iter()
        .filter_map(|comment| match comment.target {
            CommentTarget::Page { artifact, .. } => Some(artifact),
            _ => None,
        })
        .collect();
    let docs: HashMap<_, _> = match readable.deltas.get(&row.family) {
        Some(browsed) => commented
            .into_iter()
            .map(|artifact| (artifact, silt_log::doc_crdt_in(log, browsed.graph(), row.id, artifact)))
            .collect(),
        None => HashMap::new(),
    };
    Ok(Json(
        comments
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
                    CommentTarget::Page { artifact, start, end } => docs.get(&artifact).map_or(CommentLocation::Unknown, |doc| {
                        silt_proto::edited::doc_comment_location(doc, artifact, &start, &end)
                    }),
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
                artifact: hit.artifact,
            })
            .collect(),
    ))
}

/// Search hits in the current text of the docs and sheets of the threads
/// the person can read, newest first. Their text isn't in the index, as
/// it's the sum of every edit rather than any one delta; a branch or file
/// filter leaves them out, as a doc has neither.
pub(crate) async fn edited_hits(
    server: &Server,
    user: UserId,
    query: &SearchQuery,
    limit: usize,
) -> Result<Vec<silt_proto::SearchHit>, ApiError> {
    if query.branch.is_some() || query.file.is_some() || query.q.trim().is_empty() {
        return Ok(Vec::new());
    }
    let readable = Readable::load(server, user).await?;
    let mut found = Vec::new();
    for row in &readable.rows {
        if query.repo.as_ref().is_some_and(|repo| *repo != row.repo)
            || query.author.as_ref().is_some_and(|author| *author != row.owner)
        {
            continue;
        }
        let Some(browsed) = readable.deltas.get(&row.family) else { continue };
        let entries = browsed.entries(row.id);
        for (artifact, time_ms) in silt_proto::view::latest_artifacts(entries.iter().map(|(_, entry)| entry)) {
            if !matches!(artifact.kind.as_str(), "doc" | "sheet")
                || query.since.is_some_and(|since| time_ms < since)
                || query.until.is_some_and(|until| time_ms > until)
            {
                continue;
            }
            let Ok(id) = artifact.id.parse::<silt_types::ArtifactId>() else { continue };
            if let Some(snippet) = silt_proto::edited::text_hit(&browsed.edited_text(row.id, id, &artifact.kind), &query.q) {
                found.push((row, artifact.kind.clone(), snippet, time_ms, artifact.id.clone()));
            }
        }
    }
    found.sort_by_key(|(.., time_ms, _)| std::cmp::Reverse(*time_ms));
    let mut hits = Vec::new();
    for (row, kind, snippet, time_ms, artifact) in found {
        let thread_id = crate::id_bytes(row.id.0).to_vec();
        let activity = sqlx::query!("SELECT branch, merged, last_ms FROM thread_activity WHERE thread_id = ?", thread_id)
            .fetch_optional(server.db())
            .await?;
        let branch = activity.as_ref().and_then(|row| row.branch.clone());
        let status = match activity {
            Some(row) if row.merged == 1 => silt_proto::ThreadStatus::Merged,
            Some(row) if row.last_ms >= now_ms() - silt_proto::ACTIVE_MS => silt_proto::ThreadStatus::Active,
            _ => silt_proto::ThreadStatus::Idle,
        };
        if query.status.is_some_and(|wanted| wanted != status) {
            continue;
        }
        hits.push(silt_proto::SearchHit {
            thread: row.id,
            repo: row.repo.clone(),
            owner: row.owner.clone(),
            status,
            branch,
            kind,
            snippet,
            time_ms,
            artifact: Some(artifact),
        });
        if hits.len() >= limit {
            break;
        }
    }
    Ok(hits)
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
