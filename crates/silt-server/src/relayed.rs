//! What the hosted web app asks of an agent (ADR 0026): relayed as a command
//! to the daemon whose reports say it runs the thread's agent, and answered
//! with the daemon's answer, at the same paths as the daemon's own web API.

use std::time::Duration;

use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use silt_proto::{AgentReport, Capabilities, ErrorReply, RelayRequest, RemoteCommand};
use silt_types::{DeviceId, ThreadId};

use crate::app::Browser;
use crate::control::{self, Relay};
use crate::{ApiError, Caller, Server, UserId, id_bytes, id_from};

/// How long the browser waits for the daemon to answer.
const ANSWER_WAIT: Duration = Duration::from_secs(30);
/// How long a page's read of the thread's files waits on a device before the server reads them itself.
const FILES_WAIT: Duration = Duration::from_secs(8);
/// How long a page's question waits for an agent's answer (ADR 0072).
const SAMPLE_WAIT: Duration = Duration::from_secs(120);
/// How long a prompt left for an offline device waits for it.
const QUEUED_WAIT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub(crate) fn routes() -> Router<Server> {
    Router::new()
        .route("/app/api/threads/{id}/live", get(live))
        .route("/app/api/threads/{id}/agent/prompt", post(prompt))
        .route("/app/api/threads/{id}/agent/steer", post(steer))
        .route("/app/api/threads/{id}/agent/cancel", post(cancel))
        .route("/app/api/threads/{id}/agent/answer", post(answer))
        .route("/app/api/threads/{id}/agent/reply", post(reply))
        .route("/app/api/threads/{id}/agent/stop", post(stop))
        .route("/app/api/threads/{id}/agent/preset", post(preset))
        .route("/app/api/threads/{id}/agent/mode", post(mode))
        .route("/app/api/threads/{id}/agent/setting", post(setting))
        .route("/app/api/threads/{id}/agent/sign-in", post(sign_in))
        .route("/app/api/threads/{id}/agent/sign-in/code", post(sign_in_code))
        .route("/app/api/threads/{id}/agent/sign-in/cancel", post(sign_in_cancel))
        .route("/app/api/threads/{id}/queue", post(enqueue))
        .route("/app/api/threads/{id}/continue", post(continue_on))
        .route("/app/api/mrs/{id}", get(mr_details))
        .route("/app/api/mrs/{id}/seen", axum::routing::put(mr_seen))
        .route("/app/api/threads/{id}/files/at", get(file_at))
        .route("/app/api/threads/{id}/artifacts/{artifact}/sample", post(sample))
        .route("/app/api/threads/{id}/artifacts/{artifact}/sample/{ask}", get(sample_so_far))
        .route("/app/api/threads/{id}/artifacts/{artifact}/sample/{ask}/tools/{call}", post(sample_tool_result))
        .route("/app/api/threads/{id}/artifacts/{artifact}/files", post(page_files))
        .route("/app/api/threads/{id}/artifacts/{artifact}/mcp", post(page_mcp))
}

/// A file at a version for a permalink, from one of the person's devices,
/// as the server has the thread's edits but not the git texts they start
/// from (ADR 0031).
async fn file_at(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Query(query): Query<silt_proto::view::FileQuery>,
) -> Result<Json<Value>, Refused> {
    let thread = thread_for(&server, browser.user, &id).await?;
    let command = RemoteCommand::FileAt { path: query.path, at: query.at };
    Ok(Json(ask_any_device(&server, browser, thread, command).await?))
}

#[derive(Deserialize)]
struct SampleAsk {
    /// The page's name for the question, under which what's said so far
    /// is read while it streams.
    #[serde(default)]
    ask: Option<String>,
    input: Value,
    #[serde(default)]
    images: Vec<silt_proto::artifact::SampleImage>,
    #[serde(default)]
    model: silt_proto::artifact::SampleModel,
    #[serde(default)]
    tools: Vec<silt_proto::artifact::SampleTool>,
}

