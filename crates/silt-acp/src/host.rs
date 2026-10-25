//! One agent process speaking ACP, driven by the daemon: commands come in
//! over a channel and everything the agent does goes out as [`Event`]s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ClientCapabilities, ContentBlock, CreateElicitationRequest, CreateElicitationResponse,
    ElicitationCapabilities, ElicitationFormCapabilities, InitializeRequest, LoadSessionRequest, McpServer, McpServerStdio,
    NewSessionRequest, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionConfigId, SessionConfigValueId, SessionId, SessionModeId, SessionNotification,
    SessionUpdate, SetSessionConfigOptionRequest, SetSessionModeRequest, ToolCallContent,
};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, Responder};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::modes::{Modes, Setting, modes_of, settings_of};
use crate::prompt::{Accepts, Attachment, blocks};

/// How to start the agent.
#[derive(Debug, Clone)]
pub struct Start {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    /// Silt's MCP server: command and arguments.
    pub mcp: Option<(PathBuf, Vec<String>)>,
    /// A session to load instead of starting a new one, when the agent can.
    pub load: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Prompt {
        text: String,
        attachments: Vec<Attachment>,
    },
    Cancel,
    /// The person's answer to a permission request: an option id, or none
    /// to refuse.
    Answer {
        id: u64,
        option: Option<String>,
    },
    /// The person's answer to a question: each field's chosen values, or
    /// none to decline it.
    AnswerQuestion {
        id: u64,
        answers: Option<Vec<(String, Vec<String>)>>,
    },
    SetMode(String),
    /// Picks one of a setting's choices, such as a model.
    SetSetting {
        id: String,
        value: String,
    },
    Stop,
}

/// What the agent did, simplified for recording.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    UserText(String),
    AgentText(String),
    Thought(String),
    ToolCall {
        id: String,
        title: String,
        kind: String,
        input: Option<Value>,
    },
    ToolUpdate {
        id: String,
        status: Option<String>,
        output: Option<String>,
        /// What the call turned out to be, where the agent says more of it
        /// than it did when it announced the call.
        title: Option<String>,
        input: Option<Value>,
    },
    /// The agent's plan: each step and its status.
    Plan(Vec<(String, String)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permission {
    pub id: u64,
    pub title: String,
    /// Option ids and what they say.
    pub options: Vec<(String, String)>,
    /// What the tool does, as ACP names it (`edit`, `read`, `execute`…),
    /// when the request says.
    pub tool_kind: Option<String>,
    /// The tool call it asks about, whose own announcement may say its kind.
    pub tool_call_id: String,
    /// Each option's kind, by its id (`allow_once`, `allow_always`,
    /// `reject_once`, `reject_always`).
    pub option_kinds: Vec<(String, String)>,
}

/// A question the agent asks the person: an ACP form (elicitation) whose
/// fields are choices or free text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: u64,
    pub message: String,
    pub fields: Vec<QuestionField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionField {
    /// The key its answer goes under.
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    /// Its choices; none for free text.
    pub options: Vec<QuestionChoice>,
    /// More than one choice may be taken.
    pub multi: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionChoice {
    /// What the answer sends.
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

impl Question {
    /// Reads a form request's fields; a URL elicitation, which sends the
    /// person to a page, isn't one Silt can show.
    fn of(id: u64, request: &CreateElicitationRequest) -> Option<Self> {
        let raw = serde_json::to_value(request).ok()?;
        (raw.get("mode")?.as_str()? == "form").then_some(())?;
        let schema = raw.get("requestedSchema")?;
        let required: Vec<&str> =
            schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        let mut fields: Vec<QuestionField> = schema
            .get("properties")?
            .as_object()?
            .iter()
            .map(|(key, property)| {
                let text = |value: &Value, name: &str| value.get(name).and_then(Value::as_str).map(str::to_owned);
                let multi = property.get("type").and_then(Value::as_str) == Some("array");
                let choices = if multi { property.get("items").unwrap_or(&Value::Null) } else { property };
                QuestionField {
                    key: key.clone(),
                    title: text(property, "title").unwrap_or_else(|| key.clone()),
                    description: text(property, "description"),
                    options: choices_of(choices),
                    multi,
                }
            })
            .collect();
        // Required fields first, in the order the schema names them.
        fields.sort_by_key(|field| required.iter().position(|key| *key == field.key).unwrap_or(usize::MAX));
        Some(Self { id, message: request.message.clone(), fields })
    }
}

/// A schema's choices: `oneOf`/`anyOf` with titles and descriptions, else a
/// plain `enum`.
fn choices_of(schema: &Value) -> Vec<QuestionChoice> {
    let titled = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array);
    if let Some(titled) = titled {
        return titled
            .iter()
            .filter_map(|choice| {
                let value = choice.get("const")?.as_str()?.to_owned();
                let text = |key: &str| choice.get(key).and_then(Value::as_str).map(str::to_owned);
                Some(QuestionChoice {
                    label: text("title").unwrap_or_else(|| value.clone()),
                    description: text("description"),
                    value,
                })
            })
            .collect();
    }
    schema
        .get("enum")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|value| QuestionChoice { value: value.to_owned(), label: value.to_owned(), description: None })
        .collect()
}

/// The response that gives `answers`, or declines.
fn answered(answers: Option<Vec<(String, Vec<String>)>>, multi: &HashMap<String, bool>) -> CreateElicitationResponse {
    let value = match answers {
        Some(answers) => {
            let content: serde_json::Map<String, Value> = answers
                .into_iter()
                .map(|(key, mut values)| {
                    let value = if multi.get(&key).copied().unwrap_or(false) {
                        Value::from(values)
                    } else {
                        Value::from(values.pop().unwrap_or_default())
                    };
                    (key, value)
                })
                .collect();
            serde_json::json!({"action": "accept", "content": content})
        }
        None => serde_json::json!({"action": "decline"}),
    };
    serde_json::from_value(value).expect("an accept or decline action")
}

/// Why a turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    /// The prompt failed; an [`Event::Failed`] says why.
    Error,
    /// A reason from a newer protocol, by its name there.
    Other(String),
}

