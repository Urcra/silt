//! Each daemon's control channel (ADR 0026): the commands people send its
//! threads from elsewhere, which it long-polls for and answers, and its
//! reports of the agents it runs, kept in memory for browsers to show.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use silt_proto::{
    AgentReport, AgentState, AgentsReport, Capabilities, CommandAnswer, ControlCommand, ControlReply, InboxAction, RemoteCommand,
    RunningElsewhere, Startable,
};
use silt_types::{DeviceId, ThreadId};
use tokio::sync::{Mutex, Notify};

use crate::{ApiError, Caller, MAX_WAIT, Server, ServerError, UserId, id_bytes, id_from, not_found, now_ms};

/// A device not heard from for this long is offline.
const OFFLINE_AFTER: Duration = Duration::from_secs(90);
/// Most agents one report may list.
const MAX_AGENTS: usize = 256;

#[derive(Default)]
pub(crate) struct Control {
    board: Mutex<Board>,
    /// Woken when a command is relayed, for the polls to look again.
    relayed: Notify,
    /// Woken when a command is answered, for whoever waits on one.
    answered: Notify,
    /// What a device's agent has said so far of a page's question it
    /// answers, by command, for the hosted app to show as it streams.
    progress: std::sync::Mutex<HashMap<i64, String>>,
    /// Which command answers a browser's question, by its person and the
    /// name the page gave it.
    asks: std::sync::Mutex<HashMap<(UserId, String), i64>>,
    /// The device each such command went to, for stopping it there.
    asked_of: std::sync::Mutex<HashMap<i64, silt_types::DeviceId>>,
    /// Calls of a page's tools a device's agent made, waiting for the page,
    /// by command, and where each one's answer goes.
    calls: std::sync::Mutex<HashMap<i64, Vec<ToolCall>>>,
    answers: std::sync::Mutex<HashMap<(i64, u32), tokio::sync::oneshot::Sender<serde_json::Value>>>,
    /// When each person's pages last asked, for the server's own rate.
    sampled: std::sync::Mutex<HashMap<UserId, Vec<std::time::Instant>>>,
}

/// The most page questions one person's pages relay a minute, across
/// their devices; each device limits its own too.
pub(crate) const SAMPLES_PER_MINUTE: usize = 8;

/// A call of one of a page's tools, as the page hears of it.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub(crate) struct ToolCall {
    call: u32,
    name: String,
    input: serde_json::Value,
}

/// How long a page has to answer a call of one of its tools.
const TOOL_WAIT: Duration = Duration::from_secs(60);

/// The most of an answer kept as it streams, in bytes, as print mode keeps.
const PROGRESS_BYTES: usize = 256 * 1024;

impl Control {
    /// Counts a page question `user` relays, refused past the rate.
    pub(crate) fn may_sample(&self, user: UserId) -> bool {
        let now = std::time::Instant::now();
        let mut sampled = self.sampled.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let recent = sampled.entry(user).or_default();
        recent.retain(|at| now.duration_since(*at) < std::time::Duration::from_secs(60));
        if recent.len() >= SAMPLES_PER_MINUTE {
            return false;
        }
        recent.push(now);
        true
    }

    /// Notes that command `id` answers `user`'s question named `ask`.
    pub(crate) fn asked(&self, user: UserId, ask: String, id: i64, device: silt_types::DeviceId) {
        self.asks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert((user, ask), id);
        self.asked_of.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id, device);
    }

    /// The command answering the person's question named `ask`, and the
    /// device it went to, while it's answered.
    pub(crate) fn asked_on(&self, user: UserId, ask: &str) -> Option<(i64, silt_types::DeviceId)> {
        let id = self.ask_id(user, ask)?;
        let device = *self.asked_of.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&id)?;
        Some((id, device))
    }

    /// Forgets the question, once it's answered.
    pub(crate) fn forget_ask(&self, user: UserId, ask: &str) {
        let id = self.asks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&(user, ask.to_owned()));
        if let Some(id) = id {
            self.asked_of.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id);
            self.progress.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id);
            self.calls.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id);
            self.answers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|(of, _), _| *of != id);
        }
    }

    fn ask_id(&self, user: UserId, ask: &str) -> Option<i64> {
        self.asks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&(user, ask.to_owned())).copied()
    }

    /// The calls of the page's tools waiting for `user`'s page to run them.
    pub(crate) fn calls_waiting(&self, user: UserId, ask: &str) -> Vec<ToolCall> {
        let Some(id) = self.ask_id(user, ask) else { return Vec::new() };
        self.calls.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&id).cloned().unwrap_or_default()
    }

    /// The page's answer to call `call` of its question `ask`; false when no
    /// such call waits.
    pub(crate) fn answer_call(&self, user: UserId, ask: &str, call: u32, result: serde_json::Value) -> bool {
        let Some(id) = self.ask_id(user, ask) else { return false };
        if let Some(calls) = self.calls.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_mut(&id) {
            calls.retain(|waiting| waiting.call != call);
        }
        let answer = self.answers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&(id, call));
        answer.is_some_and(|answer| answer.send(result).is_ok())
    }

    /// What's been said so far of `user`'s question named `ask`.
    pub(crate) fn said_so_far(&self, user: UserId, ask: &str) -> Option<String> {
        let id = *self.asks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&(user, ask.to_owned()))?;
        self.progress.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&id).cloned()
    }
}

