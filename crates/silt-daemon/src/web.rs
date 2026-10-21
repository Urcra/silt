//! The web UI's server (ADR 0011): the built `web/` app and a JSON API on
//! 127.0.0.1, guarded by a per-start token and a `Host` check. Every API
//! call becomes a socket [`Request`], so the UI and the CLI share one path.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path, Query, Request as HttpRequest, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response as HttpResponse};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::core::Event;
use crate::protocol::{BusEvent, BusSubscription, CommentOn, Request, Response, SearchFilters};

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Assets;

#[derive(Clone)]
struct Web {
    events: mpsc::UnboundedSender<Event>,
    bus: broadcast::Sender<BusEvent>,
    token: String,
    port: u16,
    codes: OneTimeCodes,
}

/// Codes that each stand in for the token once, shortly. Shared between the
/// daemon, which hands them out, and the web server, which trades them in.
type OneTimeCodes = Arc<Mutex<HashMap<String, Instant>>>;

/// How long a one-time code may wait for its browser.
const CODE_TTL: Duration = Duration::from_secs(60);

/// A running web UI.
#[derive(Debug, Clone)]
pub(crate) struct WebInfo {
    pub(crate) port: u16,
    pub(crate) token: String,
    codes: OneTimeCodes,
}

impl WebInfo {
    pub(crate) fn url(&self) -> String {
        format!("http://127.0.0.1:{}/#token={}", self.port, self.token)
    }

    /// A URL for a browser's command line, which other users can read: it
    /// carries a code the page trades for the token once, within a minute.
    pub(crate) fn one_time_url(&self) -> String {
        let code = uuid::Uuid::new_v4().simple().to_string();
        let mut codes = self.codes.lock().expect("lock");
        codes.retain(|_, made| made.elapsed() < CODE_TTL);
        codes.insert(code.clone(), Instant::now());
        format!("http://127.0.0.1:{}/#code={code}", self.port)
    }
}