impl StopReason {
    fn of(reason: agent_client_protocol::schema::v1::StopReason) -> Self {
        use agent_client_protocol::schema::v1::StopReason as Acp;
        match reason {
            Acp::EndTurn => Self::EndTurn,
            Acp::MaxTokens => Self::MaxTokens,
            Acp::MaxTurnRequests => Self::MaxTurnRequests,
            Acp::Refusal => Self::Refusal,
            Acp::Cancelled => Self::Cancelled,
            other => Self::Other(label(&other)),
        }
    }

    /// The protocol's name for it.
    pub fn as_str(&self) -> &str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::MaxTurnRequests => "max_turn_requests",
            Self::Refusal => "refusal",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
            Self::Other(name) => name,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("could not start {program}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the agent broke off the session")]
    Protocol(#[from] agent_client_protocol::Error),
    #[error("the agent exited with {0}")]
    Exited(std::process::ExitStatus),
    #[error("could not wait for the agent")]
    Wait(#[source] std::io::Error),
}

#[derive(Debug, Clone)]
pub enum Event {
    /// The session is ready; `loaded` when an earlier one was loaded.
    Ready {
        session: String,
        loaded: bool,
        agent: String,
        modes: Option<Modes>,
        settings: Vec<Setting>,
        accepts: Accepts,
    },
    Update(Update),
    ModeChanged(String),
    /// The agent's settings as they now stand.
    Settings(Vec<Setting>),
    /// The agent turned down a mode or setting.
    Refused(String),
    Permission(Permission),
    Question(Question),
    /// What the turn that just ended took, in tokens, when the agent says.
    TurnUsage {
        input: u64,
        output: u64,
    },
    /// How much of its context window the session uses, in tokens.
    Context {
        used: u64,
        size: u64,
    },
    TurnEnded {
        reason: StopReason,
    },
    Failed(Arc<HostError>),
    Exited,
}

/// Sends commands to a running agent.
#[derive(Debug, Clone)]
pub struct AgentHandle {
    commands: mpsc::UnboundedSender<Command>,
}

impl AgentHandle {
    /// Whether the agent is still there to take it.
    pub fn send(&self, command: Command) -> bool {
        self.commands.send(command).is_ok()
    }
}

/// Starts the agent; events arrive on `events` until [`Event::Exited`].
pub fn start(start: Start, events: mpsc::UnboundedSender<Event>) -> AgentHandle {
    let (commands, receiver) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        if let Err(error) = run(start, events.clone(), receiver).await {
            let _ = events.send(Event::Failed(Arc::new(error)));
        }
        let _ = events.send(Event::Exited);
    });
    AgentHandle { commands }
}

