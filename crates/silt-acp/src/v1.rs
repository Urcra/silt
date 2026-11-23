//! A session with an agent that speaks ACP v1, as Copilot does: a prompt's
//! turn ends when its request is answered.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::{
    AuthCapabilities, AuthenticateRequest, CancelNotification, ClientCapabilities, ClientSessionCapabilities, ContentBlock,
    CreateElicitationRequest, CreateElicitationResponse, ElicitationCapabilities, ElicitationFormCapabilities, EnvVariable,
    Implementation, InitializeRequest, LoadSessionRequest, McpServer, McpServerStdio, Meta, NewSessionRequest, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome, SessionConfigId,
    SessionConfigValueId, SessionId, SessionModeId, SessionNotification, SessionUpdate, SetSessionConfigOptionRequest,
    SetSessionModeRequest, ToolCallContent,
};
use agent_client_protocol::v1::{ConnectionTo, Responder};
use agent_client_protocol::{Agent, Client, ConnectTo};
use serde_json::Value;

use crate::host::{
    Changing, Command, Event, HostError, Permission, Question, Shared, StopReason, TaskNews, Update, answered, label,
};
use crate::modes::{commands_of, modes_of, settings_of};
use crate::prompt::{Accepts, blocks};
use crate::sign_in::{SignInHow, SignInMethod};

type Waiting = Arc<Mutex<HashMap<u64, Responder<RequestPermissionResponse>>>>;
/// Questions waiting on the person, with which of their fields take several choices.
type Asking = Arc<Mutex<HashMap<u64, (Responder<CreateElicitationResponse>, HashMap<String, bool>)>>>;

/// What Silt says of itself in v1: what its v2 `initialize` says, as v1 has
/// it, so an agent that answers v2's with v1 is kept rather than started
/// again (see `Client::protocol_connector`).
pub(crate) fn initialize() -> Result<InitializeRequest, serde_json::Error> {
    // Silt shows a form of choices and text as a question; it has no browser to send the person to.
    let elicitation = ElicitationCapabilities::new().form(ElicitationFormCapabilities::new());
    // Every v2 client takes boolean settings; Silt leaves out any but choices.
    // It takes suggested prompts only over v2: the SDK reuses a connection
    // only when this says what v2's does as v1, which has no session
    // capabilities of its own to carry them.
    let session: ClientSessionCapabilities = serde_json::from_value(serde_json::json!({"configOptions": {"boolean": {}}}))?;
    // Silt runs an agent's terminal sign-in in a terminal of its own, shown on a card (ADR 0056).
    let capabilities =
        ClientCapabilities::new().elicitation(elicitation).session(session).auth(AuthCapabilities::new().terminal(true));
    Ok(InitializeRequest::new(ProtocolVersion::V1)
        .client_info(Implementation::new(crate::host::NAME, env!("CARGO_PKG_VERSION")))
        .client_capabilities(capabilities))
}

/// The ways to sign the agent in once it needs one. An agent that answered
/// Silt's v2 `initialize` in v1 may have read none of its v2 capabilities,
/// as Antigravity's `agy acp` and Claude's adapter 0.94 don't, and so listed
/// no method run in a terminal, which it lists only to a client that says it
/// runs them; such an agent is asked again with Silt's own v1 `initialize`.
async fn sign_in_methods(connection: &ConnectionTo<Agent>, listed: Vec<SignInMethod>) -> Vec<SignInMethod> {
    if listed.iter().any(|method| method.how != SignInHow::Agent) {
        return listed;
    }
    let Ok(request) = initialize() else {
        return listed;
    };
    match connection.send_request(request).block_task().await {
        Ok(again) => {
            let again = crate::sign_in::methods(&serde_json::to_value(&again.auth_methods).unwrap_or_default());
            let added: Vec<SignInMethod> =
                again.into_iter().filter(|method| listed.iter().all(|known| known.id != method.id)).collect();
            listed.into_iter().chain(added).collect()
        }
        Err(_) => listed,
    }
}