/// Starts the web server on 127.0.0.1 (`SILT_WEB_PORT`, or any free port).
pub(crate) async fn start(events: mpsc::UnboundedSender<Event>, bus: broadcast::Sender<BusEvent>) -> std::io::Result<WebInfo> {
    let port = std::env::var("SILT_WEB_PORT").ok().and_then(|port| port.parse().ok()).unwrap_or(0);
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let codes = OneTimeCodes::default();
    let web = Web { events, bus, token: token.clone(), port, codes: codes.clone() };
    let api = Router::new()
        .route("/api/events", get(events_socket))
        .route("/api/status", get(status))
        .route("/api/threads", get(threads))
        .route("/api/threads/{id}/history", get(history))
        .route("/api/threads/{id}/show", get(show))
        .route("/api/threads/{id}/turns", get(turns))
        .route("/api/threads/{id}/diff", get(diff))
        .route("/api/threads/{id}/comments", get(comments).post(add_comment))
        .route("/api/threads/{id}/comments/{comment}/resolve", post(resolve_comment))
        .route("/api/threads/{id}/comments/{comment}/replies", post(reply))
        .route("/api/search", get(search))
        .route("/api/threads/{id}/replica", get(replica).post(browser_deltas))
        .route("/api/threads/{id}/git/{oid}", get(git_text))
        .route("/api/threads/{id}/live", get(live))
        .route("/api/threads/{id}/presence", post(presence))
        .route("/api/agents", get(agents))
        .route("/api/agents/start", post(agent_start))
        .route("/api/clones/branches", get(branches))
        .route("/api/projects", get(projects).post(add_clone))
        .route("/api/orgs", get(orgs))
        .route("/api/orgs/change", post(org_change))
        .route("/api/me/notifications", get(notification_settings).put(notification_setting))
        .route("/api/devices", get(devices))
        .route("/api/pair", post(pair_phone))
        .route("/api/devices/{id}", axum::routing::patch(device_change))
        .route("/api/devices/{id}/revoke", post(device_revoke))
        .route("/api/threads/{id}/state", axum::routing::patch(thread_state))
        .route("/api/projects/clone", post(clone_project))
        .route("/api/threads/{id}/agent/prompt", post(agent_prompt).layer(DefaultBodyLimit::max(PROMPT_BODY_BYTES)))
        .route("/api/threads/{id}/agent/cancel", post(agent_cancel))
        .route("/api/threads/{id}/agent/answer", post(agent_answer))
        .route("/api/threads/{id}/agent/stop", post(agent_stop))
        .route("/api/threads/{id}/agent/steer", post(agent_steer))
        .route("/api/threads/{id}/agent/preset", post(agent_preset))
        .route("/api/threads/{id}/agent/mode", post(agent_mode))
        .route("/api/threads/{id}/agent/setting", post(agent_setting))
        .route("/api/threads/{id}/queue", get(queue).post(queue_add))
        .route("/api/threads/{id}/queue/{item}", put(queue_edit).delete(queue_remove))
        .route("/api/threads/{id}/queue/{item}/move", post(queue_move))
        .route("/api/threads/{id}/queue/{item}/send", post(queue_send))
        .route("/api/inbox", get(inbox))
        .route("/api/inbox/read", post(inbox_read))
        .route("/api/mrs", get(merge_requests))
        .route("/api/mrs/{id}", get(merge_request))
        .route("/api/mrs/{id}/approve", post(approve))
        .route("/api/mrs/{id}/decline", post(decline))
        .route("/api/mrs/{id}/request-changes", post(mr_request_changes))
        .route("/api/mrs/{id}/seen", put(mr_seen))
        .route("/api/mrs/{id}/guide", post(mr_guide))
        .route("/api/threads/{id}", delete(thread_purge))
        .route("/api/threads/{id}/fork", post(fork))
        .route("/api/threads/{id}/resume", post(resume))
        .route("/api/threads/{id}/pull", post(pull))
        .route("/api/threads/{id}/mrs", post(mr_create))
        .route("/api/threads/{id}/visibility", post(visibility))
        .route("/api/threads/{id}/share", post(share))
        .route("/api/threads/{id}/trash", post(thread_delete))
        .route("/api/threads/{id}/restore", post(thread_restore))
        .route("/api/threads/{id}/files/history", get(file_history))
        .route("/api/threads/{id}/in-flight", get(in_flight))
        .route("/api/threads/{id}/search", get(thread_search))
        .route("/api/trash", get(trash))
        .layer(middleware::from_fn_with_state(web.clone(), authenticate));
    let app = api
        .route("/api/exchange", post(exchange))
        .fallback(asset)
        .layer(middleware::from_fn_with_state(web.clone(), local_host_only))
        .with_state(web);
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::warn!(%error, "the web UI stopped");
        }
    });
    Ok(WebInfo { port, token, codes })
}

#[derive(Deserialize)]
struct Exchange {
    code: String,
}

/// Trades a one-time code from `silt open` for the token.
async fn exchange(State(web): State<Web>, Json(body): Json<Exchange>) -> HttpResponse {
    let fresh = web.codes.lock().expect("lock").remove(&body.code).is_some_and(|made| made.elapsed() < CODE_TTL);
    if fresh {
        Json(json!({ "token": web.token })).into_response()
    } else {
        (StatusCode::UNAUTHORIZED, Json(json!({"error": "this link was used already or is too old; run `silt open` again"})))
            .into_response()
    }
}

/// Refuses requests addressed to any other host, so a web page elsewhere
/// can't reach the API through DNS rebinding.
async fn local_host_only(State(web): State<Web>, request: HttpRequest, next: Next) -> HttpResponse {
    let host = request.headers().get(header::HOST).and_then(|value| value.to_str().ok());
    let allowed = [format!("127.0.0.1:{}", web.port), format!("localhost:{}", web.port)];
    if host.is_some_and(|host| allowed.iter().any(|allowed| allowed == host)) {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "unexpected host").into_response()
    }
}