#[derive(Deserialize)]
pub(crate) struct Progress {
    text: String,
}

/// A call of one of a page's tools, from the device whose agent answers
/// the page's question; answered once the page runs it, within a minute.
pub(crate) async fn tool_call(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<i64>,
    Json(call): Json<ToolCall>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let device = id_bytes(caller.device.0);
    // A runtime query, as it's read only here.
    let open: Option<i64> =
        sqlx::query_scalar("SELECT id FROM control_commands WHERE id = ? AND device_id = ? AND answered_ms IS NULL")
            .bind(id)
            .bind(device)
            .fetch_optional(server.db())
            .await
            .map_err(ServerError::from)?;
    if open.is_none() {
        return Err(not_found());
    }
    let control = &server.shared.control;
    let (answer, answered) = tokio::sync::oneshot::channel();
    control.answers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert((id, call.call), answer);
    let number = call.call;
    control.calls.lock().unwrap_or_else(std::sync::PoisonError::into_inner).entry(id).or_default().push(call);
    let heard = tokio::time::timeout(TOOL_WAIT, answered).await;
    if let Some(calls) = control.calls.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_mut(&id) {
        calls.retain(|waiting| waiting.call != number);
    }
    control.answers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&(id, number));
    match heard {
        Ok(Ok(result)) => Ok(Json(result)),
        _ => Ok(Json(serde_json::json!({"ok": false, "error": "the page didn't answer in time"}))),
    }
}

/// What a device's agent has said so far of a command it hasn't answered
/// yet; only the device it's for tells it.
pub(crate) async fn progress(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<i64>,
    Json(said): Json<Progress>,
) -> Result<StatusCode, ApiError> {
    let device = id_bytes(caller.device.0);
    // A runtime query, as it's read only here.
    let open: Option<i64> =
        sqlx::query_scalar("SELECT id FROM control_commands WHERE id = ? AND device_id = ? AND answered_ms IS NULL")
            .bind(id)
            .bind(device)
            .fetch_optional(server.db())
            .await
            .map_err(ServerError::from)?;
    if open.is_none() {
        return Err(not_found());
    }
    let mut text = said.text;
    if text.len() > PROGRESS_BYTES {
        let mut end = PROGRESS_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    // Kept only for a question a page is reading as it streams, which
    // forgets it once answered.
    let control = &server.shared.control;
    if control.asks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).values().any(|asked| *asked == id) {
        control.progress.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id, text);
    }
    Ok(StatusCode::NO_CONTENT)
}

struct Heard {
    user: UserId,
    agents: Vec<AgentReport>,
    startable: Vec<Startable>,
    at: Instant,
    /// Whether it reported since the server started, so a change to what
    /// it runs can be told from what it ran all along.
    reported: bool,
    /// Its person heard it went offline with work running.
    offline_noted: bool,
}

/// What each device last reported, by device.
#[derive(Default)]
struct Board {
    devices: HashMap<DeviceId, Heard>,
}

impl Board {
    /// Notes the device was heard from, with its report if it sent one, and
    /// gives the report before, if there was one.
    fn hear(&mut self, device: DeviceId, user: UserId, report: Option<AgentsReport>) -> Option<Vec<AgentReport>> {
        let heard = self.devices.entry(device).or_insert_with(|| Heard {
            user,
            agents: Vec::new(),
            startable: Vec::new(),
            at: Instant::now(),
            reported: false,
            offline_noted: false,
        });
        heard.at = Instant::now();
        heard.offline_noted = false;
        let report = report?;
        let before = heard.reported.then(|| std::mem::take(&mut heard.agents));
        heard.agents = report.agents;
        heard.startable = report.startable;
        heard.reported = true;
        before
    }