/// A page's question, answered by the agent of one of the person's own
/// online computers (ADR 0072), as the server runs no agent.
async fn sample(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(asked): Json<SampleAsk>,
) -> Result<Json<Value>, Refused> {
    let thread = thread_for(&server, browser.user, &id).await?;
    let latest = crate::browse::readable_artifact(&server, browser.user, &id, &artifact, None).await?;
    if !latest.capabilities.as_ref().is_some_and(|declared| declared.contains_key("sample")) {
        return Err(Refused(StatusCode::BAD_REQUEST, format!("artifact {artifact} doesn't declare sample")));
    }
    consented(&server, browser.user, &id, &artifact, "sample").await?;
    let question = silt_proto::artifact::sample_question(&asked.input).map_err(|why| Refused(StatusCode::BAD_REQUEST, why))?;
    silt_proto::artifact::sample_images(&asked.images).map_err(|why| Refused(StatusCode::BAD_REQUEST, why))?;
    let declared = latest.capabilities.clone().unwrap_or_default();
    if !asked.images.is_empty() && !silt_proto::artifact::sample_takes_images(&declared) {
        return Err(Refused(StatusCode::BAD_REQUEST, format!("artifact {artifact} doesn't declare sample with images")));
    }
    asked.model.resolved().map_err(|why| Refused(StatusCode::BAD_REQUEST, why))?;
    silt_proto::artifact::sample_tools(&asked.tools).map_err(|why| Refused(StatusCode::BAD_REQUEST, why))?;
    if !asked.tools.is_empty() && asked.ask.is_none() {
        return Err(Refused(StatusCode::BAD_REQUEST, "a question with tools has a name, for its calls to reach the page".into()));
    }
    if !server.shared.control.may_sample(browser.user) {
        return Err(Refused(
            StatusCode::TOO_MANY_REQUESTS,
            format!("rate limited: your pages ask at most {} questions a minute", crate::control::SAMPLES_PER_MINUTE),
        ));
    }
    let command = RemoteCommand::Sample { question, images: asked.images, model: asked.model, tools: asked.tools };
    let ask =
        asked.ask.filter(|ask| (1..=64).contains(&ask.len()) && ask.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    let watched = ask.clone().map(|ask| (browser.user, ask));
    // Forgotten however this ends, a browser that went away included.
    let _forget = ask.map(|ask| Forget { server: server.clone(), user: browser.user, ask });
    let answered = ask_device_watched(&server, browser, thread, command, SAMPLE_WAIT, watched).await;
    Ok(Json(answered?))
}

/// What the agent has said so far of a page's question named `ask`, while
/// a device answers it.
async fn sample_so_far(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((_, _, ask)): Path<(String, String, String)>,
) -> Result<Json<Value>, Refused> {
    let text = server.shared.control.said_so_far(browser.user, &ask).unwrap_or_default();
    let calls = server.shared.control.calls_waiting(browser.user, &ask);
    Ok(Json(serde_json::json!({ "text": text, "calls": calls })))
}

/// The page's answer to the agent's call of one of its tools, which the
/// device answering the question waits for.
async fn sample_tool_result(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((_, _, ask, call)): Path<(String, String, String, u32)>,
    Json(result): Json<Value>,
) -> Result<StatusCode, Refused> {
    if server.shared.control.answer_call(browser.user, &ask, call, result) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(Refused(StatusCode::NOT_FOUND, "no such call is waiting".into()))
    }
}

/// The thread's files for a page, read by one of the person's devices that
/// holds the family (ADR 0078), or, when none answers, folded here from the
/// family's deltas and its git mirror.
async fn page_files(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(asked): Json<silt_proto::page::FilesRequest>,
) -> Result<Json<Value>, Refused> {
    let thread = thread_for(&server, browser.user, &id).await?;
    let latest = crate::browse::readable_artifact(&server, browser.user, &id, &artifact, None).await?;
    if !latest.capabilities.as_ref().is_some_and(|declared| declared.contains_key("files")) {
        return Err(Refused(StatusCode::BAD_REQUEST, format!("artifact {artifact} doesn't declare files")));
    }
    consented(&server, browser.user, &id, &artifact, "files").await?;
    let config = latest.capabilities.as_ref().and_then(|declared| declared.get("files")).cloned().unwrap_or_default();
    // A device reads its own files at once, so one that's slow to answer has likely gone, and the server answers instead.
    match ask_device_within(&server, browser, thread, RemoteCommand::PageFiles { artifact, asked: asked.clone() }, FILES_WAIT)
        .await
    {
        Err(Refused(StatusCode::CONFLICT, _)) => {
            Ok(Json(crate::browse::projected_files(&server, browser.user, &id, config, asked).await?))
        }
        answered => Ok(Json(answered?)),
    }
}

