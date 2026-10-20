//! The harness as an ACP agent: sessions in a checkout, each prompt a loop
//! of model calls and tool runs until the model stops asking for tools.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, Content, ContentBlock, ContentChunk, Implementation, InitializeRequest,
    InitializeResponse, LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse,
    PermissionOption, PermissionOptionKind, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionId, SessionNotification, SessionUpdate, StopReason, TextContent, ToolCall,
    ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Stdio};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use silt_redact::{Overlay, Redactor};

use crate::model::{Backend, Message, Part, Role};
use crate::tools::{self, Outcome, Workspace};

/// Most model calls one prompt may take.
const MAX_STEPS: usize = 50;
const RUN_TIMEOUT: Duration = Duration::from_secs(300);
const RULES_BUDGET: usize = 8_000;

/// Where the harness runs and how.
#[derive(Debug, Clone)]
pub struct Config {
    /// The `silt` binary, for `silt sandbox`.
    pub silt: PathBuf,
    /// Where sessions are kept, to load them again.
    pub state_dir: PathBuf,
    /// Run commands without the sandbox; only when the user asked.
    pub unsandboxed: bool,
    /// More secrets to keep from the model, one per line (ADR 0015).
    pub secrets_file: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    cwd: PathBuf,
    messages: Vec<Message>,
}

struct Session {
    cwd: PathBuf,
    messages: Vec<Message>,
    workspace: Workspace,
    always_run: bool,
    cancelled: Arc<AtomicBool>,
}

/// Each session, with its cancel flag outside the session's lock, which
/// a prompt holds for its whole turn.
type Sessions = Arc<Mutex<HashMap<String, (Arc<AtomicBool>, Arc<tokio::sync::Mutex<Session>>)>>>;

struct Harness {
    config: Config,
    sessions: Sessions,
    http: reqwest::Client,
    /// What reaches the model is redacted; what it writes back has the
    /// secrets put back before it is used.
    redactor: Redactor,
    overlay: Mutex<Overlay>,
}

/// `value` with the secrets in every string put back.
fn reveal_json(overlay: &Overlay, value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(overlay.reveal(text)),
        Value::Array(items) => Value::Array(items.iter().map(|item| reveal_json(overlay, item)).collect()),
        Value::Object(fields) => Value::Object(fields.iter().map(|(key, item)| (key.clone(), reveal_json(overlay, item))).collect()),
        other => other.clone(),
    }
}

fn text(content: impl Into<String>) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(content.into())))
}

fn failed(error: impl std::fmt::Display) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(error.to_string())
}

fn system_prompt(root: &Path) -> String {
    let mut prompt = format!(
        "You are Silt's coding agent, working in a checkout of a git repository at {}. \
         Use the tools to read, change and run things there; paths are relative to the checkout. \
         Read a file before changing it. If a change is refused because the file changed, someone else edited it: \
         read it again and make your change on top of theirs. Commands run in a sandbox that can write only the \
         checkout and /tmp and has no network. Say briefly what you did.",
        root.display()
    );
    for name in ["AGENTS.md", "CLAUDE.md"] {
        if let Ok(rules) = std::fs::read_to_string(root.join(name)) {
            let rules: String = rules.chars().take(RULES_BUDGET).collect();
            prompt.push_str(&format!("\n\nThe project's {name}:\n{rules}"));
        }
    }
    prompt
}

fn title(name: &str, input: &Value) -> String {
    let path = input["path"].as_str().unwrap_or(".");
    match name {
        "read_file" => format!("Read {path}"),
        "write_file" => format!("Write {path}"),
        "edit_file" => format!("Edit {path}"),
        "list_files" => format!("List {path}"),
        "run" => format!("Run `{}`", input["command"].as_str().unwrap_or_default()),
        other => other.to_owned(),
    }
}

fn kind(name: &str) -> ToolKind {
    match name {
        "read_file" => ToolKind::Read,
        "write_file" | "edit_file" => ToolKind::Edit,
        "list_files" => ToolKind::Search,
        "run" => ToolKind::Execute,
        _ => ToolKind::Other,
    }
}

impl Harness {
    /// `text` as the model may see it.
    fn redact(&self, text: &str) -> String {
        let (redacted, found) = self.redactor.replace_found(text);
        let mut overlay = self.overlay.lock().expect("lock");
        for secret in found {
            overlay.learn(secret);
        }
        redacted
    }

    fn reveal(&self, text: &str) -> String {
        self.overlay.lock().expect("lock").reveal(text)
    }

    fn saved_path(&self, id: &str) -> PathBuf {
        self.config.state_dir.join(format!("{id}.json"))
    }

    fn save(&self, id: &str, session: &Session) {
        let saved = Saved {
            cwd: session.cwd.clone(),
            messages: session.messages.clone(),
        };
        if let Ok(text) = serde_json::to_string(&saved) {
            let _ = std::fs::create_dir_all(&self.config.state_dir);
            let _ = std::fs::write(self.saved_path(id), text);
        }
    }