    fn online(&self) -> impl Iterator<Item = (&DeviceId, &Heard)> {
        self.devices.iter().filter(|(_, heard)| heard.at.elapsed() < OFFLINE_AFTER)
    }
}

impl Control {
    #[cfg(test)]
    pub(crate) async fn online(&self, device: DeviceId) -> bool {
        self.board.lock().await.online().any(|(online, _)| *online == device)
    }

    /// Whether `device` is online, and what it last reported.
    pub(crate) async fn heard(&self, device: DeviceId) -> (bool, AgentsReport) {
        let board = self.board.lock().await;
        board.devices.get(&device).map_or_else(
            || (false, AgentsReport::default()),
            |heard| {
                let report = AgentsReport { agents: heard.agents.clone(), startable: heard.startable.clone() };
                (heard.at.elapsed() < OFFLINE_AFTER, report)
            },
        )
    }

    /// The device that last reported running an agent in `thread`, whether
    /// it's still online, and what it reported.
    pub(crate) async fn runs(&self, thread: ThreadId) -> Option<(DeviceId, bool, AgentReport)> {
        let board = self.board.lock().await;
        board
            .devices
            .iter()
            .filter_map(|(device, heard)| {
                heard.agents.iter().find(|agent| agent.thread == thread).map(|agent| (device, heard, agent))
            })
            .max_by_key(|(_, heard, _)| heard.at)
            .map(|(device, heard, agent)| (*device, heard.at.elapsed() < OFFLINE_AFTER, agent.clone()))
    }

    /// The agents running on online devices, by thread, for showing only
    /// with threads the viewer sees.
    pub(crate) async fn running(&self) -> HashMap<ThreadId, (DeviceId, AgentReport)> {
        let board = self.board.lock().await;
        board
            .online()
            .flat_map(|(device, heard)| heard.agents.iter().map(|agent| (agent.thread, (*device, agent.clone()))))
            .collect()
    }
}

/// The agents running on the caller's person's other online devices.
pub(crate) async fn running_elsewhere(server: &Server, caller: Caller) -> Result<Vec<RunningElsewhere>, sqlx::Error> {
    let mut running = Vec::new();
    for device in crate::accounts::devices_of(server.db(), caller.user).await? {
        if device.id == caller.device {
            continue;
        }
        let (online, report) = server.shared.control.heard(device.id).await;
        if online {
            running.extend(report.agents.into_iter().map(|agent| RunningElsewhere {
                device: device.id,
                device_name: device.name.clone(),
                agent,
            }));
        }
    }
    Ok(running)
}

#[derive(Deserialize)]
pub(crate) struct Wait {
    #[serde(default)]
    wait_ms: Option<u64>,
}

/// The commands waiting for the calling device, each taken only once: a
/// daemon that stops before answering one leaves it unanswered rather than
/// run twice.
pub(crate) async fn poll(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Query(wait): Query<Wait>,
) -> Result<Json<ControlReply>, ApiError> {
    let control = &server.shared.control;
    let deadline = Instant::now() + wait.wait_ms.map_or(Duration::ZERO, Duration::from_millis).min(MAX_WAIT);
    loop {
        control.board.lock().await.hear(caller.device, caller.user, None);
        let relayed = control.relayed.notified();
        tokio::pin!(relayed);
        relayed.as_mut().enable();
        let commands = take(&server, caller.device).await?;
        let left = deadline.saturating_duration_since(Instant::now());
        if !commands.is_empty() || left.is_zero() {
            return Ok(Json(ControlReply { commands }));
        }
        let _ = tokio::time::timeout(left, relayed).await;
    }
}