/// A page's call of the person's page connectors, run by one of their own
/// online computers, where the connectors are (ADR 0080).
async fn page_mcp(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, artifact)): Path<(String, String)>,
    Json(asked): Json<Value>,
) -> Result<Json<Value>, Refused> {
    let thread = thread_for(&server, browser.user, &id).await?;
    let latest = crate::browse::readable_artifact(&server, browser.user, &id, &artifact, None).await?;
    if !latest.capabilities.as_ref().is_some_and(|declared| declared.contains_key("mcp")) {
        return Err(Refused(StatusCode::BAD_REQUEST, format!("artifact {artifact} doesn't declare mcp")));
    }
    consented(&server, browser.user, &id, &artifact, "mcp").await?;
    Ok(Json(ask_device_within(&server, browser, thread, RemoteCommand::PageMcp { artifact, asked }, SAMPLE_WAIT).await?))
}

/// Forgets a named question when its request ends.
struct Forget {
    server: Server,
    user: UserId,
    ask: String,
}

impl Drop for Forget {
    fn drop(&mut self) {
        self.server.shared.control.forget_ask(self.user, &self.ask);
    }
}

/// Refuses a page's call of `capability` unless the person allowed what the
/// artifact's latest version declares of it. The page holding the frame
/// asks them; this stands behind it (ADR 0072).
async fn consented(server: &Server, user: UserId, id: &str, artifact: &str, capability: &str) -> Result<(), Refused> {
    let grants = crate::browse::grants_held(server, user, id, artifact).await?;
    if grants.get(capability).is_some_and(|state| state == "granted") {
        return Ok(());
    }
    Err(Refused(StatusCode::FORBIDDEN, format!("not_granted: the person hasn't allowed this page {capability}")))
}

/// How long one device's view of a merge request stands in for asking
/// again, as the hosted app fetches every few seconds.
const MR_VIEW_FOR: Duration = Duration::from_secs(15);

/// Has one of the person's online devices answer `command`, trying each
/// until one can: any that holds the family can.
async fn ask_any_device(server: &Server, browser: Browser, thread: ThreadId, command: RemoteCommand) -> Result<Value, Refused> {
    ask_device_within(server, browser, thread, command, ANSWER_WAIT).await
}

async fn ask_device_within(
    server: &Server,
    browser: Browser,
    thread: ThreadId,
    command: RemoteCommand,
    wait: Duration,
) -> Result<Value, Refused> {
    ask_device_watched(server, browser, thread, command, wait, None).await
}

/// [`ask_device_within`], noting each command relayed as the one answering
/// `watched`, a person's named question, so what's said so far can be read.
async fn ask_device_watched(
    server: &Server,
    browser: Browser,
    thread: ThreadId,
    command: RemoteCommand,
    wait: Duration,
    watched: Option<(crate::UserId, String)>,
) -> Result<Value, Refused> {
    if let Err(needs) = browser.capabilities.allow(&command) {
        return Err(Refused(StatusCode::FORBIDDEN, format!("this phone may not {needs}; change that on the Devices page")));
    }
    let mut last = "none of your devices is online to show the changes".to_owned();
    for device in crate::accounts::devices_of(server.db(), browser.user).await? {
        if !server.shared.control.heard(device.id).await.0 {
            continue;
        }
        let relay = Relay {
            device: device.id,
            thread,
            sender: browser.user,
            from: None,
            capabilities: browser.capabilities,
            command: command.clone(),
            ttl: wait,
        };
        let id = control::relay(server, relay).await?;
        if let Some((user, ask)) = &watched {
            server.shared.control.asked(*user, ask.clone(), id);
        }
        match control::answer_of(server, id, wait).await? {
            Some(answer) if answer.ok => return Ok(answer.data),
            Some(answer) => last = answer.message,
            None => last = format!("{} didn't answer", device.name),
        }
    }
    Err(Refused(StatusCode::CONFLICT, last))
}

/// Whose view of which merge request, with git's rewrites left out or not
/// (ADR 0069).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MrView {
    user: crate::UserId,
    mr: String,
    hide_git: bool,
}