async fn authenticate(State(web): State<Web>, request: HttpRequest, next: Next) -> HttpResponse {
    let header_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    // The browser's WebSocket can't send headers, so the event bus carries
    // the token in the query instead.
    let query_token = request.uri().query().and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("token=")));
    if header_token.or(query_token).is_some_and(|given| same_token(given, &web.token)) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, Json(json!({"error": "open the UI with `silt open`"}))).into_response()
    }
}

/// Compares without stopping at the first difference, so response timing
/// doesn't reveal how much of a guess was right.
fn same_token(given: &str, expected: &str) -> bool {
    let difference = given.bytes().zip(expected.bytes()).fold(0u8, |difference, (a, b)| difference | (a ^ b));
    given.len() == expected.len() && std::hint::black_box(difference) == 0
}

async fn asset(request: HttpRequest) -> HttpResponse {
    let path = request.uri().path().trim_start_matches('/');
    let file = match Assets::get(path) {
        Some(file) if !path.is_empty() => file,
        _ => match Assets::get("index.html") {
            Some(file) => file,
            None => {
                return (StatusCode::NOT_FOUND, "the web UI was not built; run `pnpm -C web build` and rebuild silt")
                    .into_response();
            }
        },
    };
    let mime = file.metadata.mimetype().to_owned();
    ([(header::CONTENT_TYPE, mime)], Body::from(file.data.into_owned())).into_response()
}

/// Sends a request through the daemon's event loop.
async fn ask(web: &Web, request: Request) -> HttpResponse {
    match answer(web, request).await {
        Ok(data) => Json(data).into_response(),
        Err(failed) => failed.into_response(),
    }
}

/// A request the daemon didn't carry out.
struct Failed {
    status: StatusCode,
    message: String,
}

impl IntoResponse for Failed {
    fn into_response(self) -> HttpResponse {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

async fn answer(web: &Web, request: Request) -> Result<serde_json::Value, Failed> {
    let unavailable = |message: &str| Failed { status: StatusCode::SERVICE_UNAVAILABLE, message: message.to_owned() };
    let (reply, answer) = oneshot::channel();
    if web.events.send(Event::Request(request, reply)).is_err() {
        return Err(unavailable("the daemon is stopping"));
    }
    match answer.await {
        Ok(Response::Ok { data }) => Ok(data),
        Ok(Response::Error { message }) => Err(Failed { status: StatusCode::BAD_REQUEST, message }),
        Err(_) => Err(unavailable("the daemon stopped")),
    }
}

/// Asks the request `make` builds for this machine's clone of `thread`, as
/// requests the CLI makes from a working directory need one.
async fn ask_in_clone(web: &Web, thread: &str, make: impl FnOnce(PathBuf) -> Request) -> HttpResponse {
    let clone = match answer(web, Request::CloneOf { thread: thread.to_owned() }).await {
        Ok(clone) => clone,
        Err(failed) => return failed.into_response(),
    };
    let Ok(clone) = serde_json::from_value::<PathBuf>(clone) else {
        let message = "unexpected answer from the daemon".to_owned();
        return Failed { status: StatusCode::INTERNAL_SERVER_ERROR, message }.into_response();
    };
    ask(web, make(clone)).await
}

/// The browser's event bus (ADR 0021).
async fn events_socket(State(web): State<Web>, upgrade: WebSocketUpgrade) -> HttpResponse {
    let bus = web.bus.subscribe();
    upgrade.on_upgrade(move |socket| serve_events(socket, bus))
}

/// Sends the socket the events of the topics it subscribed to, until it
/// closes.
async fn serve_events(mut socket: WebSocket, mut bus: broadcast::Receiver<BusEvent>) {
    let mut topics: HashSet<String> = HashSet::new();
    loop {
        let event = tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(change) = serde_json::from_str::<BusSubscription>(&text) {
                        topics.extend(change.subscribe);
                        topics.retain(|topic| !change.unsubscribe.contains(topic));
                    }
                    continue;
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => return,
            },
            event = bus.recv() => match event {
                Ok(event) if event.topic().is_none_or(|topic| topics.contains(&topic)) => event,
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => BusEvent::Lagged,
                Err(broadcast::error::RecvError::Closed) => return,
            },
        };
        let Ok(text) = serde_json::to_string(&event) else {
            continue;
        };
        if socket.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }
}