async fn take(server: &Server, device: DeviceId) -> Result<Vec<ControlCommand>, ServerError> {
    let (device, now) = (id_bytes(device.0), now_ms());
    let rows = sqlx::query!(
        r#"UPDATE control_commands SET taken_ms = ?
         WHERE device_id = ? AND taken_ms IS NULL AND expires_ms > ?
         RETURNING id AS "id!", thread_id, from_device, command, created_ms, capabilities,
           (SELECT name FROM users WHERE users.id = sender_id) AS "sender!: String""#,
        now,
        device,
        now
    )
    .fetch_all(server.db())
    .await?;
    let mut commands = Vec::new();
    for row in rows {
        let Ok(command) = serde_json::from_str::<RemoteCommand>(&row.command) else {
            tracing::warn!(id = row.id, "skipping a command this server can't read");
            continue;
        };
        let from = row.from_device.as_deref().map(|device| DeviceId(id_from(device)));
        let capabilities = row.capabilities.and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
        commands.push(ControlCommand {
            id: row.id,
            thread: ThreadId(id_from(&row.thread_id)),
            sender: row.sender,
            from,
            capabilities,
            sent_ms: row.created_ms,
            command,
        });
    }
    // A sign-in's code is kept no longer than it takes to deliver (ADR 0056).
    for command in commands.iter().filter(|command| matches!(command.command, RemoteCommand::SignInCode { .. })) {
        let blank = serde_json::to_string(&RemoteCommand::SignInCode { code: String::new() }).expect("a command");
        sqlx::query("UPDATE control_commands SET command = ? WHERE id = ?")
            .bind(blank)
            .bind(command.id)
            .execute(server.db())
            .await?;
    }
    commands.sort_by_key(|command| command.id);
    Ok(commands)
}

/// Takes the calling device's running agents, limited to threads its
/// person sees.
pub(crate) async fn report(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(report): Json<AgentsReport>,
) -> Result<StatusCode, ApiError> {
    let viewer = server.viewer(caller.user).await?;
    let agents: Vec<AgentReport> =
        report.agents.into_iter().filter(|agent| viewer.sees_thread(agent.thread)).take(MAX_AGENTS).collect();
    let startable = report.startable.into_iter().take(MAX_AGENTS).collect();
    let now = agents.clone();
    let before =
        server.shared.control.board.lock().await.hear(caller.device, caller.user, Some(AgentsReport { agents, startable }));
    if let Some(before) = before {
        let device = device_name(&server, caller.device).await?;
        for change in changes(&before, &now) {
            let note = change.note(caller.user, &device);
            crate::inbox::notify_person(&server, note).await?;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn device_name(server: &Server, device: DeviceId) -> Result<String, sqlx::Error> {
    let id = id_bytes(device.0);
    let name = sqlx::query_scalar!("SELECT name FROM devices WHERE id = ?", id).fetch_optional(server.db()).await?;
    Ok(name.unwrap_or_else(|| "a signed-out device".into()))
}

/// What changed about an agent between two reports that its person hears
/// of (ADR 0028).
#[derive(Debug, PartialEq, Eq)]
enum Change {
    Asks(AgentReport),
    Planned(AgentReport),
    Finished(AgentReport),
    Errored(AgentReport),
}

impl Change {
    fn note<'a>(self, user: UserId, device: &'a str) -> crate::inbox::PersonNote<'a> {
        let (kind, agent, text, action) = match self {
            Self::Asks(agent) => {
                let asked = &agent.permissions[0];
                let text = format!("{} on {device} asks to run: {}", agent.agent, asked.title);
                let action = InboxAction::Permission { id: asked.id, title: asked.title.clone(), options: asked.options.clone() };
                ("permission", agent, text, Some(action))
            }
            Self::Planned(agent) => {
                let text = format!("{} on {device} wrote a plan for you to approve", agent.agent);
                ("plan", agent, text, Some(InboxAction::Plan))
            }
            Self::Finished(agent) => {
                let text = format!("{} on {device} finished its turn", agent.agent);
                ("finished", agent, text, None)
            }
            Self::Errored(agent) => {
                let text = format!("{} on {device} stopped with an error", agent.agent);
                ("errored", agent, text, None)
            }
        };
        crate::inbox::PersonNote { user, kind, thread: agent.thread, actor: device, text, action }
    }
}

/// The changes from `before` to `after`, one device's reports.
fn changes(before: &[AgentReport], after: &[AgentReport]) -> Vec<Change> {
    let mut changes = Vec::new();
    for agent in after {
        let was = before.iter().find(|earlier| earlier.thread == agent.thread);
        let state = was.map(|was| was.state);
        let asked = |report: &AgentReport| report.permissions.first().map(|asked| asked.id);
        match agent.state {
            AgentState::AwaitingPermission if asked(agent).is_some() && was.and_then(asked) != asked(agent) => {
                changes.push(Change::Asks(agent.clone()));
            }
            AgentState::AwaitingInput if state != Some(AgentState::AwaitingInput) => changes.push(Change::Planned(agent.clone())),
            AgentState::Idle
                if agent.queued == 0 && matches!(state, Some(AgentState::Working | AgentState::AwaitingPermission)) =>
            {
                changes.push(Change::Finished(agent.clone()));
            }
            AgentState::Error if state != Some(AgentState::Error) => changes.push(Change::Errored(agent.clone())),
            _ => {}
        }
    }
    changes
}

/// How often devices gone quiet are looked for.
const OFFLINE_CHECK: Duration = Duration::from_secs(15);

impl Server {
    pub(crate) fn watch_devices(&self) {
        let server = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(OFFLINE_CHECK);
            loop {
                interval.tick().await;
                if let Err(error) = note_offline(&server).await {
                    tracing::warn!(%error, "could not note devices gone offline");
                }
            }
        });
    }
}