/// A merge request as the server holds it, with its files and diff from
/// one of the person's devices when one is online (ADR 0026).
async fn mr_details(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Query(query): Query<silt_proto::view::GitQuery>,
) -> Result<Json<Value>, Refused> {
    let hide_git = query.hide_git();
    let Json(own) = crate::browse::merge_request(State(server.clone()), Extension(browser), Path(id.clone())).await?;
    let mut details = serde_json::to_value(&own).map_err(crate::ServerError::from)?;
    if !browser.capabilities.review {
        return Ok(Json(details));
    }
    let (_, request, _) = crate::browse::seen_request(&server, browser.user, &id).await?;
    let key = MrView { user: browser.user, mr: request.id.to_string(), hide_git };
    let cached =
        server.shared.mr_views.lock().await.get(&key).filter(|(at, _)| at.elapsed() < MR_VIEW_FOR).map(|(_, view)| view.clone());
    let view = match cached {
        Some(view) => Some(view),
        None => {
            let asked =
                ask_any_device(&server, browser, request.target, RemoteCommand::MrShow { mr: request.id.to_string(), hide_git })
                    .await;
            let view = asked.ok();
            if let Some(view) = &view {
                server.shared.mr_views.lock().await.insert(key, (std::time::Instant::now(), view.clone()));
            }
            view
        }
    };
    if let Some(view) = view {
        for field in ["diff", "conflicts", "files"] {
            details[field] = view[field].clone();
        }
    }
    Ok(Json(details))
}

#[derive(Deserialize)]
struct SeenBody {
    path: String,
    seen: bool,
}

/// Marks a file seen through a device, which knows the file as it is now.
async fn mr_seen(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<SeenBody>,
) -> Result<Json<Value>, Refused> {
    let (_, request, _) = crate::browse::seen_request(&server, browser.user, &id).await?;
    let command = RemoteCommand::MrSeen { mr: request.id.to_string(), path: body.path, seen: body.seen };
    let answer = ask_any_device(&server, browser, request.target, command).await?;
    let mut views = server.shared.mr_views.lock().await;
    for hide_git in [false, true] {
        views.remove(&MrView { user: browser.user, mr: request.id.to_string(), hide_git });
    }
    drop(views);
    Ok(Json(answer))
}

/// For a daemon passing on what its own web UI asks of an agent running on
/// another of the person's devices.
pub(crate) fn device_routes() -> Router<Server> {
    Router::new().route("/v1/threads/{id}/relay", post(device_relay))
}

impl From<Refused> for ApiError {
    fn from(Refused(status, message): Refused) -> Self {
        ApiError(status, ErrorReply::Invalid { message })
    }
}

async fn device_relay(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    Json(body): Json<RelayRequest>,
) -> Result<Json<Value>, ApiError> {
    let capabilities = crate::devices::capabilities_of(server.db(), caller.device).await?;
    Ok(relay_from(&server, caller.user, Some(caller.device), capabilities, &id, body.command, body.when_back).await?)
}

/// A refusal as the web app reads one.
pub(crate) struct Refused(StatusCode, String);

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        let refusal = silt_proto::refusal_of(&self.1).or((self.0 == StatusCode::FORBIDDEN).then_some("not_granted"));
        (self.0, Json(json!({ "error": self.1, "refusal": refusal }))).into_response()
    }
}

impl From<ApiError> for Refused {
    fn from(ApiError(status, reply): ApiError) -> Self {
        let message = match reply {
            ErrorReply::Invalid { message } | ErrorReply::Forbidden { message } => message,
            ErrorReply::NotFound => "no such thread".into(),
            _ => "internal error".into(),
        };
        Self(status, message)
    }
}

impl From<crate::ServerError> for Refused {
    fn from(error: crate::ServerError) -> Self {
        ApiError::from(error).into()
    }
}

impl From<sqlx::Error> for Refused {
    fn from(error: sqlx::Error) -> Self {
        crate::ServerError::from(error).into()
    }
}

/// The thread `selector` names among those the person sees.
async fn thread_for(server: &Server, user: UserId, selector: &str) -> Result<ThreadId, Refused> {
    let viewer = server.viewer(user).await?;
    let ids = sqlx::query_scalar!("SELECT id FROM threads WHERE deleted_ms IS NULL").fetch_all(server.db()).await?;
    let found: Vec<ThreadId> = ids
        .iter()
        .map(|id| ThreadId(id_from(id)))
        .filter(|thread| thread.to_string().ends_with(selector) && viewer.sees_thread(*thread))
        .collect();
    match found.as_slice() {
        [thread] => Ok(*thread),
        [] => Err(Refused(StatusCode::NOT_FOUND, "no such thread".into())),
        _ => Err(Refused(StatusCode::BAD_REQUEST, "that names several threads".into())),
    }
}