async fn status(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Status).await
}

async fn threads(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Threads).await
}

async fn history(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    let request = Request::HistoryRead { thread, offset: 0, limit: usize::MAX };
    ask(&web, request).await
}

async fn show(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Show { thread }).await
}

async fn turns(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Turns { thread }).await
}

#[derive(Deserialize)]
struct DiffQuery {
    turn: Option<usize>,
    base: Option<String>,
}

async fn diff(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<DiffQuery>) -> HttpResponse {
    let request = Request::Diff { thread, turn: query.turn, base: query.base.filter(|base| !base.is_empty()) };
    ask(&web, request).await
}

/// The web UI has no clone of its own; comments that need one say so.
fn no_clone() -> std::path::PathBuf {
    std::path::PathBuf::from("/")
}

async fn comments(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Comments { thread, cwd: no_clone() }).await
}

#[derive(Deserialize)]
struct NewComment {
    on: CommentOn,
    text: String,
}

async fn add_comment(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<NewComment>) -> HttpResponse {
    let request = Request::Comment { thread, on: body.on, text: body.text, cwd: no_clone() };
    ask(&web, request).await
}

async fn resolve_comment(State(web): State<Web>, Path((thread, comment)): Path<(String, String)>) -> HttpResponse {
    let request = Request::ResolveComment { thread, comment, cwd: no_clone() };
    ask(&web, request).await
}

#[derive(Deserialize)]
struct NewReply {
    text: String,
}

async fn reply(
    State(web): State<Web>,
    Path((thread, comment)): Path<(String, String)>,
    Json(body): Json<NewReply>,
) -> HttpResponse {
    ask(&web, Request::Reply { thread, comment, text: body.text, cwd: no_clone() }).await
}

async fn search(State(web): State<Web>, Query(query): Query<HashMap<String, String>>) -> HttpResponse {
    let text = |name: &str| query.get(name).filter(|value| !value.is_empty()).cloned();
    let number = |name: &str| text(name).and_then(|value| value.parse().ok());
    let request = Request::Search {
        query: text("q").unwrap_or_default(),
        limit: 50,
        filters: SearchFilters {
            repo: text("repo"),
            author: text("author"),
            status: text("status").and_then(|status| status.parse().ok()),
            branch: text("branch"),
            file: text("file"),
            since: number("since"),
            until: number("until"),
        },
    };
    ask(&web, request).await
}

#[derive(Deserialize)]
struct AfterQuery {
    #[serde(default)]
    after: usize,
}

async fn replica(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<AfterQuery>) -> HttpResponse {
    ask(&web, Request::Replica { thread, after: query.after }).await
}

#[derive(Deserialize)]
struct BrowserDeltas {
    deltas: String,
}

async fn browser_deltas(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<BrowserDeltas>) -> HttpResponse {
    ask(&web, Request::BrowserDeltas { thread, deltas: body.deltas }).await
}

async fn git_text(State(web): State<Web>, Path((thread, oid)): Path<(String, String)>) -> HttpResponse {
    ask(&web, Request::GitText { thread, oid }).await
}

#[derive(Deserialize, Default)]
struct PresenceBody {
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    draft: String,
    #[serde(default)]
    leaving: bool,
}

async fn presence(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<PresenceBody>) -> HttpResponse {
    let request = Request::Presence { thread, file: body.file, line: body.line, draft: body.draft, leaving: body.leaving };
    ask(&web, request).await
}

/// The thread's live state now; what changes later comes on the event bus.
async fn live(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Live { thread }).await
}

async fn agents(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::AcpAgents).await
}