/// Tells each person whose device went offline with an agent at work
/// there, once per time it goes (ADR 0028).
pub(crate) async fn note_offline(server: &Server) -> Result<(), sqlx::Error> {
    let gone: Vec<(DeviceId, UserId, Vec<AgentReport>)> = {
        let mut board = server.shared.control.board.lock().await;
        board
            .devices
            .iter_mut()
            .filter(|(_, heard)| heard.reported && !heard.offline_noted && heard.at.elapsed() >= OFFLINE_AFTER)
            .filter_map(|(device, heard)| {
                let busy: Vec<AgentReport> = heard
                    .agents
                    .iter()
                    .filter(|agent| {
                        matches!(agent.state, AgentState::Working | AgentState::AwaitingPermission | AgentState::AwaitingInput)
                    })
                    .cloned()
                    .collect();
                heard.offline_noted = true;
                (!busy.is_empty()).then_some((*device, heard.user, busy))
            })
            .collect()
    };
    for (device, user, busy) in gone {
        let name = device_name(server, device).await?;
        for agent in busy {
            let text = format!("{name} went offline while {} was at work there", agent.agent);
            let note = crate::inbox::PersonNote { user, kind: "offline", thread: agent.thread, actor: &name, text, action: None };
            crate::inbox::notify_person(server, note).await?;
        }
    }
    Ok(())
}

pub(crate) async fn answer(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<i64>,
    Json(answer): Json<CommandAnswer>,
) -> Result<StatusCode, ApiError> {
    let (device, now, answer) = (id_bytes(caller.device.0), now_ms(), serde_json::to_string(&answer).map_err(ServerError::from)?);
    let answered = sqlx::query!(
        "UPDATE control_commands SET answered_ms = ?, answer = ? WHERE id = ? AND device_id = ? AND answered_ms IS NULL",
        now,
        answer,
        id,
        device
    )
    .execute(server.db())
    .await?;
    if answered.rows_affected() == 0 {
        return Err(not_found());
    }
    server.shared.control.answered.notify_waiters();
    Ok(StatusCode::NO_CONTENT)
}

/// The command's answer, once its daemon gives one within `wait`.
pub(crate) async fn answer_of(server: &Server, id: i64, wait: Duration) -> Result<Option<CommandAnswer>, ServerError> {
    let deadline = Instant::now() + wait;
    loop {
        let answered = server.shared.control.answered.notified();
        tokio::pin!(answered);
        answered.as_mut().enable();
        let answer = sqlx::query_scalar!("SELECT answer FROM control_commands WHERE id = ?", id).fetch_one(server.db()).await?;
        if let Some(answer) = answer {
            return Ok(Some(serde_json::from_str(&answer)?));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(None);
        }
        let _ = tokio::time::timeout(left, answered).await;
    }
}

/// A command for `device`, from `sender` on the device `from` if any.
pub(crate) struct Relay {
    pub(crate) device: DeviceId,
    pub(crate) thread: ThreadId,
    pub(crate) sender: UserId,
    pub(crate) from: Option<DeviceId>,
    /// What the sending device or session may do, as it stands now.
    pub(crate) capabilities: Capabilities,
    pub(crate) command: RemoteCommand,
    pub(crate) ttl: Duration,
}

/// Leaves the command for its daemon's next poll, and wakes the polls.
pub(crate) async fn relay(server: &Server, relay: Relay) -> Result<i64, ServerError> {
    let (device, thread, from) = (id_bytes(relay.device.0), id_bytes(relay.thread.0), relay.from.map(|from| id_bytes(from.0)));
    let command = serde_json::to_string(&relay.command)?;
    let capabilities = serde_json::to_string(&relay.capabilities)?;
    let now = now_ms();
    let expires = now.saturating_add(i64::try_from(relay.ttl.as_millis()).unwrap_or(i64::MAX));
    let id = sqlx::query_scalar!(
        r#"INSERT INTO control_commands (device_id, thread_id, sender_id, from_device, command, created_ms, expires_ms, capabilities)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id AS "id!""#,
        device,
        thread,
        relay.sender,
        from,
        command,
        now,
        expires,
        capabilities
    )
    .fetch_one(server.db())
    .await?;
    server.shared.control.relayed.notify_waiters();
    Ok(id)
}

