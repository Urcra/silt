//! What the hosted web app asks of an agent (ADR 0026): relayed as a command
//! to the daemon whose reports say it runs the thread's agent, and answered
//! with the daemon's answer, at the same paths as the daemon's own web API.

use std::time::Duration;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use silt_proto::{AgentReport, ErrorReply, RemoteCommand};
use silt_types::{DeviceId, ThreadId};

use crate::app::Browser;
use crate::control::{self, Relay};
use crate::{ApiError, Server, UserId, id_bytes, id_from};

/// How long the browser waits for the daemon to answer.
const ANSWER_WAIT: Duration = Duration::from_secs(30);
/// How long a prompt left for an offline device waits for it.
const QUEUED_WAIT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub(crate) fn routes() -> Router<Server> {
    Router::new()
        .route("/app/api/threads/{id}/live", get(live))
        .route("/app/api/threads/{id}/agent/prompt", post(prompt))
        .route("/app/api/threads/{id}/agent/steer", post(steer))
        .route("/app/api/threads/{id}/agent/cancel", post(cancel))
        .route("/app/api/threads/{id}/agent/answer", post(answer))
        .route("/app/api/threads/{id}/agent/stop", post(stop))
        .route("/app/api/threads/{id}/agent/preset", post(preset))
        .route("/app/api/threads/{id}/agent/mode", post(mode))
        .route("/app/api/threads/{id}/agent/setting", post(setting))
        .route("/app/api/threads/{id}/queue", post(enqueue))
}

/// A refusal as the web app reads one.
pub(crate) struct Refused(StatusCode, String);

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
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

/// Relays `command` and answers with what the daemon answers. A prompt for
/// an offline device waits for it when `when_back` says so.
async fn relayed(
    server: &Server,
    browser: Browser,
    selector: &str,
    command: RemoteCommand,
    when_back: bool,
) -> Result<Json<Value>, Refused> {
    let thread = thread_for(server, browser.user, selector).await?;
    let (device, name, online, _) = runner(server, browser.user, thread).await?;
    let queues = when_back && matches!(command, RemoteCommand::Prompt { .. } | RemoteCommand::Queue { .. });
    if !online && !queues {
        return Err(Refused(StatusCode::CONFLICT, format!("{name} is offline")));
    }
    let ttl = if online { ANSWER_WAIT } else { QUEUED_WAIT };
    let id = control::relay(server, Relay { device, thread, sender: browser.user, from: None, command, ttl }).await?;
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
            "error": null,
            "modes": null,
            "settings": [],
            "takes_images": false,
            "takes_files": false,
        })
    });
    Ok(Json(json!({ "deltas": 0, "agent": agent, "preset": preset.unwrap_or_else(|| "ask_before_commands".into()) })))
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
