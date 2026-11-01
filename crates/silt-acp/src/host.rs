//! One agent process speaking ACP, driven by the daemon: commands come in
//! over a channel and everything the agent does goes out as [`Event`]s.
//! Silt asks for ACP v2 and speaks v1 to an agent that answers with it
//! (ADR 0050): both come out as the same events.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use agent_client_protocol::{ByteStreams, Client};
use futures::io::{AsyncRead, AsyncWrite};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::modes::{Modes, Setting};
use crate::prompt::{Accepts, Attachment};

/// What Silt calls itself to agents.
pub(crate) const NAME: &str = "silt";

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
    /// A sign-in method of the agent's own to ask it to follow before the
    /// session opens (ADR 0054).
    pub sign_in_first: Option<String>,
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
        task: Option<Box<TaskNews>>,
    },
    /// The agent's plan: each step and its status.
    Plan(Vec<(String, String)>),
    /// What a subagent did, inside the tool call `parent` that started it.
    Within {
        parent: String,
        update: Box<Update>,
    },
}

/// What a call's update says of work it left running beside the turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskNews {
    /// The agent's id for it, when the update names it.
    pub id: Option<String>,
    /// `subagent` or `background`, when the update says.
    pub kind: Option<String>,
    /// `running`, `completed`, `failed` or `stopped`.
    pub status: String,
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
    /// The form can't be sent without it.
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionChoice {
    /// What the answer sends.
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

impl Question {
    /// Reads a form request's fields, as both protocols write it; a URL
    /// elicitation, which sends the person to a page, isn't one Silt can
    /// show.
    pub(crate) fn of(id: u64, raw: &Value) -> Option<Self> {
        (raw.get("mode")?.as_str()? == "form").then_some(())?;
        let message = raw.get("message").and_then(Value::as_str).unwrap_or_default().to_owned();
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
                    required: required.contains(&key.as_str()),
                }
            })
            .collect();
        // Required fields first, in the order the schema names them.
        fields.sort_by_key(|field| required.iter().position(|key| *key == field.key).unwrap_or(usize::MAX));
        Some(Self { id, message, fields })
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

/// The response that gives `answers`, or declines, as both protocols
/// write it.
pub(crate) fn answered<Response: DeserializeOwned>(
    answers: Option<Vec<(String, Vec<String>)>>,
    multi: &HashMap<String, bool>,
) -> Response {
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
        /// The agent says when it works and when it stops (ACP v2), so work
        /// it takes up by itself, between prompts, starts with
        /// [`Event::Working`] and ends with [`Event::TurnEnded`] too.
        reports_work: bool,
    },
    /// The agent works, on a prompt or on something it took up by itself.
    Working,
    Update(Update),
    ModeChanged(String),
    /// The agent's settings as they now stand.
    Settings(Vec<Setting>),
    /// The commands the agent offers as they now stand.
    Commands(Vec<crate::SlashCommand>),
    /// The agent turned down a mode or setting.
    Refused(String),
    Permission(Permission),
    Question(Question),
    /// The agent needs a sign-in, in one of the ways it lists, before it
    /// opens its session or takes the prompt it refused (ADR 0054). The
    /// session ended if it hadn't opened.
    SignIn(Vec<crate::SignInMethod>),
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
    stop: Arc<watch::Sender<bool>>,
}

impl AgentHandle {
    /// Whether the agent is still there to take it.
    pub fn send(&self, command: Command) -> bool {
        // Commands are read once the session is ready; a stop is heard
        // before that too, as an agent can hang while starting.
        if matches!(command, Command::Stop) {
            self.stop.send_replace(true);
        }
        self.commands.send(command).is_ok()
    }
}

/// Starts the agent; events arrive on `events` until [`Event::Exited`].
pub fn start(start: Start, events: mpsc::UnboundedSender<Event>) -> AgentHandle {
    let (commands, receiver) = mpsc::unbounded_channel();
    let (stop, stopped) = watch::channel(false);
    tokio::spawn(async move {
        if let Err(error) = run(start, events.clone(), receiver, stopped).await {
            let _ = events.send(Event::Failed(Arc::new(error)));
        }
        let _ = events.send(Event::Exited);
    });
    AgentHandle { commands, stop: Arc::new(stop) }
}

pub(crate) fn label<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}

/// What a session in either protocol starts from: how the agent was
/// started, where its events go, and the daemon's commands, which the
/// session the agent settles on takes.
#[derive(Clone)]
pub(crate) struct Shared {
    pub(crate) start: Start,
    pub(crate) events: mpsc::UnboundedSender<Event>,
    pub(crate) commands: Arc<Mutex<Option<mpsc::UnboundedReceiver<Command>>>>,
}

type Outgoing = Pin<Box<dyn AsyncWrite + Send>>;
type Incoming = Pin<Box<dyn AsyncRead + Send>>;

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
    commands: mpsc::UnboundedReceiver<Command>,
    mut stopped: watch::Receiver<bool>,
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
    let mut transport: Option<ByteStreams<Outgoing, Incoming>> =
        Some(ByteStreams::new(Box::pin(stdin.compat_write()), Box::pin(stdout.compat())));
    let shared = Shared { start, events, commands: Arc::new(Mutex::new(Some(commands))) };
    let (v1, v2) = (shared.clone(), shared);
    let session = Client
        .protocol_connector()
        .with_v2(move || crate::v2::V2(v2.clone()))
        .with_v1(move || crate::v1::V1(v1.clone()))
        // Silt's v1 `initialize` says what its v2 one does, so an agent that
        // answers with v1 goes on over the same connection; were it asked
        // for another, nothing would answer it and the session would end.
        .connect_to(move || {
            transport.take().unwrap_or_else(|| {
                ByteStreams::new(Box::pin(futures::io::sink()) as Outgoing, Box::pin(futures::io::empty()) as Incoming)
            })
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
        _ = stopped.wait_for(|stopped| *stopped) => Ok(()),
    };
    if let Some(group) = group {
        silt_os::kill_tree(group);
    }
    let _ = child.kill().await;
    result
}