/// The device running the thread's agent, if it's the person's own: only
/// they may run things on their machine (ADR 0026). The daemon checks too.
async fn runner(server: &Server, user: UserId, thread: ThreadId) -> Result<(DeviceId, String, bool, AgentReport), Refused> {
    let Some((device, online, report)) = server.shared.control.runs(thread).await else {
        return Err(Refused(StatusCode::CONFLICT, "no device runs an agent in this thread".into()));
    };
    let id = id_bytes(device.0);
    let owner = sqlx::query!(
        "SELECT d.name, d.user_id, u.name AS owner FROM devices d JOIN users u ON u.id = d.user_id WHERE d.id = ?",
        id
    )
    .fetch_optional(server.db())
    .await?
    .ok_or_else(|| Refused(StatusCode::CONFLICT, "the device running this thread's agent was signed out".into()))?;
    if owner.user_id != user.0 {
        return Err(Refused(
            StatusCode::FORBIDDEN,
            format!("{}'s agent runs on their machine; only they can direct it", owner.owner),
        ));
    }
    Ok((device, owner.name, online, report))
}

async fn relayed(
    server: &Server,
    browser: Browser,
    selector: &str,
    command: RemoteCommand,
    when_back: bool,
) -> Result<Json<Value>, Refused> {
    relay_from(server, browser.user, None, browser.capabilities, selector, command, when_back).await
}

/// Relays `command` from `user`, on their device `from` if not a browser,
/// and answers with what the daemon answers. A prompt for an offline device
/// waits for it when `when_back` says so.
async fn relay_from(
    server: &Server,
    user: UserId,
    from: Option<DeviceId>,
    capabilities: Capabilities,
    selector: &str,
    command: RemoteCommand,
    when_back: bool,
) -> Result<Json<Value>, Refused> {
    if let Err(needs) = capabilities.allow(&command) {
        return Err(Refused(StatusCode::FORBIDDEN, format!("this device may not {needs}; change that on the Devices page")));
    }
    let thread = thread_for(server, user, selector).await?;
    let (device, name, online, _) = runner(server, user, thread).await?;
    if from == Some(device) {
        return Err(Refused(StatusCode::CONFLICT, "the agent runs on this device".into()));
    }
    let queues = when_back && matches!(command, RemoteCommand::Prompt { .. } | RemoteCommand::Queue { .. });
    if !online && !queues {
        return Err(Refused(StatusCode::CONFLICT, format!("{name} is offline")));
    }
    let ttl = if online { ANSWER_WAIT } else { QUEUED_WAIT };
    let id = control::relay(server, Relay { device, thread, sender: user, from, capabilities, command, ttl }).await?;
    if !online {
        return Ok(Json(json!(format!("queued until {name} is back"))));
    }
    match control::answer_of(server, id, ANSWER_WAIT).await? {
        Some(answer) if answer.ok => Ok(Json(answer.data)),
        Some(answer) => Err(Refused(StatusCode::CONFLICT, answer.message)),
        None => Err(Refused(StatusCode::GATEWAY_TIMEOUT, format!("{name} didn't answer"))),
    }
}