    fn open(&self, id: &str, cwd: PathBuf, messages: Vec<Message>) -> agent_client_protocol::Result<()> {
        let cancelled = Arc::<AtomicBool>::default();
        let session = Session {
            workspace: Workspace::new(&cwd).map_err(failed)?,
            cwd,
            messages,
            always_run: false,
            cancelled: cancelled.clone(),
        };
        self.sessions
            .lock()
            .expect("lock")
            .insert(id.to_owned(), (cancelled, Arc::new(tokio::sync::Mutex::new(session))));
        Ok(())
    }

    fn session(&self, id: &str) -> agent_client_protocol::Result<Arc<tokio::sync::Mutex<Session>>> {
        self.sessions
            .lock()
            .expect("lock")
            .get(id)
            .map(|(_, session)| session.clone())
            .ok_or_else(|| failed(format!("no session {id}")))
    }

    async fn prompt(&self, connection: &ConnectionTo<Client>, id: &SessionId, prompt: String) -> agent_client_protocol::Result<StopReason> {
        let backend = Backend::from_env(|name| std::env::var(name).ok()).map_err(failed)?;
        let session = self.session(&id.to_string())?;
        let mut session = session.lock().await;
        session.cancelled.store(false, Ordering::SeqCst);
        session.messages.push(Message {
            role: Role::User,
            parts: vec![Part::Text { text: self.redact(&prompt) }],
        });
        let system = self.redact(&system_prompt(session.workspace.root()));
        let specs = tools::specs();
        let notify = |update| connection.send_notification(SessionNotification::new(id.clone(), update));
        for _ in 0..MAX_STEPS {
            if session.cancelled.load(Ordering::SeqCst) {
                self.save(&id.to_string(), &session);
                return Ok(StopReason::Cancelled);
            }
            let reply = backend.complete(&self.http, &system, &session.messages, &specs).await.map_err(failed)?;
            for part in &reply.parts {
                if let Part::Text { text: said } = part {
                    notify(SessionUpdate::AgentMessageChunk(text(self.reveal(said))))?;
                }
            }
            session.messages.push(Message {
                role: Role::Assistant,
                parts: reply.parts.clone(),
            });
            let uses: Vec<(String, String, Value)> = reply
                .tool_uses()
                .map(|(call, name, input)| (call.to_owned(), name.to_owned(), input.clone()))
                .collect();
            if uses.is_empty() {
                self.save(&id.to_string(), &session);
                return Ok(StopReason::EndTurn);
            }
            let mut results = Vec::new();
            for (call, name, input) in uses {
                let started = ToolCall::new(call.clone(), title(&name, &input))
                    .kind(kind(&name))
                    .status(ToolCallStatus::InProgress)
                    .raw_input(input.clone());
                notify(SessionUpdate::ToolCall(started))?;
                let outcome = if session.cancelled.load(Ordering::SeqCst) {
                    Err("cancelled".to_owned())
                } else {
                    let revealed = reveal_json(&self.overlay.lock().expect("lock"), &input);
                    self.use_tool(connection, id, &mut session, &call, &name, &revealed).await?
                };
                let (status, output, is_error) = match outcome {
                    Ok(output) => (ToolCallStatus::Completed, output, false),
                    Err(error) => (ToolCallStatus::Failed, error, true),
                };
                let shown = ToolCallContent::Content(Content::new(ContentBlock::Text(TextContent::new(output.clone()))));
                let finished = ToolCallUpdate::new(call.clone(), ToolCallUpdateFields::new().status(status).content(vec![shown]));
                notify(SessionUpdate::ToolCallUpdate(finished))?;
                results.push(Part::ToolResult {
                    id: call,
                    output: self.redact(&output),
                    is_error,
                });
            }
            session.messages.push(Message {
                role: Role::User,
                parts: results,
            });
            self.save(&id.to_string(), &session);
        }
        Ok(StopReason::MaxTurnRequests)
    }

    async fn use_tool(
        &self,
        connection: &ConnectionTo<Client>,
        id: &SessionId,
        session: &mut Session,
        call: &str,
        name: &str,
        input: &Value,
    ) -> agent_client_protocol::Result<Outcome> {
        Ok(match name {
            "read_file" => session.workspace.read_file(input),
            "write_file" => session.workspace.write_file(input),
            "edit_file" => session.workspace.edit_file(input),
            "list_files" => session.workspace.list_files(input),
            "run" => {
                let Some(command) = input["command"].as_str() else {
                    return Ok(Err("`command` is missing".into()));
                };
                if !session.always_run {
                    match self.ask_to_run(connection, id, call, command).await? {
                        Some(always) => session.always_run |= always,
                        None => return Ok(Err("the user did not allow this command".into())),
                    }
                }
                self.run(session.workspace.root(), command).await
            }
            other => Err(format!("there is no tool {other}")),
        })
    }

