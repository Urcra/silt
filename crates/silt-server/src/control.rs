//! Each daemon's control channel (ADR 0026): the commands people send its
//! threads from elsewhere, which it long-polls for and answers, and its
//! reports of the agents it runs, kept in memory for browsers to show.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use silt_proto::{AgentReport, AgentsReport, Capabilities, CommandAnswer, ControlCommand, ControlReply, RemoteCommand};
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
}

struct Heard {
    agents: Vec<AgentReport>,
    at: Instant,
}

/// What each device last reported, by device.
#[derive(Default)]
struct Board {
    devices: HashMap<DeviceId, Heard>,
}

impl Board {
    fn hear(&mut self, device: DeviceId, agents: Option<Vec<AgentReport>>) {
        let heard = self.devices.entry(device).or_insert_with(|| Heard { agents: Vec::new(), at: Instant::now() });
        heard.at = Instant::now();
        if let Some(agents) = agents {
            heard.agents = agents;
        }
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
        control.board.lock().await.hear(caller.device, None);
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
         RETURNING id AS "id!", thread_id, from_device, command, created_ms,
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
        commands.push(ControlCommand {
            id: row.id,
            thread: ThreadId(id_from(&row.thread_id)),
            sender: row.sender,
            from: row.from_device.as_deref().map(|device| DeviceId(id_from(device))),
            // Browsers and devices alike may do everything, until people
            // choose per device.
            capabilities: Capabilities::ALL,
            sent_ms: row.created_ms,
            command,
        });
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
    let agents = report.agents.into_iter().filter(|agent| viewer.sees_thread(agent.thread)).take(MAX_AGENTS).collect();
    server.shared.control.board.lock().await.hear(caller.device, Some(agents));
    Ok(StatusCode::NO_CONTENT)
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
    pub(crate) command: RemoteCommand,
    pub(crate) ttl: Duration,
}

/// Leaves the command for its daemon's next poll, and wakes the polls.
pub(crate) async fn relay(server: &Server, relay: Relay) -> Result<i64, ServerError> {
    let (device, thread, from) = (id_bytes(relay.device.0), id_bytes(relay.thread.0), relay.from.map(|from| id_bytes(from.0)));
    let command = serde_json::to_string(&relay.command)?;
    let now = now_ms();
    let expires = now.saturating_add(i64::try_from(relay.ttl.as_millis()).unwrap_or(i64::MAX));
    let id = sqlx::query_scalar!(
        r#"INSERT INTO control_commands (device_id, thread_id, sender_id, from_device, command, created_ms, expires_ms)
         VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id AS "id!""#,
        device,
        thread,
        relay.sender,
        from,
        command,
        now,
        expires
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
        let relay = |command, ttl| Relay { device, thread: ThreadId(7), sender: alice, from: Some(phone_device), command, ttl };
        let id = super::relay(&server, relay(prompt.clone(), Duration::from_secs(60))).await.expect("relays");
        let (laptop, commands) = tokio::time::timeout(Duration::from_secs(5), waiting).await.expect("wakes").expect("joins");
        assert_eq!(commands.len(), 1, "the waiting poll hears of it at once");
        assert_eq!((commands[0].id, &commands[0].command, commands[0].sender.as_str()), (id, &prompt, "alice"));
        assert_eq!(commands[0].from, Some(phone_device));
        assert!(laptop.poll(0).await.is_empty(), "a command is taken once");

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
            state: AgentState::AwaitingPermission,
            permissions: Vec::new(),
            queued: 2,
            preset: "full_access".into(),
        };
        // A thread Alice doesn't see isn't hers to report.
        let report = AgentsReport { agents: vec![agent(ThreadId(7)), agent(ThreadId(99))] };
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
}