#[derive(Deserialize)]
struct AgentStart {
    #[serde(default)]
    thread: Option<String>,
    #[serde(default)]
    clone: Option<std::path::PathBuf>,
    agent: String,
    #[serde(default, flatten)]
    options: crate::protocol::StartOptions,
}

async fn agent_start(State(web): State<Web>, Json(body): Json<AgentStart>) -> HttpResponse {
    let request = Request::AcpStart { thread: body.thread, clone: body.clone, agent: body.agent, options: body.options };
    ask(&web, request).await
}

#[derive(Deserialize)]
struct CloneQuery {
    path: std::path::PathBuf,
}

async fn branches(State(web): State<Web>, Query(query): Query<CloneQuery>) -> HttpResponse {
    ask(&web, Request::Branches { clone: query.path }).await
}

async fn thread_state(
    State(web): State<Web>,
    Path(thread): Path<String>,
    Json(change): Json<crate::protocol::StateChange>,
) -> HttpResponse {
    ask(&web, Request::ThreadState { thread, change }).await
}

async fn orgs(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Orgs).await
}

async fn org_change(State(web): State<Web>, Json(change): Json<silt_proto::OrgChange>) -> HttpResponse {
    ask(&web, Request::OrgChange { change }).await
}

async fn projects(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Projects).await
}

async fn notification_settings(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::NotificationSettings).await
}

async fn notification_setting(State(web): State<Web>, Json(setting): Json<silt_proto::KindSetting>) -> HttpResponse {
    ask(&web, Request::NotificationSetting { setting }).await
}

async fn pair_phone(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::PairPhone).await
}

async fn devices(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Devices).await
}

async fn device_change(
    State(web): State<Web>,
    Path(id): Path<String>,
    Json(change): Json<crate::protocol::DeviceChange>,
) -> HttpResponse {
    ask(&web, Request::DeviceChange { id, change }).await
}

async fn device_revoke(State(web): State<Web>, Path(id): Path<String>) -> HttpResponse {
    ask(&web, Request::DeviceRevoke { id }).await
}

async fn add_clone(State(web): State<Web>, Json(body): Json<CloneQuery>) -> HttpResponse {
    ask(&web, Request::AddClone { path: body.path }).await
}

#[derive(Deserialize)]
struct CloneProject {
    url: String,
    path: PathBuf,
}

async fn clone_project(State(web): State<Web>, Json(body): Json<CloneProject>) -> HttpResponse {
    ask(&web, Request::CloneProject { url: body.url, path: body.path }).await
}

#[derive(Deserialize)]
struct AgentPrompt {
    text: String,
    #[serde(default)]
    plan: bool,
    #[serde(default)]
    attachments: Vec<crate::protocol::PromptAttachment>,
}

/// A prompt's body: its files, in base64, as large as `acp::decode` takes.
const PROMPT_BODY_BYTES: usize = 36 << 20;

async fn agent_prompt(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<AgentPrompt>) -> HttpResponse {
    ask(&web, Request::AcpPrompt { thread, text: body.text, plan: body.plan, attachments: body.attachments }).await
}

async fn agent_cancel(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::AcpCancel { thread }).await
}

#[derive(Deserialize)]
struct AgentAnswer {
    id: u64,
    #[serde(default)]
    option: Option<String>,
}

async fn agent_answer(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<AgentAnswer>) -> HttpResponse {
    let request = Request::AcpAnswer { thread, id: body.id, option: body.option };
    ask(&web, request).await
}

async fn agent_steer(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<AgentPrompt>) -> HttpResponse {
    ask(&web, Request::AcpSteer { thread, text: body.text }).await
}

#[derive(Deserialize)]
struct PresetBody {
    preset: crate::protocol::Preset,
}

async fn agent_preset(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<PresetBody>) -> HttpResponse {
    ask(&web, Request::AgentPreset { thread, preset: body.preset }).await
}

#[derive(Deserialize)]
struct ModeBody {
    mode: String,
}

async fn agent_mode(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<ModeBody>) -> HttpResponse {
    ask(&web, Request::AcpMode { thread, mode: body.mode }).await
}