    /// Asks the person whether to run `command`; `Some(true)` allows it
    /// for the rest of the session.
    async fn ask_to_run(&self, connection: &ConnectionTo<Client>, id: &SessionId, call: &str, command: &str) -> agent_client_protocol::Result<Option<bool>> {
        let options = vec![
            PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
            PermissionOption::new("always", "Allow all commands in this session", PermissionOptionKind::AllowAlways),
            PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
        ];
        let request = RequestPermissionRequest::new(
            id.clone(),
            ToolCallUpdate::new(call.to_owned(), ToolCallUpdateFields::new().title(format!("Run `{command}`"))),
            options,
        );
        let answer = connection.send_request(request).block_task().await?;
        Ok(match &answer.outcome {
            RequestPermissionOutcome::Selected(selected) => match selected.option_id.to_string().as_str() {
                "allow" => Some(false),
                "always" => Some(true),
                _ => None,
            },
            _ => None,
        })
    }

    async fn run(&self, root: &Path, command: &str) -> Outcome {
        let mut process = if self.config.unsandboxed {
            tokio::process::Command::new("sh")
        } else {
            let mut sandboxed = tokio::process::Command::new(&self.config.silt);
            sandboxed.arg("sandbox").arg("--write").arg(root).args(["--", "sh"]);
            sandboxed
        };
        process.arg("-c").arg(command).current_dir(root).kill_on_drop(true);
        let output = match tokio::time::timeout(RUN_TIMEOUT, process.output()).await {
            Ok(output) => output.map_err(|error| format!("cannot run the command: {error}"))?,
            Err(_) => return Err(format!("the command ran longer than {} seconds", RUN_TIMEOUT.as_secs())),
        };
        let mut shown = format!("exit status {}\n", output.status.code().unwrap_or(-1));
        shown.push_str(&String::from_utf8_lossy(&output.stdout));
        shown.push_str(&String::from_utf8_lossy(&output.stderr));
        Ok(tools::truncate(shown))
    }
}

fn prompt_text(request: &PromptRequest) -> String {
    request
        .prompt
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Serves the harness over ACP on stdin and stdout until the client leaves.
pub async fn serve(config: Config) -> agent_client_protocol::Result<()> {
    let redactor = Redactor::from_env().with(config.secrets_file.as_deref().map(silt_redact::read_secrets_file).unwrap_or_default());
    let overlay = Mutex::new(Overlay::new(redactor.secrets().iter().cloned()));
    let harness = Arc::new(Harness {
        config,
        sessions: Sessions::default(),
        http: reqwest::Client::new(),
        redactor,
        overlay,
    });
    let (created, loading, prompted, cancelling) = (harness.clone(), harness.clone(), harness.clone(), harness);
    Agent
        .builder()
        .name("silt-harness")
        .on_receive_request(
            async move |initialize: InitializeRequest, responder, _connection| {
                responder.respond(
                    InitializeResponse::new(initialize.protocol_version)
                        .agent_capabilities(AgentCapabilities::new().load_session(true))
                        .agent_info(Implementation::new("Silt harness", env!("CARGO_PKG_VERSION"))),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _connection| {
                let id = uuid::Uuid::now_v7().simple().to_string();
                created.open(&id, request.cwd, Vec::new())?;
                responder.respond(NewSessionResponse::new(SessionId::new(id)))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: LoadSessionRequest, responder, connection| {
                let id = request.session_id.to_string();
                let stored = std::fs::read_to_string(loading.saved_path(&id)).map_err(|_| failed(format!("no saved session {id}")))?;
                let saved: Saved = serde_json::from_str(&stored).map_err(failed)?;
                for message in &saved.messages {
                    for part in &message.parts {
                        if let Part::Text { text: said } = part {
                            let update = match message.role {
                                Role::User => SessionUpdate::UserMessageChunk(text(said.clone())),
                                Role::Assistant => SessionUpdate::AgentMessageChunk(text(said.clone())),
                            };
                            connection.send_notification(SessionNotification::new(request.session_id.clone(), update))?;
                        }
                    }
                }
                loading.open(&id, request.cwd, saved.messages)?;
                responder.respond(LoadSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection| {
                let harness = prompted.clone();
                let worker = connection.clone();
                connection.spawn(async move {
                    let stop = harness.prompt(&worker, &request.session_id, prompt_text(&request)).await;
                    match stop {
                        Ok(stop) => responder.respond(PromptResponse::new(stop)),
                        Err(error) => responder.respond_with_error(error),
                    }
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |cancel: CancelNotification, _connection| {
                if let Some((cancelled, _)) = cancelling.sessions.lock().expect("lock").get(&cancel.session_id.to_string()) {
                    cancelled.store(true, Ordering::SeqCst);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await
}