#[cfg(test)]
mod tests {
    use silt_proto::{AgentState, LoginReply, PROTOCOL, PROTOCOL_HEADER, PasswordLoginRequest, ThreadInfo};

    use super::*;
    use crate::{Admin, Config};

    const PASSWORD: &str = "correct horse battery";

    struct Device {
        http: reqwest::Client,
        base: String,
        token: String,
    }

    impl Device {
        async fn sign_in(base: &str, user: &str) -> Self {
            let http = reqwest::Client::new();
            let reply: LoginReply = http
                .post(format!("{base}/v1/login/password"))
                .header(PROTOCOL_HEADER, PROTOCOL.to_string())
                .json(&PasswordLoginRequest { user: user.into(), password: PASSWORD.into(), name: "laptop".into() })
                .send()
                .await
                .expect("sends")
                .json()
                .await
                .expect("login");
            Self { http, base: base.to_owned(), token: reply.token }
        }

        fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
            self.http
                .request(method, format!("{}{path}", self.base))
                .header(PROTOCOL_HEADER, PROTOCOL.to_string())
                .bearer_auth(&self.token)
        }

        async fn poll(&self, wait_ms: u64) -> Vec<ControlCommand> {
            let reply: ControlReply = self
                .request(reqwest::Method::GET, &format!("/v1/control?wait_ms={wait_ms}"))
                .send()
                .await
                .expect("sends")
                .json()
                .await
                .expect("reply");
            reply.commands
        }
    }

    async fn start() -> (tempfile::TempDir, Server, String) {
        let data = tempfile::tempdir().expect("temp dir");
        let admin = Admin::open(data.path()).await.expect("opens");
        for name in ["alice", "bob"] {
            admin.add_user(name, Some(PASSWORD), false).await.expect("adds");
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let server = Server::open_with(data.path(), Config::default()).await.expect("opens");
        tokio::spawn(server.clone().serve(listener));
        (data, server, base)
    }

    async fn device_of(server: &Server, user: &str) -> (UserId, DeviceId) {
        let row = sqlx::query!(
            "SELECT d.id, d.user_id FROM devices d JOIN users u ON u.id = d.user_id WHERE u.name = ? ORDER BY d.created_ms DESC",
            user
        )
        .fetch_one(server.db())
        .await
        .expect("a device");
        (UserId(row.user_id), DeviceId(id_from(&row.id)))
    }

    #[tokio::test]
    async fn a_sign_in_code_is_kept_no_longer_than_it_takes_to_deliver() {
        let (_data, server, base) = start().await;
        let laptop = Device::sign_in(&base, "alice").await;
        let (alice, device) = device_of(&server, "alice").await;
        let command = RemoteCommand::SignInCode { code: "the-code".into() };
        let relay = Relay {
            device,
            thread: ThreadId(7),
            sender: alice,
            from: None,
            capabilities: Capabilities::ALL,
            command: command.clone(),
            ttl: Duration::from_secs(60),
        };
        let id = super::relay(&server, relay).await.expect("relays");
        let stored = || {
            sqlx::query_scalar::<_, String>("SELECT command FROM control_commands WHERE id = ?").bind(id).fetch_one(server.db())
        };
        assert!(stored().await.expect("kept").contains("the-code"), "kept for its device to take");
        let taken = laptop.poll(0).await;
        assert_eq!(taken.iter().map(|taken| &taken.command).collect::<Vec<_>>(), [&command], "delivered whole");
        assert!(!stored().await.expect("kept").contains("the-code"), "and gone once taken");
    }

    #[tokio::test]
    async fn a_device_takes_its_commands_once_and_answers_them() {
        let (_data, server, base) = start().await;
        let laptop = Device::sign_in(&base, "alice").await;
        let (alice, device) = device_of(&server, "alice").await;
        let phone = Device::sign_in(&base, "alice").await;
        let (_, phone_device) = device_of(&server, "alice").await;
        assert!(laptop.poll(0).await.is_empty());

        let waiting = tokio::spawn(async move {
            let commands = laptop.poll(10_000).await;
            (laptop, commands)
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        let prompt = RemoteCommand::Prompt { text: "add a test".into(), plan: false };
        let relay_with = |command, ttl, capabilities| Relay {
            device,
            thread: ThreadId(7),
            sender: alice,
            from: Some(phone_device),
            capabilities,
            command,
            ttl,
        };
        let relay = |command, ttl| relay_with(command, ttl, Capabilities::ALL);
        let id = super::relay(&server, relay(prompt.clone(), Duration::from_secs(60))).await.expect("relays");
        let (laptop, commands) = tokio::time::timeout(Duration::from_secs(5), waiting).await.expect("wakes").expect("joins");
        assert_eq!(commands.len(), 1, "the waiting poll hears of it at once");
        assert_eq!((commands[0].id, &commands[0].command, commands[0].sender.as_str()), (id, &prompt, "alice"));
        assert_eq!(commands[0].from, Some(phone_device));
        assert_eq!(commands[0].capabilities, Capabilities::ALL, "a new device may do everything");
        assert!(laptop.poll(0).await.is_empty(), "a command is taken once");

        super::relay(&server, relay_with(RemoteCommand::Cancel, Duration::from_secs(60), Capabilities::PHONE))
            .await
            .expect("relays");
        let limited = laptop.poll(0).await;
        assert_eq!(limited[0].capabilities, Capabilities::PHONE, "a command carries what its sender could do when sent");

        let stale = super::relay(&server, relay(RemoteCommand::Stop, Duration::ZERO)).await.expect("relays");
        assert!(laptop.poll(0).await.is_empty(), "an expired command isn't taken");

        let answer = CommandAnswer { ok: true, message: String::new(), data: "queued".into() };
        let answered =
            |device: &Device, id: i64| device.request(reqwest::Method::POST, &format!("/v1/control/{id}")).json(&answer).send();
        assert_eq!(answer_of(&server, id, Duration::ZERO).await.expect("reads"), None);
        let waiting = {
            let server = server.clone();
            tokio::spawn(async move { answer_of(&server, id, Duration::from_secs(10)).await })
        };
        assert_eq!(answered(&phone, id).await.expect("sends").status(), StatusCode::NOT_FOUND, "only its device answers");
        assert_eq!(answered(&laptop, id).await.expect("sends").status(), StatusCode::NO_CONTENT);
        assert_eq!(answered(&laptop, id).await.expect("sends").status(), StatusCode::NOT_FOUND, "once");
        let heard = tokio::time::timeout(Duration::from_secs(5), waiting).await.expect("wakes").expect("joins");
        assert_eq!(heard.expect("reads"), Some(answer.clone()), "whoever waits hears the answer at once");
        assert_ne!(stale, id);
    }

    #[tokio::test]
    async fn what_a_device_says_of_a_pages_question_so_far_reaches_the_person_who_asked() {
        let (_data, server, base) = start().await;
        let laptop = Device::sign_in(&base, "alice").await;
        let (alice, device) = device_of(&server, "alice").await;
        let phone = Device::sign_in(&base, "alice").await;
        let command = RemoteCommand::Sample {
            question: "Name three rivers".into(),
            images: Vec::new(),
            model: Default::default(),
            tools: Vec::new(),
        };
        let relay = Relay {
            device,
            thread: ThreadId(7),
            sender: alice,
            from: None,
            capabilities: Capabilities::ALL,
            command,
            ttl: Duration::from_secs(60),
        };
        let id = super::relay(&server, relay).await.expect("relays");
        server.shared.control.asked(alice, "q1".into(), id, device);
        let said = |device: &Device, text: &str| {
            device
                .request(reqwest::Method::POST, &format!("/v1/control/{id}/progress"))
                .json(&serde_json::json!({"text": text}))
                .send()
        };
        assert_eq!(said(&phone, "nope").await.expect("sends").status(), StatusCode::NOT_FOUND, "only the device answering it");
        assert_eq!(said(&laptop, "The Nile,").await.expect("sends").status(), StatusCode::NO_CONTENT);
        assert_eq!(server.shared.control.said_so_far(alice, "q1").as_deref(), Some("The Nile,"));
        assert_eq!(server.shared.control.said_so_far(UserId(alice.0 + 1), "q1"), None, "only the person who asked reads it");
        server.shared.control.forget_ask(alice, "q1");
        assert_eq!(server.shared.control.said_so_far(alice, "q1"), None, "forgotten once answered");
    }

    #[tokio::test]
    async fn reports_say_what_runs_on_each_online_device() {
        let (_data, server, base) = start().await;
        let laptop = Device::sign_in(&base, "alice").await;
        let (_, device) = device_of(&server, "alice").await;
        let info = ThreadInfo {
            id: ThreadId(7),
            family: silt_types::FamilyId(8),
            repo: "github.com/acme/app".into(),
            created_ms: 1,
            lease: None,
            owner: String::new(),
            collaborators: Vec::new(),
        };
        let registered = laptop.request(reqwest::Method::POST, "/v1/threads").json(&info).send().await.expect("sends");
        assert_eq!(registered.status(), StatusCode::NO_CONTENT);
        assert!(!server.shared.control.online(device).await);

        let agent = |thread| AgentReport {
            thread,
            agent: "claude".into(),
            agent_id: String::new(),
            state: AgentState::AwaitingPermission,
            permissions: Vec::new(),
            questions: Vec::new(),
            queued: 2,
            preset: "full_access".into(),
            sign_in: None,
            background: 0,
        };
        // A thread Alice doesn't see isn't hers to report.
        let report = AgentsReport { agents: vec![agent(ThreadId(7)), agent(ThreadId(99))], startable: Vec::new() };
        let sent = laptop.request(reqwest::Method::PUT, "/v1/control/agents").json(&report).send().await.expect("sends");
        assert_eq!(sent.status(), StatusCode::NO_CONTENT);
        assert!(server.shared.control.online(device).await);
        let running = server.shared.control.running().await;
        assert_eq!(running.len(), 1);
        assert_eq!(running[&ThreadId(7)], (device, agent(ThreadId(7))));
        assert_eq!(server.shared.control.runs(ThreadId(7)).await, Some((device, true, agent(ThreadId(7)))));
        assert_eq!(server.shared.control.runs(ThreadId(99)).await, None);

        if let Some(heard) = server.shared.control.board.lock().await.devices.get_mut(&device) {
            heard.at = heard.at.checked_sub(OFFLINE_AFTER).expect("the clock has run this long");
        }
        assert!(server.shared.control.running().await.is_empty(), "a device gone quiet runs nothing anyone sees");
        assert_eq!(
            server.shared.control.runs(ThreadId(7)).await,
            Some((device, false, agent(ThreadId(7)))),
            "but its thread is still known to run there, for a prompt to wait for it"
        );
    }

    fn report(state: AgentState, asked: Option<u64>, queued: u32) -> AgentReport {
        AgentReport {
            thread: ThreadId(7),
            agent: "Fake agent".into(),
            agent_id: String::new(),
            state,
            permissions: asked
                .map(|id| silt_proto::AskedPermission {
                    id,
                    title: "Edit a.rs".into(),
                    options: vec![("allow".into(), "Allow".into())],
                })
                .into_iter()
                .collect(),
            questions: Vec::new(),
            queued,
            preset: String::new(),
            sign_in: None,
            background: 0,
        }
    }

    #[test]
    fn a_person_hears_when_their_agent_needs_them_or_is_done() {
        let from = |before: &AgentReport, after: &AgentReport| changes(std::slice::from_ref(before), std::slice::from_ref(after));
        let working = report(AgentState::Working, None, 0);
        let asking = report(AgentState::AwaitingPermission, Some(1), 0);
        assert_eq!(from(&working, &asking), [Change::Asks(asking.clone())]);
        assert!(from(&asking, &asking).is_empty(), "the same request once");
        let again = report(AgentState::AwaitingPermission, Some(2), 0);
        assert_eq!(from(&asking, &again), [Change::Asks(again)], "a new request again");

        let idle = report(AgentState::Idle, None, 0);
        assert_eq!(from(&working, &idle), [Change::Finished(idle.clone())]);
        assert!(from(&working, &report(AgentState::Idle, None, 1)).is_empty(), "the queue goes on");
        assert!(changes(&[], std::slice::from_ref(&idle)).is_empty(), "an agent just started did nothing yet");

        let planned = report(AgentState::AwaitingInput, None, 0);
        assert_eq!(from(&working, &planned), [Change::Planned(planned)]);
        let failed = report(AgentState::Error, None, 0);
        assert_eq!(from(&working, &failed), [Change::Errored(failed.clone())]);
        assert!(from(&failed, &failed).is_empty());

        let note = Change::Asks(asking).note(UserId(3), "workstation");
        assert_eq!((note.kind, note.text.as_str()), ("permission", "Fake agent on workstation asks to run: Edit a.rs"));
        assert!(matches!(note.action, Some(InboxAction::Permission { id: 1, .. })));
    }
}