fn text_of(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(text) => text.text.clone(),
        _ => String::new(),
    }
}

pub(crate) fn label<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}

fn tool_output(content: &[ToolCallContent]) -> Option<String> {
    let text: Vec<String> = content
        .iter()
        .map(|item| match item {
            ToolCallContent::Content(content) => text_of(&content.content),
            ToolCallContent::Diff(diff) => format!("edited {}", diff.path.display()),
            _ => String::new(),
        })
        .filter(|text| !text.is_empty())
        .collect();
    (!text.is_empty()).then(|| text.join("\n"))
}

fn event_of(update: SessionUpdate) -> Option<Event> {
    match update {
        SessionUpdate::CurrentModeUpdate(update) => Some(Event::ModeChanged(update.current_mode_id.0.to_string())),
        SessionUpdate::ConfigOptionUpdate(update) => Some(Event::Settings(settings_of(update.config_options))),
        SessionUpdate::UsageUpdate(update) => Some(Event::Context { used: update.used, size: update.size }),
        update => simplify(update).map(Event::Update),
    }
}

fn simplify(update: SessionUpdate) -> Option<Update> {
    Some(match update {
        SessionUpdate::UserMessageChunk(chunk) => Update::UserText(text_of(&chunk.content)),
        SessionUpdate::AgentMessageChunk(chunk) => Update::AgentText(text_of(&chunk.content)),
        SessionUpdate::AgentThoughtChunk(chunk) => Update::Thought(text_of(&chunk.content)),
        SessionUpdate::ToolCall(call) => Update::ToolCall {
            id: call.tool_call_id.to_string(),
            title: call.title,
            kind: label(&call.kind),
            input: call.raw_input,
        },
        SessionUpdate::ToolCallUpdate(update) => Update::ToolUpdate {
            id: update.tool_call_id.to_string(),
            status: update.fields.status.as_ref().map(label),
            output: update
                .fields
                .content
                .as_deref()
                .and_then(tool_output)
                .or_else(|| update.fields.raw_output.as_ref().map(Value::to_string)),
            title: update.fields.title,
            input: update.fields.raw_input,
        },
        SessionUpdate::Plan(plan) => {
            Update::Plan(plan.entries.into_iter().map(|entry| (entry.content, label(&entry.status))).collect())
        }
        _ => return None,
    })
}

type Waiting = Arc<Mutex<HashMap<u64, Responder<RequestPermissionResponse>>>>;
/// Questions waiting on the person, with which of their fields take several choices.
type Asking = Arc<Mutex<HashMap<u64, (Responder<CreateElicitationResponse>, HashMap<String, bool>)>>>;

/// The program to start. Windows runs only what it's given in full, so a
/// bare `npx` is looked up there, `npx.cmd` included, on the agent's own
/// `PATH`.
fn program(start: &Start) -> std::path::PathBuf {
    let bare = std::path::PathBuf::from(&start.program);
    if !cfg!(windows) || bare.components().count() > 1 {
        return bare;
    }
    let path = start.env.iter().find(|(name, _)| name.eq_ignore_ascii_case("PATH")).map(|(_, value)| value.as_str());
    path.and_then(|path| silt_os::find_program(&start.program, std::ffi::OsStr::new(path))).unwrap_or(bare)
}