#[derive(Deserialize)]
struct SettingBody {
    id: String,
    value: String,
}

async fn agent_setting(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<SettingBody>) -> HttpResponse {
    ask(&web, Request::AcpSetting { thread, id: body.id, value: body.value }).await
}

async fn queue(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Queue { thread }).await
}

async fn queue_add(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<AgentPrompt>) -> HttpResponse {
    ask(&web, Request::QueueAdd { thread, text: body.text }).await
}

async fn queue_edit(
    State(web): State<Web>,
    Path((thread, id)): Path<(String, i64)>,
    Json(body): Json<AgentPrompt>,
) -> HttpResponse {
    ask(&web, Request::QueueEdit { thread, id, text: body.text }).await
}

async fn queue_remove(State(web): State<Web>, Path((thread, id)): Path<(String, i64)>) -> HttpResponse {
    ask(&web, Request::QueueRemove { thread, id }).await
}

#[derive(Deserialize)]
struct QueueMove {
    to: usize,
}

async fn queue_move(
    State(web): State<Web>,
    Path((thread, id)): Path<(String, i64)>,
    Json(body): Json<QueueMove>,
) -> HttpResponse {
    ask(&web, Request::QueueMove { thread, id, to: body.to }).await
}

async fn queue_send(State(web): State<Web>, Path((thread, id)): Path<(String, i64)>) -> HttpResponse {
    ask(&web, Request::QueueSend { thread, id }).await
}

async fn agent_stop(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::AcpStop { thread }).await
}

#[derive(Deserialize)]
struct InboxQuery {
    #[serde(default)]
    all: bool,
}

async fn inbox(State(web): State<Web>, Query(query): Query<InboxQuery>) -> HttpResponse {
    ask(&web, Request::Inbox { all: query.all }).await
}

async fn inbox_read(State(web): State<Web>, Json(read): Json<silt_proto::InboxRead>) -> HttpResponse {
    ask(&web, Request::InboxRead { read }).await
}

async fn merge_requests(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::MrList { thread: None }).await
}

async fn merge_request(State(web): State<Web>, Path(mr): Path<String>) -> HttpResponse {
    ask(&web, Request::MrShow { mr }).await
}

async fn approve(State(web): State<Web>, Path(mr): Path<String>) -> HttpResponse {
    ask(&web, Request::MrApprove { mr }).await
}

#[derive(Deserialize, Default)]
struct Decline {
    reason: Option<String>,
}

async fn decline(State(web): State<Web>, Path(mr): Path<String>, body: Option<Json<Decline>>) -> HttpResponse {
    let reason = body.map(|Json(body)| body).unwrap_or_default().reason;
    ask(&web, Request::MrDecline { mr, reason }).await
}

#[derive(Deserialize)]
struct ChangesBody {
    reason: String,
}

async fn mr_request_changes(State(web): State<Web>, Path(mr): Path<String>, Json(body): Json<ChangesBody>) -> HttpResponse {
    ask(&web, Request::MrRequestChanges { mr, reason: body.reason }).await
}

#[derive(Deserialize)]
struct SeenBody {
    path: String,
    seen: bool,
}

async fn mr_seen(State(web): State<Web>, Path(mr): Path<String>, Json(body): Json<SeenBody>) -> HttpResponse {
    ask(&web, Request::MrSeen { mr, path: body.path, seen: body.seen }).await
}

#[derive(Deserialize, Default)]
struct GuideBody {
    #[serde(default)]
    text: Option<String>,
}

async fn mr_guide(State(web): State<Web>, Path(mr): Path<String>, body: Option<Json<GuideBody>>) -> HttpResponse {
    let GuideBody { text } = body.map(|Json(body)| body).unwrap_or_default();
    ask(&web, Request::MrGuide { mr, text }).await
}

#[derive(Deserialize, Default)]
struct Continue {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    private: bool,
}