pub(crate) struct V1(pub(crate) Shared);

impl ConnectTo<Agent> for V1 {
    async fn connect_to(self, agent: impl ConnectTo<Client>) -> Result<(), agent_client_protocol::Error> {
        let Shared { start, events, commands } = self.0;
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
            .map(|mcp| {
                let env = mcp.env.iter().map(|(name, value)| EnvVariable::new(name, value)).collect();
                McpServer::Stdio(McpServerStdio::new("silt", mcp.command.clone()).args(mcp.args.clone()).env(env))
            })
            .collect();

        let updates = events.clone();
        let quiet = replaying.clone();
        let asked = events.clone();
        let parked = waiting.clone();
        let following: Arc<Mutex<Following>> = Arc::default();
        // An update can come before the session's answer is read, as Copilot
        // sends its commands once the session is open; it waits for Ready.
        let early: Arc<Mutex<Option<Vec<Event>>>> = Arc::new(Mutex::new(Some(Vec::new())));
        let held_back = early.clone();
        Client
            .v1()
            .name(crate::host::NAME)
            .on_receive_notification(
                async move |notification: SessionNotification, _connection| {
                    if !quiet.load(Ordering::SeqCst)
                        && let Some((agent, event)) = event_of(notification.update)
                    {
                        let followed = following.lock().expect("lock").follow(agent, event);
                        match held_back.lock().expect("lock").as_mut() {
                            Some(waiting) => waiting.extend(followed),
                            None => {
                                for event in followed {
                                    let _ = updates.send(event);
                                }
                            }
                        }
                    }
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_request(
                async move |request: RequestPermissionRequest, responder, _connection| {
                    let id = next_id.fetch_add(1, Ordering::SeqCst);
                    let options =
                        request.options.iter().map(|option| (option.option_id.to_string(), option.name.clone())).collect();
                    let option_kinds =
                        request.options.iter().map(|option| (option.option_id.to_string(), label(&option.kind))).collect();
                    let title = request.tool_call.fields.title.clone().unwrap_or_else(|| "a tool call".to_owned());
                    let tool_kind = request.tool_call.fields.kind.as_ref().map(label);
                    let tool_call_id = request.tool_call.tool_call_id.to_string();
                    parked.lock().expect("lock").insert(id, responder);
                    let _ =
                        asked.send(Event::Permission(Permission { id, title, options, tool_kind, tool_call_id, option_kinds }));
                    Ok(())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: CreateElicitationRequest, responder, _connection| {
                    let id = question_ids.fetch_add(1, Ordering::SeqCst);
                    let raw = serde_json::to_value(&request).unwrap_or_default();
                    match Question::of(id, &raw) {
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
            .connect_with(agent, async move |connection: ConnectionTo<Agent>| {
                let request = initialize().map_err(agent_client_protocol::Error::into_internal_error)?;
                let init = connection.send_request(request).block_task().await?;
                let Some(mut commands) = commands.lock().expect("lock").take() else {
                    return Ok(());
                };
                let agent = init.agent_info.as_ref().map_or_else(|| start.program.clone(), |info| info.name.clone());
                let methods = crate::sign_in::methods(&serde_json::to_value(&init.auth_methods).unwrap_or_default());
                if let Some(method) = start.sign_in_first.clone()
                    && let Err(error) = connection.send_request(AuthenticateRequest::new(method)).block_task().await
                {
                    if crate::sign_in::needed(&error) {
                        let _ = events.send(Event::SignIn(sign_in_methods(&connection, methods).await));
                        return Ok(());
                    }
                    return Err(error);
                }
                let takes = &init.agent_capabilities.prompt_capabilities;
                let accepts = Accepts { image: takes.image, embedded: takes.embedded_context };
                let loaded = match start.load.clone().filter(|_| init.agent_capabilities.load_session) {
                    Some(id) => {
                        replaying.store(true, Ordering::SeqCst);
                        let loaded = connection
                            .send_request(
                                LoadSessionRequest::new(SessionId::new(id.clone()), start.cwd.clone())
                                    .mcp_servers(mcp_servers.clone())
                                    .meta(crate::session_meta()),
                            )
                            .block_task()
                            .await;
                        replaying.store(false, Ordering::SeqCst);
                        match loaded {
                            Err(error) if crate::sign_in::needed(&error) => {
                                let _ = events.send(Event::SignIn(sign_in_methods(&connection, methods).await));
                                return Ok(());
                            }
                            // A session made on another machine can't be loaded
                            // here; the daemon primes a new one instead.
                            loaded => loaded.ok().map(|response| (id, response.modes, response.config_options)),
                        }
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
                            reports_work: false,
                        });
                        SessionId::new(id)
                    }
                    None => {
                        let created = match connection
                            .send_request(
                                NewSessionRequest::new(start.cwd.clone())
                                    .mcp_servers(mcp_servers.clone())
                                    .meta(crate::session_meta()),
                            )
                            .block_task()
                            .await
                        {
                            Err(error) if crate::sign_in::needed(&error) => {
                                let _ = events.send(Event::SignIn(sign_in_methods(&connection, methods).await));
                                return Ok(());
                            }
                            created => created?,
                        };
                        let _ = events.send(Event::Ready {
                            session: created.session_id.to_string(),
                            loaded: false,
                            agent,
                            modes: modes_of(created.modes),
                            settings: settings_of(created.config_options.unwrap_or_default()),
                            accepts,
                            reports_work: false,
                        });
                        created.session_id
                    }
                };
                for event in early.lock().expect("lock").take().unwrap_or_default() {
                    let _ = events.send(event);
                }
                let mut changing = Changing::default();
                while let Some(command) = commands.recv().await {
                    match command {
                        Command::Prompt { text, attachments } => {
                            changing.settled().await;
                            let request = PromptRequest::new(session.clone(), blocks(&text, &attachments, accepts));
                            let ended = events.clone();
                            let methods = methods.clone();
                            let asking = connection.clone();
                            // Sent here rather than in the task, so a mode set
                            // just before reaches the agent first.
                            let sent = connection.send_request(request);
                            connection.spawn(async move {
                                match sent.block_task().await {
                                    Ok(response) => {
                                        if let Some(usage) = &response.usage {
                                            let _ = ended.send(Event::TurnUsage {
                                                input: usage.input_tokens,
                                                output: usage.output_tokens,
                                            });
                                        }
                                        let _ = ended.send(Event::TurnEnded { reason: stop_reason(response.stop_reason) });
                                    }
                                    Err(error) if crate::sign_in::needed(&error) => {
                                        let _ = ended.send(Event::SignIn(sign_in_methods(&asking, methods).await));
                                        let _ = ended.send(Event::TurnEnded { reason: StopReason::Error });
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
                            let asked: Vec<_> =
                                asking.lock().expect("lock").drain().map(|(_, (responder, _))| responder).collect();
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
                            let answered = changing.asked();
                            let sent = connection.send_request(request);
                            connection.spawn(async move {
                                let event = match sent.block_task().await {
                                    Ok(_) => Event::ModeChanged(mode),
                                    Err(error) => Event::Refused(format!("the agent kept its mode: {error}")),
                                };
                                let _ = told.send(event);
                                drop(answered);
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
                            let answered = changing.asked();
                            let sent = connection.send_request(request);
                            connection.spawn(async move {
                                let event = match sent.block_task().await {
                                    Ok(response) => Event::Settings(settings_of(response.config_options)),
                                    Err(error) => Event::Refused(format!("the agent kept its setting: {error}")),
                                };
                                let _ = told.send(event);
                                drop(answered);
                                Ok(())
                            })?;
                        }
                        Command::Stop => break,
                    }
                }
                Ok(())
            })
            .await
    }
}

fn stop_reason(reason: agent_client_protocol::v1::schema::StopReason) -> StopReason {
    use agent_client_protocol::v1::schema::StopReason as Acp;
    match reason {
        Acp::EndTurn => StopReason::EndTurn,
        Acp::MaxTokens => StopReason::MaxTokens,
        Acp::MaxTurnRequests => StopReason::MaxTurnRequests,
        Acp::Refusal => StopReason::Refusal,
        Acp::Cancelled => StopReason::Cancelled,
        other => StopReason::Other(label(&other)),
    }
}

fn text_of(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(text) => text.text.clone(),
        _ => String::new(),
    }
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

/// The event an update is, and the subagent it is tagged with, as Copilot
/// tags its subagents' tool calls with `_meta["github.com/copilot"].agentId`.
fn event_of(update: SessionUpdate) -> Option<(Option<String>, Event)> {
    let meta = match &update {
        SessionUpdate::ToolCall(call) => call.meta.as_ref(),
        SessionUpdate::ToolCallUpdate(update) => update.meta.as_ref(),
        SessionUpdate::AgentMessageChunk(chunk) | SessionUpdate::AgentThoughtChunk(chunk) => chunk.meta.as_ref(),
        _ => None,
    };
    let agent = meta.and_then(|meta| meta.get("github.com/copilot")?.get("agentId")?.as_str().map(str::to_owned));
    let event = match update {
        SessionUpdate::CurrentModeUpdate(update) => Event::ModeChanged(update.current_mode_id.0.to_string()),
        SessionUpdate::ConfigOptionUpdate(update) => Event::Settings(settings_of(update.config_options)),
        SessionUpdate::AvailableCommandsUpdate(update) => Event::Commands(commands_of(update.available_commands)),
        SessionUpdate::UsageUpdate(update) => Event::Context { used: update.used, size: update.size },
        update => Event::Update(simplify(update)?),
    };
    Some((agent, event))
}

/// Work an agent on v1 names only in its own words, as Copilot does: the
/// subagents it starts and the shells it detaches.
#[derive(Debug, Default)]
struct Following {
    /// The call that started each detached shell, by the shell's id.
    detached: HashMap<String, String>,
    /// Subagent calls still running, oldest first.
    subagents: Vec<Subagent>,
}

#[derive(Debug)]
struct Subagent {
    call: String,
    /// The id its updates are tagged with, once one is.
    agent: Option<String>,
    /// The agent waits for it, so whatever it says meanwhile is the subagent's.
    waited: bool,
}

impl Following {
    fn follow(&mut self, agent: Option<String>, event: Event) -> Vec<Event> {
        if let Some(agent) = agent {
            // The first update of a subagent comes after the call that started it.
            let subagent = match self.subagents.iter().position(|subagent| subagent.agent.as_deref() == Some(&agent)) {
                Some(at) => Some(at),
                None => self.subagents.iter().position(|subagent| subagent.agent.is_none()),
            };
            return match subagent {
                Some(at) => {
                    self.subagents[at].agent = Some(agent);
                    vec![within(&self.subagents[at].call, event)]
                }
                None => vec![event],
            };
        }
        match &event {
            Event::Update(Update::ToolCall { id, input: Some(input), .. }) if input.get("agent_type").is_some() => {
                let waited = input.get("mode").and_then(Value::as_str) != Some("background");
                self.subagents.push(Subagent { call: id.clone(), agent: None, waited });
                let started = news(id, id, "subagent", "running");
                vec![event, started]
            }
            Event::Update(Update::AgentText(_) | Update::Thought(_)) => {
                match self.subagents.iter().rev().find(|subagent| subagent.waited) {
                    Some(subagent) => vec![within(&subagent.call, event)],
                    None => vec![event],
                }
            }
            Event::Update(Update::ToolUpdate { id, status: Some(status), task: None, .. })
                if matches!(status.as_str(), "completed" | "failed")
                    && self.subagents.iter().any(|subagent| subagent.call == *id) =>
            {
                self.subagents.retain(|subagent| subagent.call != *id);
                let ended = news(id, id, "subagent", status);
                vec![event, ended]
            }
            _ => self.follow_detached(event),
        }
    }

    /// News of shells an agent left running: "<command started in detached
    /// background with shellId: 0>", or "<command started in background
    /// with shellId: 0>" for an async one, and, when it reads one that
    /// ended, "<detached command with shellId: 0 completed with exit code
    /// 0>" or "<command with id: 0 exited with exit code 0>", or "<command
    /// with id: 0 stopped>" when it ends one. The news of its end is the
    /// call's that started it.
    fn follow_detached(&mut self, mut event: Event) -> Vec<Event> {
        let Event::Update(Update::ToolUpdate { id, status: Some(status), output: Some(output), task, .. }) = &mut event else {
            return vec![event];
        };
        if status != "completed" || task.is_some() {
            return vec![event];
        }
        if let Some(shell) = shell_id(output, "started in detached background with shellId: ")
            .or_else(|| shell_id(output, "<command started in background with shellId: "))
        {
            self.detached.insert(shell.clone(), id.clone());
            *task =
                Some(Box::new(TaskNews { id: Some(shell), kind: Some("background".to_owned()), status: "running".to_owned() }));
            return vec![event];
        }
        // Ended by the agent: "<command with id: 0 stopped>".
        if let Some(shell) =
            shell_id(output, "<command with id: ").filter(|shell| output.contains(&format!("id: {shell} stopped>")))
            && let Some(call) = self.detached.remove(&shell)
        {
            return vec![news(&call, &shell, "background", "stopped"), event];
        }
        let Some(shell) = shell_id(output, "detached command with shellId: ").or_else(|| {
            shell_id(output, "<command with id: ").filter(|shell| output.contains(&format!("id: {shell} exited with")))
        }) else {
            return vec![event];
        };
        let Some(call) = self.detached.remove(&shell) else {
            return vec![event];
        };
        let ended = output.split_once(" with exit code ").and_then(|(_, code)| code.split(|c: char| !c.is_ascii_digit()).next());
        let status = if ended == Some("0") { "completed" } else { "failed" };
        vec![news(&call, &shell, "background", status), event]
    }
}

/// `event` as what the subagent the call `call` started did.
fn within(call: &str, event: Event) -> Event {
    match event {
        Event::Update(update) => Event::Update(Update::Within { parent: call.to_owned(), update: Box::new(update) }),
        other => other,
    }
}

/// News of work the call `call` left running beside the turn.
fn news(call: &str, task: &str, kind: &str, status: &str) -> Event {
    let news = TaskNews { id: Some(task.to_owned()), kind: Some(kind.to_owned()), status: status.to_owned() };
    Event::Update(Update::ToolUpdate {
        id: call.to_owned(),
        status: None,
        output: None,
        title: None,
        input: None,
        task: Some(Box::new(news)),
    })
}

/// The shell id `output` names right after `before`.
fn shell_id(output: &str, before: &str) -> Option<String> {
    let (_, rest) = output.split_once(before)?;
    let id: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (!id.is_empty()).then_some(id)
}

/// Claude's adapter says in `_meta.claudeCode` which subagent call an
/// update is inside, as `parentToolUseId`.
fn parent_of(meta: Option<&Meta>) -> Option<String> {
    meta?.get("claudeCode")?.get("parentToolUseId")?.as_str().map(str::to_owned)
}

/// What a call's update says it left running or saw end: Claude's adapter
/// passes the tool's structured result as `_meta.claudeCode.toolResponse`,
/// naming the subagent (`agentId`) or background command
/// (`backgroundTaskId`) it started or the finished subagent `SendMessage`
/// set running again (`resumedAgentId`), and the exit of a background command as
/// `_meta.terminal_exit` once it ends. Claude Code gives that no exit code:
/// the command ran its course.
fn task_of(meta: Option<&Meta>) -> Option<TaskNews> {
    let meta = meta?;
    let response = meta.get("claudeCode").and_then(|claude| claude.get("toolResponse"));
    let field = |key: &str| response.and_then(|response| response.get(key)).and_then(Value::as_str).map(str::to_owned);
    if let Some(agent) = field("agentId").or_else(|| field("resumedAgentId")) {
        let status = match field("status").as_deref() {
            None | Some("async_launched") => "running".to_owned(),
            Some(status) => status.to_owned(),
        };
        return Some(TaskNews { id: Some(agent), kind: Some("subagent".into()), status });
    }
    if let Some(background) = field("backgroundTaskId") {
        return Some(TaskNews { id: Some(background), kind: Some("background".into()), status: "running".into() });
    }
    let exit = meta.get("terminal_exit")?;
    let code = exit.get("exit_code").and_then(Value::as_i64);
    let status = if code.is_none_or(|code| code == 0) { "completed" } else { "failed" };
    Some(TaskNews { id: None, kind: None, status: status.to_owned() })
}

fn simplify(update: SessionUpdate) -> Option<Update> {
    let parent = match &update {
        SessionUpdate::AgentMessageChunk(chunk) | SessionUpdate::AgentThoughtChunk(chunk) => parent_of(chunk.meta.as_ref()),
        SessionUpdate::ToolCall(call) => parent_of(call.meta.as_ref()),
        SessionUpdate::ToolCallUpdate(update) => parent_of(update.meta.as_ref()),
        _ => None,
    };
    let simple = simplify_own(update)?;
    Some(match parent {
        Some(parent) => Update::Within { parent, update: Box::new(simple) },
        None => simple,
    })
}

fn simplify_own(update: SessionUpdate) -> Option<Update> {
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
            task: task_of(update.meta.as_ref()).map(Box::new),
            id: update.tool_call_id.to_string(),
            status: update.fields.status.as_ref().map(label),
            output: update.fields.content.as_deref().and_then(tool_output).or_else(|| {
                update.fields.raw_output.as_ref().map(|output| match output {
                    // A message, as Claude's tools answer, rather than data to show as JSON.
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
            }),
            title: update.fields.title,
            input: update.fields.raw_input,
        },
        SessionUpdate::Plan(plan) => {
            Update::Plan(plan.entries.into_iter().map(|entry| (entry.content, label(&entry.status))).collect())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::v1::schema::{ContentChunk, ToolCall, ToolCallUpdate, ToolCallUpdateFields};
    use serde_json::json;

    use super::*;

    fn meta(value: Value) -> Option<Meta> {
        value.as_object().cloned()
    }

    /// As Copilot (1.0.105) lists its commands, once its session opens.
    #[test]
    fn the_commands_an_agent_offers_are_named_with_what_they_take() {
        let update = json!({"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "add-dir", "description": "Allow file access to a directory", "input": {"hint": "directory"}},
            {"name": "compact", "description": "Summarize the conversation", "input": null}
        ]});
        let Some((None, Event::Commands(commands))) = event_of(serde_json::from_value(update).expect("an update")) else {
            panic!("the commands");
        };
        let said: Vec<(&str, Option<&str>)> =
            commands.iter().map(|command| (command.name.as_str(), command.hint.as_deref())).collect();
        assert_eq!(said, [("add-dir", Some("directory")), ("compact", None)]);
    }

    #[test]
    fn a_subagent_s_updates_are_kept_inside_the_call_that_started_it() {
        let mut call = ToolCall::new("t2", "Read");
        call.meta = meta(json!({"claudeCode": {"parentToolUseId": "t1", "toolName": "Read"}}));
        let Some(Update::Within { parent, update }) = simplify(SessionUpdate::ToolCall(call)) else {
            panic!("inside its call");
        };
        assert_eq!(parent, "t1");
        assert!(matches!(*update, Update::ToolCall { ref id, .. } if id == "t2"));
        let mut chunk = ContentChunk::new(ContentBlock::from("found it"));
        chunk.meta = meta(json!({"claudeCode": {"parentToolUseId": "t1"}}));
        assert!(matches!(simplify(SessionUpdate::AgentMessageChunk(chunk)), Some(Update::Within { .. })));
        let own = ContentChunk::new(ContentBlock::from("mine"));
        assert_eq!(simplify(SessionUpdate::AgentMessageChunk(own)), Some(Update::AgentText("mine".into())));
    }

    /// Copilot (1.0.104) detaching a shell and reading it once it ended, as recorded.
    #[test]
    fn a_shell_an_agent_detached_runs_until_it_reads_that_it_ended() {
        let completed = |id: &str, output: &str| {
            Event::Update(Update::ToolUpdate {
                id: id.into(),
                status: Some("completed".into()),
                output: Some(output.into()),
                title: None,
                input: None,
                task: None,
            })
        };
        let news = |event: &Event| match event {
            Event::Update(Update::ToolUpdate { id, task: Some(task), .. }) => {
                Some((id.clone(), task.id.clone(), task.status.clone()))
            }
            _ => None,
        };
        let mut following = Following::default();
        let started = following.follow(None, completed("t1", "<command started in detached background with shellId: 0>"));
        assert_eq!(news(&started[0]), Some(("t1".into(), Some("0".into()), "running".into())));
        let read = following
            .follow(None, completed("r1", "Output:\nCOPILOT_BG\n<detached command with shellId: 0 completed with exit code 0>"));
        assert_eq!(news(&read[0]), Some(("t1".into(), Some("0".into()), "completed".into())), "the news is the starting call's");
        assert!(matches!(&read[1], Event::Update(Update::ToolUpdate { id, task: None, .. }) if id == "r1"));
        let failed = following.follow(None, completed("t2", "<command started in detached background with shellId: 3>"));
        assert_eq!(failed.len(), 1);
        let ended = following.follow(None, completed("r2", "<detached command with shellId: 3 completed with exit code 2>"));
        assert_eq!(news(&ended[0]).map(|news| news.2), Some("failed".into()));
        let unknown = following.follow(None, completed("r3", "<detached command with shellId: 9 completed with exit code 0>"));
        assert_eq!(unknown.len(), 1, "a shell it never saw start is left alone");
        // Copilot (1.0.112) ending one with stop_bash.
        following.follow(None, completed("t3", "<command started in detached background with shellId: webdev>"));
        let stopped = following.follow(None, completed("s1", "<command with id: webdev stopped>"));
        assert_eq!(news(&stopped[0]), Some(("t3".into(), Some("webdev".into()), "stopped".into())));
        assert_eq!(following.follow(None, completed("s2", "<command with id: webdev stopped>")).len(), 1, "it ends once");
        // Copilot's async shells, which aren't detached, as recorded.
        let started = following.follow(None, completed("t4", "<command started in background with shellId: gate8>"));
        assert_eq!(news(&started[0]), Some(("t4".into(), Some("gate8".into()), "running".into())));
        let still = following.follow(None, completed("r4", "Running gate\n"));
        assert_eq!(still.len(), 1, "reading it while it runs ends nothing");
        let read = following.follow(None, completed("r5", "ok\n<command with id: gate8 exited with exit code 0>"));
        assert_eq!(news(&read[0]), Some(("t4".into(), Some("gate8".into()), "completed".into())));
        following.follow(None, completed("t5", "<command started in background with shellId: 29>"));
        let piped = following.follow(None, completed("r6", ":\n<command with id: 29 exited with exit code 141>"));
        assert_eq!(news(&piped[0]).map(|news| news.2), Some("failed".into()));
    }

    /// Copilot (1.0.104) running an explore subagent, as recorded: its tool
    /// calls are tagged with its own id, and its words aren't tagged at all.
    #[test]
    fn a_subagent_copilot_tags_by_its_own_id_is_kept_inside_the_call_that_started_it() {
        let mut following = Following::default();
        let update = |json: Value| event_of(serde_json::from_value(json).expect("an update")).expect("an event");
        let (agent, call) = update(
            json!({"sessionUpdate": "tool_call", "toolCallId": "task-1", "title": "Count README lines", "kind": "other", "status": "pending", "rawInput": {"name": "readme-lines", "prompt": "How many lines?", "agent_type": "explore", "description": "Count README lines"}}),
        );
        let started = following.follow(agent, call);
        assert!(
            matches!(&started[1], Event::Update(Update::ToolUpdate { id, task: Some(task), .. }) if id == "task-1" && task.kind.as_deref() == Some("subagent") && task.status == "running")
        );
        let tagged = json!({"github.com/copilot": {"agentId": "715dcc7e"}});
        let (agent, run) = update(
            json!({"sessionUpdate": "tool_call", "toolCallId": "wc-1", "title": "Count lines", "kind": "execute", "status": "pending", "rawInput": {"command": "wc -l README.md"}, "_meta": tagged}),
        );
        assert_eq!(agent.as_deref(), Some("715dcc7e"));
        assert!(
            matches!(&following.follow(agent, run)[..], [Event::Update(Update::Within { parent, .. })] if parent == "task-1")
        );
        let (agent, said) = update(
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "README.md has 3 lines."}}),
        );
        assert!(
            matches!(&following.follow(agent, said)[..], [Event::Update(Update::Within { parent, .. })] if parent == "task-1"),
            "the agent waits on it"
        );
        let (agent, done) = update(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "task-1", "status": "completed", "content": [{"type": "content", "content": {"type": "text", "text": "README.md has 3 lines."}}]}),
        );
        let ended = following.follow(agent, done);
        assert!(matches!(&ended[1], Event::Update(Update::ToolUpdate { task: Some(task), .. }) if task.status == "completed"));
        let (agent, own) =
            update(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "lines counted"}}));
        assert!(matches!(&following.follow(agent, own)[..], [Event::Update(Update::AgentText(text))] if text == "lines counted"));
    }

    #[test]
    fn a_call_s_update_names_what_it_left_running_and_when_that_ended() {
        let news = |value: Value| {
            let mut update = ToolCallUpdate::new("t1", ToolCallUpdateFields::new());
            update.meta = meta(value);
            match simplify(SessionUpdate::ToolCallUpdate(update)) {
                Some(Update::ToolUpdate { task, .. }) => task,
                other => panic!("an update, not {other:?}"),
            }
        };
        let background = TaskNews { id: Some("b3r".into()), kind: Some("background".into()), status: "running".into() };
        assert_eq!(
            news(json!({"claudeCode": {"toolResponse": {"backgroundTaskId": "b3r", "stdout": ""}}})),
            Some(Box::new(background))
        );
        let subagent = news(json!({"claudeCode": {"toolResponse": {"agentId": "ad7", "status": "completed"}}}));
        assert_eq!(subagent.map(|news| (news.kind, news.status)), Some((Some("subagent".into()), "completed".into())));
        let resumed = news(json!({"claudeCode": {"toolResponse": {"success": true, "resumedAgentId": "ad7"}}}));
        assert_eq!(resumed.map(|news| (news.id, news.status)), Some((Some("ad7".into()), "running".into())));
        let exited = news(json!({"terminal_exit": {"exit_code": 1, "terminal_id": "t1"}}));
        assert_eq!(exited.map(|news| news.status), Some("failed".into()));
        let ran = news(json!({"terminal_exit": {"exit_code": null, "signal": null, "terminal_id": "t1"}}));
        assert_eq!(ran.map(|news| news.status), Some("completed".into()), "Claude Code gives a background command no code");
        assert_eq!(news(json!({"claudeCode": {"toolName": "Bash"}})), None);
    }
}