/// How the thread's agent stands as its device last reported, shaped as a
/// daemon's live state.
async fn live(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Refused> {
    let thread = thread_for(&server, browser.user, &id).await?;
    let report = server.shared.control.runs(thread).await.filter(|(_, online, _)| *online).map(|(_, _, report)| report);
    let preset = report.as_ref().map(|report| report.preset.clone()).filter(|preset| !preset.is_empty());
    let agent = report.map(|report| {
        json!({
            "agent": report.agent,
            "state": report.state,
            "streaming": "",
            "thinking": "",
            "permissions": report.permissions,
            "questions": report.questions,
            "sign_in": report.sign_in,
            "context": null,
            "error": null,
            "modes": null,
            "settings": [],
            "takes_images": false,
            "takes_files": false,
            "commands": [],
            "stoppable": [],
        })
    });
    Ok(Json(json!({ "deltas": 0, "agent": agent, "preset": preset.unwrap_or_else(|| "ask_before_commands".into()) })))
}

#[derive(Deserialize)]
struct ContinueBody {
    device: String,
    agent: String,
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    mode: Option<String>,
}

/// Starts an agent in the thread on the person's chosen device, which takes
/// the thread over from wherever it ran: its checkout there, and the
/// conversation so far for the agent to go on from.
async fn continue_on(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<ContinueBody>,
) -> Result<Json<Value>, Refused> {
    let thread = thread_for(&server, browser.user, &id).await?;
    let (device, name) = crate::devices::own_device(&server, browser.user, &body.device).await?;
    if let Some((running_on, true, report)) = server.shared.control.runs(thread).await
        && running_on != device
        && report.state.running()
    {
        return Err(Refused(
            StatusCode::CONFLICT,
            format!("{} runs in this thread elsewhere; stop it there first", report.agent),
        ));
    }
    if !server.shared.control.heard(device).await.0 {
        return Err(Refused(StatusCode::CONFLICT, format!("{name} is offline")));
    }
    let command = RemoteCommand::Start { agent: body.agent, preset: body.preset, mode: body.mode };
    if let Err(needs) = browser.capabilities.allow(&command) {
        return Err(Refused(StatusCode::FORBIDDEN, format!("this phone may not {needs}; change that on the Devices page")));
    }
    let relay =
        Relay { device, thread, sender: browser.user, from: None, capabilities: browser.capabilities, command, ttl: ANSWER_WAIT };
    let id = control::relay(&server, relay).await?;
    match control::answer_of(&server, id, ANSWER_WAIT).await? {
        Some(answer) if answer.ok => Ok(Json(answer.data)),
        Some(answer) => Err(Refused(StatusCode::CONFLICT, answer.message)),
        None => Err(Refused(StatusCode::GATEWAY_TIMEOUT, format!("{name} didn't answer"))),
    }
}

#[derive(Deserialize)]
struct PromptBody {
    text: String,
    #[serde(default)]
    plan: bool,
    #[serde(default)]
    attachments: Vec<Value>,
    #[serde(default)]
    when_back: bool,
}

async fn prompt(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<PromptBody>,
) -> Result<Json<Value>, Refused> {
    if !body.attachments.is_empty() {
        return Err(Refused(StatusCode::BAD_REQUEST, "attach files from the machine running the agent".into()));
    }
    relayed(&server, browser, &id, RemoteCommand::Prompt { text: body.text, plan: body.plan }, body.when_back).await
}

#[derive(Deserialize)]
struct TextBody {
    text: String,
    #[serde(default)]
    when_back: bool,
}

async fn steer(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<TextBody>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::Steer { text: body.text }, false).await
}

async fn enqueue(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<TextBody>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::Queue { text: body.text }, body.when_back).await
}

async fn cancel(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::Cancel, false).await
}

async fn stop(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::Stop, false).await
}

#[derive(Deserialize)]
struct AnswerBody {
    id: u64,
    #[serde(default)]
    option: Option<String>,
}

async fn answer(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<AnswerBody>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::Answer { id: body.id, option: body.option }, false).await
}

#[derive(Deserialize)]
struct ReplyBody {
    id: u64,
    /// Each field's chosen values; none declines the question.
    #[serde(default)]
    answers: Option<std::collections::BTreeMap<String, Vec<String>>>,
}

async fn reply(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<ReplyBody>,
) -> Result<Json<Value>, Refused> {
    let answers = body.answers.map(|answers| answers.into_iter().collect());
    relayed(&server, browser, &id, RemoteCommand::Reply { id: body.id, answers }, false).await
}

#[derive(Deserialize)]
struct PresetBody {
    preset: String,
}

async fn preset(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<PresetBody>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::Preset { preset: body.preset }, false).await
}

#[derive(Deserialize)]
struct ModeBody {
    mode: String,
}

async fn mode(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<ModeBody>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::Mode { mode: body.mode }, false).await
}

#[derive(Deserialize)]
struct SettingBody {
    id: String,
    value: String,
}

async fn setting(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<SettingBody>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::Setting { id: body.id, value: body.value }, false).await
}

#[derive(Deserialize)]
struct SignInBody {
    method: String,
}

/// Signs the thread's agent in on its device (ADR 0056).
async fn sign_in(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<SignInBody>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::SignIn { method: body.method }, false).await
}

#[derive(Deserialize)]
struct SignInCodeBody {
    code: String,
}

async fn sign_in_code(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<SignInCodeBody>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::SignInCode { code: body.code }, false).await
}

async fn sign_in_cancel(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Refused> {
    relayed(&server, browser, &id, RemoteCommand::SignInCancel, false).await
}