/// A fork in a checkout of its own, to start an agent in from the UI.
async fn fork(State(web): State<Web>, Path(thread): Path<String>, body: Option<Json<Continue>>) -> HttpResponse {
    let body = body.map(|Json(body)| body).unwrap_or_default();
    let request =
        |cwd| Request::Fork { thread: thread.clone(), cwd, here: false, agent: body.agent.clone(), private: body.private };
    ask_in_clone(&web, &thread, request).await
}

async fn resume(State(web): State<Web>, Path(thread): Path<String>, body: Option<Json<Continue>>) -> HttpResponse {
    let agent = body.and_then(|Json(body)| body.agent);
    ask_in_clone(&web, &thread, |cwd| Request::Resume { thread: thread.clone(), cwd, here: false, agent }).await
}

async fn pull(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask_in_clone(&web, &thread, |cwd| Request::Pull { thread: Some(thread.clone()), cwd }).await
}

#[derive(Deserialize, Default)]
struct NewMr {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

async fn mr_create(State(web): State<Web>, Path(thread): Path<String>, body: Option<Json<NewMr>>) -> HttpResponse {
    let NewMr { title, summary } = body.map(|Json(body)| body).unwrap_or_default();
    ask_in_clone(&web, &thread, |cwd| Request::MrCreate { thread: Some(thread.clone()), cwd, title, summary }).await
}

#[derive(Deserialize)]
struct NewVisibility {
    visibility: silt_proto::Visibility,
}

async fn visibility(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<NewVisibility>) -> HttpResponse {
    let request = |cwd| Request::SetVisibility { cwd, thread: Some(thread.clone()), visibility: body.visibility };
    ask_in_clone(&web, &thread, request).await
}

#[derive(Deserialize)]
struct Share {
    user: String,
    #[serde(default)]
    remove: bool,
}

async fn share(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<Share>) -> HttpResponse {
    ask(&web, Request::ThreadShare { thread, user: body.user, remove: body.remove }).await
}

async fn trash(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Trash).await
}

async fn thread_delete(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::ThreadDelete { thread }).await
}

async fn thread_restore(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::ThreadRestore { thread }).await
}

async fn thread_purge(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::ThreadPurge { thread }).await
}

#[derive(Deserialize)]
struct PathQuery {
    path: String,
}

async fn file_history(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<PathQuery>) -> HttpResponse {
    ask(&web, Request::FileHistory { thread, path: query.path }).await
}

/// Other threads of the thread's repository that changed the path.
async fn in_flight(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<PathQuery>) -> HttpResponse {
    ask_in_clone(&web, &thread, |cwd| Request::InFlight { cwd, path: query.path }).await
}

#[derive(Deserialize)]
struct ThreadSearch {
    q: String,
    #[serde(default)]
    limit: Option<u32>,
}

async fn thread_search(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<ThreadSearch>) -> HttpResponse {
    ask(&web, Request::HistorySearch { thread, query: query.q, limit: query.limit.unwrap_or(50) }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_one_time_code_gives_the_token_once() {
        let (events, _inbox) = mpsc::unbounded_channel();
        let codes = OneTimeCodes::default();
        let info = WebInfo { port: 1, token: "secret".into(), codes: codes.clone() };
        let bus = broadcast::channel(1).0;
        let web = Web { events, bus, token: info.token.clone(), port: 1, codes };
        let url = info.one_time_url();
        assert!(!url.contains("secret"), "{url}");
        let code = url.split("code=").nth(1).expect("a code").to_owned();
        let trade = || exchange(State(web.clone()), Json(Exchange { code: code.clone() }));
        assert_eq!(trade().await.status(), StatusCode::OK);
        assert_eq!(trade().await.status(), StatusCode::UNAUTHORIZED, "a code works once");
    }

    #[test]
    fn tokens_match_only_exactly() {
        assert!(same_token("abc123", "abc123"));
        assert!(!same_token("abc124", "abc123"));
        assert!(!same_token("abc12", "abc123"), "a prefix is not the token");
        assert!(!same_token("abc1234", "abc123"));
        assert!(!same_token("", "abc123"));
    }
}