async fn run(
    start: Start,
    events: mpsc::UnboundedSender<Event>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> Result<(), HostError> {
    let mut command = tokio::process::Command::new(program(&start));
    // Its own process group, so stopping it stops what it started too,
    // such as the agent `npx` runs.
    silt_os::own_group(&mut command);
    let mut child = command
        .args(&start.args)
        // Only what the daemon chose to pass: see `registry::agent_environment`.
        .env_clear()
        .envs(start.env.iter().map(|(key, value)| (key, value)))
        .current_dir(&start.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| HostError::Spawn { program: start.program.clone(), source })?;
    let group = child.id();
    let stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let transport = ByteStreams::new(stdin.compat_write(), stdout.compat());

    let replaying = Arc::new(AtomicBool::new(false));
    let waiting: Waiting = Arc::default();
    let asking: Asking = Arc::default();
    let next_id = Arc::new(AtomicU64::new(1));
    let question_ids = next_id.clone();
    let questioned = events.clone();
    let held = asking.clone();
    let mcp_servers: Vec<McpServer> = start
        .mcp
        .iter()
        .map(|(command, args)| McpServer::Stdio(McpServerStdio::new("silt", command.clone()).args(args.clone())))
        .collect();

    let updates = events.clone();
    let quiet = replaying.clone();
    let asked = events.clone();
    let parked = waiting.clone();
    let session = Client
        .builder()
        .name("silt")
        .on_receive_notification(
            async move |notification: SessionNotification, _connection| {
                if !quiet.load(Ordering::SeqCst)
                    && let Some(event) = event_of(notification.update)
                {
                    let _ = updates.send(event);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                let id = next_id.fetch_add(1, Ordering::SeqCst);
                let options = request.options.iter().map(|option| (option.option_id.to_string(), option.name.clone())).collect();
                let option_kinds =
                    request.options.iter().map(|option| (option.option_id.to_string(), label(&option.kind))).collect();
                let title = request.tool_call.fields.title.clone().unwrap_or_else(|| "a tool call".to_owned());
                let tool_kind = request.tool_call.fields.kind.as_ref().map(label);
                let tool_call_id = request.tool_call.tool_call_id.to_string();
                parked.lock().expect("lock").insert(id, responder);
                let _ = asked.send(Event::Permission(Permission { id, title, options, tool_kind, tool_call_id, option_kinds }));
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: CreateElicitationRequest, responder, _connection| {
                let id = question_ids.fetch_add(1, Ordering::SeqCst);
                match Question::of(id, &request) {
                    Some(question) => {
                        let multi = question.fields.iter().map(|field| (field.key.clone(), field.multi)).collect();
                        held.lock().expect("lock").insert(id, (responder, multi));
                        let _ = questioned.send(Event::Question(question));
                    }
                    None => responder.respond(answered(None, &HashMap::new()))?,
                }
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |connection: ConnectionTo<Agent>| {
            // Silt shows a form of choices and text as a question; it has no browser to send the person to.
            let capabilities =
                ClientCapabilities::new().elicitation(ElicitationCapabilities::new().form(ElicitationFormCapabilities::new()));
            let init = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(capabilities))
                .block_task()
                .await?;
            let agent = init.agent_info.as_ref().map_or_else(|| start.program.clone(), |info| info.name.clone());
            let accepts = Accepts::of(&init.agent_capabilities.prompt_capabilities);
            let loaded = match start.load.clone().filter(|_| init.agent_capabilities.load_session) {
                Some(id) => {
                    replaying.store(true, Ordering::SeqCst);
                    let loaded = connection
                        .send_request(
                            LoadSessionRequest::new(SessionId::new(id.clone()), start.cwd.clone())
                                .mcp_servers(mcp_servers.clone()),
                        )
                        .block_task()
                        .await;
                    replaying.store(false, Ordering::SeqCst);
                    // A session made on another machine can't be loaded
                    // here; the daemon primes a new one instead.
                    loaded.ok().map(|response| (id, response.modes, response.config_options))
                }
                None => None,
            };
            let session: SessionId = match loaded {
                Some((id, modes, options)) => {
                    let _ = events.send(Event::Ready {
                        session: id.clone(),
                        loaded: true,
                        agent,
                        modes: modes_of(modes),
                        settings: settings_of(options.unwrap_or_default()),
                        accepts,
                    });
                    SessionId::new(id)
                }
                None => {
                    let created = connection
                        .send_request(NewSessionRequest::new(start.cwd.clone()).mcp_servers(mcp_servers.clone()))
                        .block_task()
                        .await?;
                    let _ = events.send(Event::Ready {
                        session: created.session_id.to_string(),
                        loaded: false,
                        agent,
                        modes: modes_of(created.modes),
                        settings: settings_of(created.config_options.unwrap_or_default()),
                        accepts,
                    });
                    created.session_id
                }
            };
            while let Some(command) = commands.recv().await {
                match command {
                    Command::Prompt { text, attachments } => {
                        let request = PromptRequest::new(session.clone(), blocks(&text, &attachments, accepts));
                        let ended = events.clone();
                        // Sent here rather than in the task, so a mode set
                        // just before reaches the agent first.
                        let sent = connection.send_request(request);
                        connection.spawn(async move {
                            match sent.block_task().await {
                                Ok(response) => {
                                    if let Some(usage) = &response.usage {
                                        let _ = ended
                                            .send(Event::TurnUsage { input: usage.input_tokens, output: usage.output_tokens });
                                    }
                                    let _ = ended.send(Event::TurnEnded { reason: StopReason::of(response.stop_reason) });
                                }
                                Err(error) => {
                                    let _ = ended.send(Event::Failed(Arc::new(HostError::Protocol(error))));
                                    let _ = ended.send(Event::TurnEnded { reason: StopReason::Error });
                                }
                            }
                            Ok(())
                        })?;
                    }
                    Command::Cancel => {
                        connection.send_notification(CancelNotification::new(session.clone()))?;
                        // The protocol asks clients to answer every pending
                        // permission request once they cancel.
                        let parked: Vec<_> = waiting.lock().expect("lock").drain().map(|(_, responder)| responder).collect();
                        for responder in parked {
                            responder.respond(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled))?;
                        }
                        let asked: Vec<_> = asking.lock().expect("lock").drain().map(|(_, (responder, _))| responder).collect();
                        for responder in asked {
                            responder.respond(
                                serde_json::from_value(serde_json::json!({"action": "cancel"})).expect("a cancel action"),
                            )?;
                        }
                    }
                    Command::Answer { id, option } => {
                        let responder = waiting.lock().expect("lock").remove(&id);
                        if let Some(responder) = responder {
                            let outcome = match option {
                                Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option)),
                                None => RequestPermissionOutcome::Cancelled,
                            };
                            responder.respond(RequestPermissionResponse::new(outcome))?;
                        }
                    }
                    Command::AnswerQuestion { id, answers } => {
                        let held = asking.lock().expect("lock").remove(&id);
                        if let Some((responder, multi)) = held {
                            responder.respond(answered(answers, &multi))?;
                        }
                    }
                    Command::SetMode(mode) => {
                        let request = SetSessionModeRequest::new(session.clone(), SessionModeId::new(mode.clone()));
                        let told = events.clone();
                        let sent = connection.send_request(request);
                        connection.spawn(async move {
                            let event = match sent.block_task().await {
                                Ok(_) => Event::ModeChanged(mode),
                                Err(error) => Event::Refused(format!("the agent kept its mode: {error}")),
                            };
                            let _ = told.send(event);
                            Ok(())
                        })?;
                    }
                    Command::SetSetting { id, value } => {
                        let request = SetSessionConfigOptionRequest::new(
                            session.clone(),
                            SessionConfigId::new(id),
                            SessionConfigValueId::new(value),
                        );
                        let told = events.clone();
                        let sent = connection.send_request(request);
                        connection.spawn(async move {
                            let event = match sent.block_task().await {
                                Ok(response) => Event::Settings(settings_of(response.config_options)),
                                Err(error) => Event::Refused(format!("the agent kept its setting: {error}")),
                            };
                            let _ = told.send(event);
                            Ok(())
                        })?;
                    }
                    Command::Stop => break,
                }
            }
            Ok(())
        });
    // The agent may die without closing the connection cleanly; its exit
    // ends the session either way.
    let result = tokio::select! {
        result = session => result.map_err(HostError::Protocol),
        status = child.wait() => match status {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(HostError::Exited(status)),
            Err(error) => Err(HostError::Wait(error)),
        },
    };
    if let Some(group) = group {
        silt_os::kill_tree(group);
    }
    let _ = child.kill().await;
    result
}
