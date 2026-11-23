//! A session with an agent that speaks ACP v2, as Claude's adapter does: a
//! prompt is answered once the agent takes it in, and the agent says when
//! it works and when it stops, for work it takes up by itself between
//! prompts too, such as Claude's once a background task ends.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::{MaybeUndefined, ProtocolVersion};
use agent_client_protocol::v2::schema::{
    self as v2, CancelSessionNotification, CancelledPermissionOutcome, ClientAuthCapabilities, ClientCapabilities,
    ClientSessionCapabilities, ContentBlock, CreateElicitationRequest, CreateElicitationResponse, ElicitationCapabilities,
    ElicitationFormCapabilities, EnvVariable, Implementation, InitializeRequest, LoginAuthRequest, McpServer, McpServerStdio,
    NewSessionRequest, PromptRequest, PromptSuggestionsCapabilities, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, RequestPermissionSubject, ResumeSessionRequest, SelectedPermissionOutcome, SessionConfigValueId,
    SessionId, SessionMessage, SessionUpdate, SetSessionConfigOptionRequest, StateUpdate, SubagentUpdate,
    TerminalAuthCapabilities, ToolCallContent, ToolCallStatus, ToolCallUpdate, UpdateSessionNotification,
};
use agent_client_protocol::v2::{ConnectionTo, Responder};
use agent_client_protocol::{Agent, Client, ConnectTo};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;

use crate::host::{
    Changing, Command, Event, HostError, Permission, Question, Shared, StopReason, TaskNews, Update, answered, label,
};
use crate::modes::{Setting, commands_v2, modes_in, settings_v2, with_change};
use crate::prompt::{Accepts, blocks_v2};

type Waiting = Arc<Mutex<HashMap<u64, Responder<RequestPermissionResponse>>>>;
/// Questions waiting on the person, with which of their fields take several choices.
type Asking = Arc<Mutex<HashMap<u64, (Responder<CreateElicitationResponse>, HashMap<String, bool>)>>>;

pub(crate) struct V2(pub(crate) Shared);

impl ConnectTo<Agent> for V2 {
    async fn connect_to(self, agent: impl ConnectTo<Client>) -> Result<(), agent_client_protocol::Error> {
        let Shared { start, events, commands } = self.0;
        let reading: Arc<Mutex<Reading>> = Arc::default();
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
        let read = reading.clone();
        let asked = events.clone();
        let parked = waiting.clone();
        Client
            .v2()
            .name(crate::host::NAME)
            .on_receive_notification(
                async move |notification: UpdateSessionNotification, _connection: ConnectionTo<Agent>| {
                    for event in read.lock().expect("lock").read(&notification.session_id, notification.update) {
                        let _ = updates.send(event);
                    }
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_request(
                async move |request: RequestPermissionRequest, responder, _connection: ConnectionTo<Agent>| {
                    let id = next_id.fetch_add(1, Ordering::SeqCst);
                    let permission = permission(id, request);
                    parked.lock().expect("lock").insert(id, responder);
                    let _ = asked.send(Event::Permission(permission));
                    Ok(())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: CreateElicitationRequest, responder, _connection: ConnectionTo<Agent>| {
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
                let capabilities = ClientCapabilities::new()
                    .elicitation(ElicitationCapabilities::new().form(ElicitationFormCapabilities::new()))
                    .auth(ClientAuthCapabilities::new().terminal(TerminalAuthCapabilities::new()))
                    .session(ClientSessionCapabilities::new().prompt_suggestions(PromptSuggestionsCapabilities::new()));
                let request = InitializeRequest::new(
                    ProtocolVersion::V2,
                    Implementation::new(crate::host::NAME, env!("CARGO_PKG_VERSION")),
                )
                .capabilities(capabilities);
                let init = connection.send_request(request).block_task().await?;
                let steers = crate::steering::supported(init.meta.as_ref());
                let methods = crate::sign_in::methods(&serde_json::to_value(&init.auth_methods).unwrap_or_default());
                reading.lock().expect("lock").sign_in.clone_from(&methods);
                if let Some(method) = start.sign_in_first.clone()
                    && let Err(error) = connection.send_request(LoginAuthRequest::new(method)).block_task().await
                {
                    if crate::sign_in::needed(&error) {
                        let _ = events.send(Event::SignIn(methods));
                        return Ok(());
                    }
                    return Err(error);
                }
                let Some(mut commands) = commands.lock().expect("lock").take() else {
                    return Ok(());
                };
                let takes = init.capabilities.session.as_ref().and_then(|session| session.prompt.as_ref());
                let accepts = Accepts {
                    image: takes.is_some_and(|takes| takes.image.is_some()),
                    embedded: takes.is_some_and(|takes| takes.embedded_context.is_some()),
                };
                let resumed = match start.load.clone() {
                    Some(id) => match connection
                        .send_request(
                            ResumeSessionRequest::new(SessionId::new(id.clone()), start.cwd.clone())
                                .mcp_servers(mcp_servers.clone())
                                .meta(crate::session_meta()),
                        )
                        .block_task()
                        .await
                    {
                        Err(error) if crate::sign_in::needed(&error) => {
                            let _ = events.send(Event::SignIn(methods));
                            return Ok(());
                        }
                        // A session made on another machine can't be resumed
                        // here; the daemon primes a new one instead.
                        resumed => resumed
                            .ok()
                            .map(|response| (SessionId::new(id), response.config_options, response.available_commands)),
                    },
                    None => None,
                };
                let loaded = resumed.is_some();
                let (session, options, offered) = match resumed {
                    Some(resumed) => resumed,
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
                                let _ = events.send(Event::SignIn(methods));
                                return Ok(());
                            }
                            created => created?,
                        };
                        (created.session_id, created.config_options, created.available_commands)
                    }
                };
                let settings = settings_v2(options);
                let modes = reading.lock().expect("lock").begin(&session, &settings);
                let _ = events.send(Event::Ready {
                    session: session.to_string(),
                    loaded,
                    agent: init.info.name.clone(),
                    modes,
                    settings,
                    accepts,
                    reports_work: true,
                    steers,
                });
                // Later changes come as updates.
                let _ = events.send(Event::Commands(commands_v2(offered)));
                let mut changing = Changing::default();
                while let Some(command) = commands.recv().await {
                    match command {
                        Command::Prompt { text, attachments } => {
                            changing.settled().await;
                            let prompt = blocks_v2(&text, &attachments, accepts)
                                .map_err(agent_client_protocol::Error::into_internal_error)?;
                            let failed = events.clone();
                            let methods = methods.clone();
                            // Sent here rather than in the task, so a mode set
                            // just before reaches the agent first.
                            let sent = connection.send_request(PromptRequest::new(session.clone(), prompt));
                            connection.spawn(async move {
                                // Taken in; the agent says when it is done.
                                match sent.block_task().await {
                                    Err(error) if crate::sign_in::needed(&error) => {
                                        let _ = failed.send(Event::SignIn(methods));
                                        let _ = failed.send(Event::TurnEnded { reason: StopReason::Error });
                                    }
                                    Err(error) => {
                                        let _ = failed.send(Event::Failed(Arc::new(HostError::Protocol(error))));
                                        let _ = failed.send(Event::TurnEnded { reason: StopReason::Error });
                                    }
                                    Ok(_) => {}
                                }
                                Ok(())
                            })?;
                        }
                        Command::Cancel => {
                            connection.send_notification(CancelSessionNotification::new(session.clone()))?;
                            // A subagent left running in the background outlives
                            // its parent's cancel; each is stopped by its own.
                            for child in reading.lock().expect("lock").running_children() {
                                connection.send_notification(CancelSessionNotification::new(child))?;
                            }
                            // The protocol asks clients to answer every pending
                            // permission request once they cancel.
                            let parked: Vec<_> = waiting.lock().expect("lock").drain().map(|(_, responder)| responder).collect();
                            for responder in parked {
                                responder.respond(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled(
                                    CancelledPermissionOutcome::new(),
                                )))?;
                            }
                            let asked: Vec<_> =
                                asking.lock().expect("lock").drain().map(|(_, (responder, _))| responder).collect();
                            for responder in asked {
                                responder.respond(
                                    serde_json::from_value(serde_json::json!({"action": "cancel"})).expect("a cancel action"),
                                )?;
                            }
                        }
                        Command::Steer(text) => {
                            let request = crate::steering::request(&session.to_string(), &text)?;
                            let (told, sent) = (events.clone(), connection.send_request(request));
                            connection.spawn(async move {
                                let outcome = crate::steering::outcome(sent.block_task().await);
                                let _ = told.send(Event::Steered { text, outcome });
                                Ok(())
                            })?;
                        }
                        Command::Answer { id, option } => {
                            let responder = waiting.lock().expect("lock").remove(&id);
                            if let Some(responder) = responder {
                                let outcome = match option {
                                    Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option)),
                                    None => RequestPermissionOutcome::Cancelled(CancelledPermissionOutcome::new()),
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
                            let Some(setting) = reading.lock().expect("lock").mode_setting() else {
                                let _ = events.send(Event::Refused("the agent has no modes".to_owned()));
                                continue;
                            };
                            let request = SetSessionConfigOptionRequest::new(
                                session.clone(),
                                setting.clone(),
                                SessionConfigValueId::new(mode.clone()),
                            );
                            let (told, read) = (events.clone(), reading.clone());
                            let answered = changing.asked();
                            let sent = connection.send_request(request);
                            connection.spawn(async move {
                                match sent.block_task().await {
                                    Ok(response) => {
                                        // As v1 takes a mode change answered without error as made.
                                        let settings = with_change(settings_v2(response.config_options), &setting, &mode);
                                        for event in read.lock().expect("lock").settings(settings) {
                                            let _ = told.send(event);
                                        }
                                    }
                                    Err(error) => {
                                        let _ = told.send(Event::Refused(format!("the agent kept its mode: {error}")));
                                    }
                                }
                                drop(answered);
                                Ok(())
                            })?;
                        }
                        Command::SetSetting { id, value } => {
                            let request = SetSessionConfigOptionRequest::new(
                                session.clone(),
                                id.clone(),
                                SessionConfigValueId::new(value.clone()),
                            );
                            let (told, read) = (events.clone(), reading.clone());
                            let answered = changing.asked();
                            let sent = connection.send_request(request);
                            connection.spawn(async move {
                                match sent.block_task().await {
                                    Ok(response) => {
                                        let settings = with_change(settings_v2(response.config_options), &id, &value);
                                        for event in read.lock().expect("lock").settings(settings) {
                                            let _ = told.send(event);
                                        }
                                    }
                                    Err(error) => {
                                        let _ = told.send(Event::Refused(format!("the agent kept its setting: {error}")));
                                    }
                                }
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

/// A permission request as Silt asks it: the call it is for, when it is for
/// one, says what kind of tool asks.
fn permission(id: u64, request: RequestPermissionRequest) -> Permission {
    let options = request.options.iter().map(|option| (option.option_id.to_string(), option.name.clone())).collect();
    let option_kinds = request.options.iter().map(|option| (option.option_id.to_string(), label(&option.kind))).collect();
    let (tool_call_id, tool_kind) = match &request.subject {
        Some(RequestPermissionSubject::ToolCall(subject)) => {
            let call = &subject.tool_call;
            let kind = match &call.kind {
                MaybeUndefined::Value(kind) => Some(label(kind)),
                _ => None,
            };
            (call.tool_call_id.to_string(), kind)
        }
        Some(RequestPermissionSubject::Command(command)) => {
            (command.tool_call_id.as_ref().map(ToString::to_string).unwrap_or_default(), Some("execute".to_owned()))
        }
        _ => (String::new(), None),
    };
    Permission { id, title: request.title, options, tool_kind, tool_call_id, option_kinds }
}

/// What the agent said of its session so far, to read v2's updates as
/// Silt's events: v2 names each message, may send one whole again, and
/// announces a tool call by its first update.
#[derive(Debug, Default)]
pub(crate) struct Reading {
    session: Option<SessionId>,
    /// How the agent says it can be signed in, for a turn that ends needing it.
    sign_in: Vec<crate::SignInMethod>,
    /// Whether the agent said it works and hasn't said it stopped since.
    working: bool,
    /// Each message's and thought's text so far, by its id.
    said: HashMap<String, String>,
    calls: HashSet<String>,
    /// What each call printed so far, from its terminal or its content.
    printed: HashMap<String, String>,
    /// The call each terminal runs for.
    terminals: HashMap<String, String>,
    /// The setting that is the agent's mode, and its value.
    mode: Option<(String, String)>,
    /// Each call's input as last said, by the call.
    inputs: HashMap<String, Value>,
    /// Commands left running in the background: the agent's id for each,
    /// by the call that started it.
    background: HashMap<String, String>,
    /// The agent's subagents, by their sessions.
    children: HashMap<String, Child>,
}

/// A subagent, as its parent's updates say it: v2 runs each in a session
/// of its own (ACP's subagents RFD).
#[derive(Debug, Default)]
struct Child {
    /// The call that started it, which the prompt the parent sent it names.
    call: Option<String>,
    /// What tells it from its siblings: Claude's adapter gives its name,
    /// else its task's description, else its kind.
    title: Option<String>,
    /// Its role, as Claude's `subagent_type`.
    role: Option<String>,
    /// The session that started it: the agent's own, or a subagent's.
    parent: Option<String>,
    /// Commands it left running in the background, by their calls.
    background: HashSet<String>,
    ended: bool,
    /// Its end was told, which a later run doesn't undo.
    end_told: bool,
}

impl Reading {
    /// The session is up, with these settings; its modes, if one of them is.
    pub(crate) fn begin(&mut self, session: &SessionId, settings: &[Setting]) -> Option<crate::Modes> {
        self.session = Some(session.clone());
        let (id, modes) = modes_in(settings)?;
        self.mode = Some((id, modes.current.clone()));
        Some(modes)
    }

    fn mode_setting(&self) -> Option<String> {
        self.mode.as_ref().map(|(id, _)| id.clone())
    }

    /// The settings as they now stand, and the mode if it changed.
    pub(crate) fn settings(&mut self, settings: Vec<Setting>) -> Vec<Event> {
        let mut events = Vec::new();
        if let Some((id, current)) = &mut self.mode
            && let Some(mode) = settings.iter().find(|setting| setting.id == *id)
            && mode.current != *current
        {
            current.clone_from(&mode.current);
            events.push(Event::ModeChanged(mode.current.clone()));
        }
        events.push(Event::Settings(settings));
        events
    }

    /// The subagents still running, by their sessions, which a cancel of
    /// their own stops.
    pub(crate) fn running_children(&self) -> Vec<SessionId> {
        self.children.iter().filter(|(_, child)| !child.ended).map(|(id, _)| SessionId::new(id.clone())).collect()
    }

    /// What an update of `session` says, as Silt's events. A subagent's
    /// session is recorded inside the call that started it; others are left
    /// out.
    pub(crate) fn read(&mut self, session: &SessionId, update: SessionUpdate) -> Vec<Event> {
        if self.session.as_ref() != Some(session) {
            return self.child(session, update);
        }
        match update {
            SessionUpdate::SubagentUpdate(update) => self.subagent(session, update),
            SessionUpdate::SessionMessage(message) => self.message(message),
            SessionUpdate::ConfigOptionUpdate(update) => self.settings(settings_v2(update.config_options)),
            SessionUpdate::AvailableCommandsUpdate(update) => vec![Event::Commands(commands_v2(update.available_commands))],
            SessionUpdate::UsageUpdate(usage) => vec![Event::Context { used: usage.used, size: usage.size }],
            SessionUpdate::PromptSuggestionsUpdate(update) => vec![Event::Suggestion(
                update.suggestions.first().map(|suggestion| all_text(&suggestion.prompt)).and_then(crate::host::suggested),
            )],
            update => self.said(update),
        }
    }

    /// What a subagent's own session says, inside the call that started it.
    fn child(&mut self, session: &SessionId, update: SessionUpdate) -> Vec<Event> {
        let id = session.to_string();
        let Some(parent) = self.children.get(&id).and_then(|child| child.call.clone()) else {
            return Vec::new();
        };
        let events = match update {
            SessionUpdate::StateUpdate(_) => return Vec::new(),
            // A subagent may start subagents of its own (Claude Code's
            // `spawnDepth`), announced on its session.
            SessionUpdate::SubagentUpdate(update) => return self.subagent(session, update),
            SessionUpdate::SessionMessage(message) => return self.message(message),
            update => self.said(update),
        };
        for event in &events {
            if let Event::Update(Update::ToolUpdate { id: call, task: Some(news), .. }) = event
                && news.kind.as_deref() == Some("background")
            {
                // Only a command whose end its terminal tells holds the
                // subagent: the adapter tells no end of PowerShell's.
                let told = self.terminals.values().any(|of| of == call);
                let child = self.children.get_mut(&id).expect("looked up above");
                if news.status == "running" && told {
                    child.background.insert(call.clone());
                } else {
                    child.background.remove(call);
                }
            }
        }
        inside(Some(parent), events)
    }

    /// Where the work of `session` is recorded: `Some(None)` for the
    /// agent's own, inside the call that started it for a subagent's.
    fn place(&self, session: &str) -> Option<Option<String>> {
        if self.session.as_ref().is_some_and(|own| own.to_string() == session) {
            return Some(None);
        }
        self.children.get(session)?.call.clone().map(Some)
    }

    /// Whether what the subagent `session` started still runs: commands it
    /// left in the background, or subagents of its own.
    fn still_working(&self, session: &str) -> bool {
        let child = &self.children[session];
        !child.background.is_empty()
            || self.children.values().any(|other| other.parent.as_deref() == Some(session) && !other.ended)
    }

    /// News of a subagent, which `announcer` started: what it is, and how
    /// it ended. Each of its runs ends idle, and one that ended its turn
    /// while what it started still runs isn't its end: Claude Code takes it
    /// up again once that ends.
    fn subagent(&mut self, announcer: &SessionId, update: SubagentUpdate) -> Vec<Event> {
        let id = update.session_id.to_string();
        let child = self.children.entry(id.clone()).or_default();
        child.parent.get_or_insert_with(|| announcer.to_string());
        if let MaybeUndefined::Value(title) = update.title {
            child.title = Some(title);
        }
        if let MaybeUndefined::Value(role) = update.description {
            child.role = Some(role);
        }
        let idle = match update.state {
            MaybeUndefined::Value(StateUpdate::Idle(idle)) => idle,
            MaybeUndefined::Value(StateUpdate::Running(_) | StateUpdate::RequiresAction(_)) => {
                child.ended = false;
                return Vec::new();
            }
            _ => return Vec::new(),
        };
        let status = match idle.stop_reason {
            None | Some(v2::StopReason::EndTurn) => "completed",
            Some(v2::StopReason::Cancelled) => "stopped",
            Some(_) => "failed",
        };
        if status == "completed" && self.still_working(&id) {
            return Vec::new();
        }
        let child = self.children.get_mut(&id).expect("entered above");
        child.ended = true;
        if std::mem::replace(&mut child.end_told, true) {
            return Vec::new();
        }
        let Some(call) = child.call.clone() else { return Vec::new() };
        let Some(place) = self.place(&announcer.to_string()) else { return Vec::new() };
        inside(place, vec![news(&call, &id, "subagent", status)])
    }

    /// A message between the agent, or a subagent, and a subagent it
    /// started: the prompt it gave one, which names the call that started
    /// it, or what one reported back.
    fn message(&mut self, message: SessionMessage) -> Vec<Event> {
        let (Some(sender), Some(recipient)) = (message.sender_session_id.take(), message.recipient_session_id.take()) else {
            return Vec::new();
        };
        let (sender, recipient) = (sender.to_string(), recipient.to_string());
        let text = match &message.content {
            MaybeUndefined::Value(content) => all_text(content),
            _ => String::new(),
        };
        let reply = self.children.get(&sender).filter(|child| child.parent.as_deref() == Some(recipient.as_str()));
        if let Some(call) = reply.and_then(|child| child.call.clone()) {
            let Some(place) = self.place(&recipient) else { return Vec::new() };
            let ended = Update::ToolUpdate {
                id: call,
                status: Some("completed".to_owned()),
                output: Some(text),
                title: None,
                input: None,
                task: None,
            };
            return inside(place, vec![Event::Update(ended)]);
        }
        let Some(place) = self.place(&sender) else { return Vec::new() };
        if self.place(&recipient) == Some(None) {
            return Vec::new();
        }
        let child = self.children.entry(recipient.clone()).or_default();
        // A later message to a subagent already started goes on its work.
        if child.call.is_some() {
            return Vec::new();
        }
        child.parent.get_or_insert(sender);
        let call = message.message_id.to_string();
        child.call = Some(call.clone());
        let input = serde_json::json!({"title": child.title, "role": child.role, "prompt": text});
        self.calls.insert(call.clone());
        let started = vec![
            Event::Update(Update::ToolCall {
                id: call.clone(),
                title: "Agent".to_owned(),
                kind: "other".to_owned(),
                input: Some(input),
            }),
            news(&call, &recipient, "subagent", "running"),
        ];
        inside(place, started)
    }

    /// What the agent itself did: its words, thoughts, tool calls and plan.
    fn said(&mut self, update: SessionUpdate) -> Vec<Event> {
        match update {
            SessionUpdate::StateUpdate(StateUpdate::Running(_) | StateUpdate::RequiresAction(_)) => {
                self.working = true;
                vec![Event::Working]
            }
            // An idle the agent reports before it worked, as when a session
            // opens, ends nothing.
            SessionUpdate::StateUpdate(StateUpdate::Idle(idle)) if self.working => {
                self.working = false;
                self.said.clear();
                let mut events = Vec::new();
                if let Some(usage) = idle.usage {
                    events.push(Event::TurnUsage { input: usage.input_tokens, output: usage.output_tokens });
                }
                if let Some(v2::StopReason::Error(stopped)) = &idle.stop_reason
                    && stopped.error.as_deref().is_some_and(crate::sign_in::needed)
                {
                    events.push(Event::SignIn(self.sign_in.clone()));
                }
                events.push(Event::TurnEnded { reason: idle.stop_reason.map_or(StopReason::EndTurn, stop_reason) });
                events
            }
            SessionUpdate::AgentMessageChunk(chunk) => self
                .added(format!("message {}", chunk.message_id), &text_of(&chunk.content), true)
                .map(Update::AgentText)
                .into_iter()
                .map(Event::Update)
                .collect(),
            SessionUpdate::AgentMessage(message) => match message.content {
                MaybeUndefined::Value(content) => self
                    .added(format!("message {}", message.message_id), &all_text(&content), false)
                    .map(Update::AgentText)
                    .into_iter()
                    .map(Event::Update)
                    .collect(),
                _ => Vec::new(),
            },
            SessionUpdate::AgentThoughtChunk(chunk) => self
                .added(format!("thought {}", chunk.message_id), &text_of(&chunk.content), true)
                .map(Update::Thought)
                .into_iter()
                .map(Event::Update)
                .collect(),
            SessionUpdate::AgentThought(thought) => match thought.content {
                MaybeUndefined::Value(content) => self
                    .added(format!("thought {}", thought.message_id), &all_text(&content), false)
                    .map(Update::Thought)
                    .into_iter()
                    .map(Event::Update)
                    .collect(),
                _ => Vec::new(),
            },
            SessionUpdate::ToolCallUpdate(call) => self.call(call),
            SessionUpdate::ToolCallContentChunk(chunk) => {
                let id = chunk.tool_call_id.to_string();
                match content_text(&chunk.content) {
                    Some(text) => self.print(&id, &text, true),
                    None => {
                        self.watch(&id, &chunk.content);
                        Vec::new()
                    }
                }
            }
            SessionUpdate::TerminalUpdate(terminal) => {
                let Some(call) = self.terminal_call(&terminal.terminal_id.to_string()) else {
                    return Vec::new();
                };
                if let MaybeUndefined::Value(output) = terminal.output {
                    return self.print(&call, &decoded(&output.data), false);
                }
                // A command left in the background ends with its terminal,
                // which Claude Code gives no exit code; it ran its course.
                // Other updates, such as the one offering to cancel it, leave it running.
                let MaybeUndefined::Value(exit) = terminal.exit_status else {
                    return Vec::new();
                };
                let Some(task) = self.background.remove(&call) else {
                    return Vec::new();
                };
                let code = serde_json::to_value(&exit).ok().and_then(|exit| exit.get("exitCode")?.as_i64());
                let status = if code.is_none_or(|code| code == 0) { "completed" } else { "failed" };
                vec![news(&call, &task, "background", status)]
            }
            SessionUpdate::TerminalOutputChunk(chunk) => {
                let Some(call) = self.terminal_call(&chunk.terminal_id.to_string()) else {
                    return Vec::new();
                };
                self.print(&call, &decoded(&chunk.data), true)
            }
            SessionUpdate::PlanUpdate(update) => match update.plan {
                v2::PlanUpdateContent::Items(items) => {
                    let steps = items.entries.into_iter().map(|entry| (entry.content, label(&entry.status))).collect();
                    vec![Event::Update(Update::Plan(steps))]
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// What `text` adds to what the message or thought `key` said: a chunk
    /// adds itself, a whole message what it says past what came before.
    fn added(&mut self, key: String, text: &str, chunk: bool) -> Option<String> {
        let said = self.said.entry(key).or_default();
        let added = if chunk {
            text.to_owned()
        } else {
            // A message said again otherwise than it began can't be taken
            // back from what was streamed; only what it adds is kept.
            let added = text.strip_prefix(said.as_str()).unwrap_or_default().to_owned();
            said.clear();
            added
        };
        said.push_str(&added);
        (!added.is_empty()).then_some(added)
    }

    /// A tool call's update: the first announces the call.
    fn call(&mut self, call: ToolCallUpdate) -> Vec<Event> {
        let id = call.tool_call_id.to_string();
        let mut events = Vec::new();
        let title = call.title.take().or_else(|| call.name.take());
        let input = call.raw_input.take();
        if let Some(input) = &input {
            self.inputs.insert(id.clone(), input.clone());
        }
        if self.calls.insert(id.clone()) {
            events.push(Event::Update(Update::ToolCall {
                id: id.clone(),
                title: title.clone().unwrap_or_else(|| "a tool call".to_owned()),
                kind: call.kind.take().as_ref().map(label).unwrap_or_default(),
                input: input.clone(),
            }));
        }
        let mut output = None;
        if let MaybeUndefined::Value(content) = &call.content {
            for item in content {
                self.watch(&id, item);
            }
            let text: Vec<String> = content.iter().filter_map(content_text).collect();
            output = (!text.is_empty()).then(|| text.join("\n"));
        }
        let output = output.or_else(|| {
            call.raw_output.take().map(|output| match output {
                // A message, as Claude's tools answer, rather than data to show as JSON.
                Value::String(text) => text,
                other => other.to_string(),
            })
        });
        let status = call.status.take().map(|status| match status {
            // The daemon closes a call that failed or completed; a
            // cancelled one ended without doing its work.
            ToolCallStatus::Cancelled => "failed".to_owned(),
            other => label(&other),
        });
        let ended = matches!(status.as_deref(), Some("completed" | "failed"));
        let output = if ended { output.or_else(|| self.printed.remove(&id)) } else { output };
        let task = (status.as_deref() == Some("completed")).then(|| self.left_running(&id, output.as_deref())).flatten();
        if status.is_some() || output.is_some() || (events.is_empty() && (title.is_some() || input.is_some())) {
            events.push(Event::Update(Update::ToolUpdate { id, status, output, title, input, task }));
        }
        events
    }

    /// News that a command which completed went on in the background, as
    /// Claude's Bash does with `run_in_background`, under the id its answer
    /// gives it.
    fn left_running(&mut self, call: &str, output: Option<&str>) -> Option<Box<TaskNews>> {
        let background = self.inputs.get(call)?.get("run_in_background")?.as_bool()?;
        if !background {
            return None;
        }
        let task = output.and_then(background_id).unwrap_or_else(|| call.to_owned());
        self.background.insert(call.to_owned(), task.clone());
        Some(Box::new(TaskNews { id: Some(task), kind: Some("background".to_owned()), status: "running".to_owned() }))
    }

    fn watch(&mut self, call: &str, content: &ToolCallContent) {
        if let ToolCallContent::Terminal(terminal) = content {
            self.terminals.insert(terminal.terminal_id.to_string(), call.to_owned());
        }
    }

    /// The call a terminal runs for; Claude names its terminals after them.
    fn terminal_call(&self, terminal: &str) -> Option<String> {
        self.terminals.get(terminal).cloned().or_else(|| self.calls.contains(terminal).then(|| terminal.to_owned()))
    }

    /// The call's output so far, with `text` added or in its place.
    fn print(&mut self, call: &str, text: &str, chunk: bool) -> Vec<Event> {
        let printed = self.printed.entry(call.to_owned()).or_default();
        if !chunk {
            printed.clear();
        }
        printed.push_str(text);
        let output = Some(printed.clone());
        vec![Event::Update(Update::ToolUpdate {
            id: call.to_owned(),
            status: None,
            output,
            title: None,
            input: None,
            task: None,
        })]
    }
}

/// `events` as recorded inside the call `parent` that started the subagent
/// they're of, or as they are where they're the agent's own.
fn inside(parent: Option<String>, events: Vec<Event>) -> Vec<Event> {
    let Some(parent) = parent else { return events };
    events
        .into_iter()
        .filter_map(|event| match event {
            Event::Update(update) => Some(Event::Update(Update::Within { parent: parent.clone(), update: Box::new(update) })),
            _ => None,
        })
        .collect()
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

/// The id Claude Code gives a command it left running, from its answer:
/// "Command running in background with ID: bjc3wn7du. Output is …".
fn background_id(output: &str) -> Option<String> {
    let (_, rest) = output.split_once("with ID: ")?;
    let id: String = rest.chars().take_while(char::is_ascii_alphanumeric).collect();
    (!id.is_empty()).then_some(id)
}

fn stop_reason(reason: v2::StopReason) -> StopReason {
    match reason {
        v2::StopReason::EndTurn => StopReason::EndTurn,
        v2::StopReason::MaxTokens => StopReason::MaxTokens,
        v2::StopReason::MaxTurnRequests => StopReason::MaxTurnRequests,
        v2::StopReason::Refusal => StopReason::Refusal,
        v2::StopReason::Cancelled => StopReason::Cancelled,
        v2::StopReason::Error(_) => StopReason::Error,
        other => StopReason::Other(label(&other)),
    }
}

fn text_of(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(text) => text.text.clone(),
        _ => String::new(),
    }
}

fn all_text(blocks: &[ContentBlock]) -> String {
    blocks.iter().map(text_of).collect()
}

/// What a tool's content says: its text, or which files it edited.
fn content_text(content: &ToolCallContent) -> Option<String> {
    let text = match content {
        ToolCallContent::Content(content) => text_of(&content.content),
        ToolCallContent::Diff(diff) => {
            let paths: Vec<String> = diff
                .changes
                .iter()
                .filter_map(|change| serde_json::to_value(change).ok()?.get("path")?.as_str().map(str::to_owned))
                .collect();
            if paths.is_empty() { String::new() } else { format!("edited {}", paths.join(", ")) }
        }
        _ => String::new(),
    };
    (!text.is_empty()).then_some(text)
}

/// A terminal's output, which v2 sends as base64.
fn decoded(data: &str) -> String {
    BASE64.decode(data).map_or_else(|_| data.to_owned(), |bytes| String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(json: Value) -> SessionUpdate {
        serde_json::from_value(json).expect("a v2 update")
    }

    fn reading() -> (Reading, SessionId) {
        let session = SessionId::new("s1");
        let mut reading = Reading::default();
        reading.begin(&session, &[]);
        (reading, session)
    }

    fn read(reading: &mut Reading, session: &SessionId, json: Value) -> Vec<Event> {
        reading.read(session, update(json))
    }

    /// As Claude's adapter (0.99) tags its commands over v2: each a
    /// `command`, its input `text`.
    #[test]
    fn the_commands_an_agent_offers_are_named_with_what_they_take() {
        let (mut reading, s) = reading();
        let said = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "available_commands_update", "availableCommands": [
                {"type": "command", "name": "engineering:code-review", "description": "Review code changes", "input": {"type": "text", "hint": "<PR URL, diff, or file path>"}},
                {"type": "command", "name": "goal", "description": "Set a goal"}
            ]}),
        );
        let [Event::Commands(commands)] = &said[..] else { panic!("the commands: {said:?}") };
        let said: Vec<(&str, Option<&str>)> =
            commands.iter().map(|command| (command.name.as_str(), command.hint.as_deref())).collect();
        assert_eq!(said, [("engineering:code-review", Some("<PR URL, diff, or file path>")), ("goal", None)]);
    }

    /// Claude's own words for a background command, then its going on by
    /// itself once the command ended, as recorded from its adapter (0.98).
    #[test]
    fn work_the_agent_takes_up_by_itself_starts_and_ends_like_a_prompt_s() {
        let (mut reading, s) = reading();
        assert!(read(&mut reading, &s, serde_json::json!({"sessionUpdate": "state_update", "state": "idle"})).is_empty());
        assert!(matches!(
            read(&mut reading, &s, serde_json::json!({"sessionUpdate": "state_update", "state": "running"}))[..],
            [Event::Working]
        ));
        let said = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": {"type": "text", "text": "Started."}}),
        );
        assert!(matches!(&said[..], [Event::Update(Update::AgentText(text))] if text == "Started."));
        let ended = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn", "usage": {"inputTokens": 6, "outputTokens": 217, "totalTokens": 223}}),
        );
        assert!(matches!(
            &ended[..],
            [Event::TurnUsage { input: 6, output: 217 }, Event::TurnEnded { reason: StopReason::EndTurn }]
        ));

        assert!(matches!(
            read(&mut reading, &s, serde_json::json!({"sessionUpdate": "state_update", "state": "running"}))[..],
            [Event::Working]
        ));
        let said = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": "m2", "content": {"type": "text", "text": "finished BG_DONE"}}),
        );
        assert!(matches!(&said[..], [Event::Update(Update::AgentText(text))] if text == "finished BG_DONE"));
        let ended = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn"}),
        );
        assert!(matches!(&ended[..], [Event::TurnEnded { reason: StopReason::EndTurn }]));
    }

    #[test]
    fn a_command_is_announced_by_its_first_update_and_ends_with_its_terminal_s_output() {
        let (mut reading, s) = reading();
        let announced = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "name": "Bash", "rawInput": {}, "status": "pending", "title": "Terminal", "kind": "execute", "content": [{"type": "terminal", "terminalId": "t1"}]}),
        );
        assert!(
            matches!(&announced[0], Event::Update(Update::ToolCall { id, title, kind, .. }) if id == "t1" && title == "Terminal" && kind == "execute")
        );
        let named = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "rawInput": {"command": "echo FOREGROUND"}, "title": "echo FOREGROUND"}),
        );
        assert!(
            matches!(&named[..], [Event::Update(Update::ToolUpdate { title: Some(title), input: Some(_), status: None, .. })] if title == "echo FOREGROUND")
        );
        let printed = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "terminal_update", "terminalId": "t1", "output": {"data": "Rk9SRUdST1VORA=="}}),
        );
        assert!(
            matches!(&printed[..], [Event::Update(Update::ToolUpdate { output: Some(output), status: None, .. })] if output == "FOREGROUND")
        );
        let done = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed"}),
        );
        assert!(
            matches!(&done[..], [Event::Update(Update::ToolUpdate { status: Some(status), output: Some(output), .. })] if status == "completed" && output == "FOREGROUND")
        );
    }

    #[test]
    fn a_tool_s_own_output_is_its_result() {
        let (mut reading, s) = reading();
        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "r1", "name": "Read", "rawInput": {}, "status": "pending", "title": "Read File", "kind": "read", "content": []}),
        );
        let done = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "r1", "status": "completed", "rawOutput": "1\tBG_DONE", "content": [{"type": "content", "content": {"type": "text", "text": "BG_DONE"}}]}),
        );
        assert!(matches!(&done[..], [Event::Update(Update::ToolUpdate { output: Some(output), .. })] if output == "BG_DONE"));
    }

    #[test]
    fn a_message_sent_whole_again_adds_only_what_is_new() {
        let (mut reading, s) = reading();
        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": {"type": "text", "text": "Hello"}}),
        );
        let whole = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "agent_message", "messageId": "m1", "content": [{"type": "text", "text": "Hello world"}]}),
        );
        assert!(matches!(&whole[..], [Event::Update(Update::AgentText(text))] if text == " world"));
        let again = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "agent_message", "messageId": "m1", "content": [{"type": "text", "text": "Hello world"}]}),
        );
        assert!(again.is_empty());
    }

    fn task_of(event: &Event) -> Option<(&str, Option<&str>, Option<&str>, &str)> {
        match event {
            Event::Update(Update::ToolUpdate { id, task: Some(task), .. }) => {
                Some((id.as_str(), task.id.as_deref(), task.kind.as_deref(), task.status.as_str()))
            }
            _ => None,
        }
    }

    /// Claude's adapter (0.98) running a subagent, as recorded.
    #[test]
    fn a_subagent_s_session_is_recorded_inside_the_call_that_started_it() {
        let (mut reading, s) = reading();
        let child = SessionId::new("a260308e0ffbde619");
        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "subagent_update", "sessionId": "a260308e0ffbde619", "title": "Say ok", "description": "general-purpose", "capabilities": {"cancel": {}}, "state": {"state": "running"}}),
        );
        assert!(read(&mut reading, &child, serde_json::json!({"sessionUpdate": "state_update", "state": "running"})).is_empty());
        let started = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "session_message", "messageId": "toolu_A", "senderSessionId": "s1", "recipientSessionId": "a260308e0ffbde619", "content": [{"type": "text", "text": "Reply with exactly: SUB_OK."}]}),
        );
        let Event::Update(Update::ToolCall { id, title, input: Some(input), .. }) = &started[0] else {
            panic!("the call that started it: {started:?}");
        };
        assert_eq!((id.as_str(), title.as_str()), ("toolu_A", "Agent"));
        assert_eq!((input["title"].as_str(), input["role"].as_str()), (Some("Say ok"), Some("general-purpose")));
        assert_eq!(task_of(&started[1]), Some(("toolu_A", Some("a260308e0ffbde619"), Some("subagent"), "running")));
        assert_eq!(reading.running_children(), std::slice::from_ref(&child));

        let said = read(
            &mut reading,
            &child,
            serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": {"type": "text", "text": "SUB_OK"}, "_meta": {"claudeCode": {"parentToolUseId": "toolu_A"}}}),
        );
        assert!(
            matches!(&said[..], [Event::Update(Update::Within { parent, update })] if parent == "toolu_A" && **update == Update::AgentText("SUB_OK".into()))
        );
        let ended = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "subagent_update", "sessionId": "a260308e0ffbde619", "state": {"state": "idle", "stopReason": "end_turn"}}),
        );
        assert_eq!(task_of(&ended[0]), Some(("toolu_A", Some("a260308e0ffbde619"), Some("subagent"), "completed")));
        assert!(reading.running_children().is_empty());
        let reported = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "session_message", "messageId": "toolu_A:reply", "senderSessionId": "a260308e0ffbde619", "recipientSessionId": "s1", "content": [{"type": "text", "text": "SUB_OK"}]}),
        );
        assert!(
            matches!(&reported[..], [Event::Update(Update::ToolUpdate { id, status: Some(status), output: Some(output), .. })] if id == "toolu_A" && status == "completed" && output == "SUB_OK")
        );
    }

    /// Starts the subagent `child` of the agent `s1` with the call `call`.
    fn start_child(reading: &mut Reading, session: &SessionId, child: &str, call: &str) -> Vec<Event> {
        read(
            reading,
            session,
            serde_json::json!({"sessionUpdate": "subagent_update", "sessionId": child, "title": child, "state": {"state": "running"}}),
        );
        let message = serde_json::json!({"sessionUpdate": "session_message", "messageId": call, "senderSessionId": session.to_string(), "recipientSessionId": child, "content": [{"type": "text", "text": "Go."}]});
        read(reading, session, message)
    }

    fn idle(child: &str) -> serde_json::Value {
        serde_json::json!({"sessionUpdate": "subagent_update", "sessionId": child, "state": {"state": "idle", "stopReason": "end_turn"}})
    }

    /// A subagent that leaves a command running and ends its turn to wait
    /// on it is taken up again once it ends: it runs on until it ends a turn
    /// with nothing of its own left running.
    #[test]
    fn a_subagent_waiting_on_its_own_background_command_still_runs() {
        let (mut reading, s) = reading();
        let child = SessionId::new("c1");
        start_child(&mut reading, &s, "c1", "toolu_A");
        let call = serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "bg", "name": "Bash", "rawInput": {"command": "make", "run_in_background": true}, "status": "pending", "title": "make", "kind": "execute", "content": [{"type": "terminal", "terminalId": "bg"}]});
        read(&mut reading, &child, call);
        let answer = base64::engine::general_purpose::STANDARD.encode("Command running in background with ID: b1.");
        read(
            &mut reading,
            &child,
            serde_json::json!({"sessionUpdate": "terminal_update", "terminalId": "bg", "output": {"data": answer}}),
        );
        read(
            &mut reading,
            &child,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "bg", "status": "completed"}),
        );

        assert!(read(&mut reading, &s, idle("c1")).is_empty(), "its turn ended, not it");
        assert_eq!(reading.running_children(), std::slice::from_ref(&child));
        let ended = read(
            &mut reading,
            &child,
            serde_json::json!({"sessionUpdate": "terminal_update", "terminalId": "bg", "exitStatus": {}}),
        );
        assert!(
            matches!(&ended[..], [Event::Update(Update::Within { parent, update })] if parent == "toolu_A" && matches!(&**update, Update::ToolUpdate { task: Some(_), .. }))
        );
        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "subagent_update", "sessionId": "c1", "state": {"state": "running"}}),
        );
        let done = read(&mut reading, &s, idle("c1"));
        assert_eq!(task_of(&done[0]), Some(("toolu_A", Some("c1"), Some("subagent"), "completed")));
        assert!(read(&mut reading, &s, idle("c1")).is_empty(), "its end is told once");
    }

    /// A subagent's own subagent (Claude Code's `spawnDepth` 2) is announced
    /// on its session: it's a task inside the call that started the first,
    /// with its words inside its own, and the first runs while it does.
    #[test]
    fn a_subagent_s_own_subagent_is_a_task_inside_it() {
        let (mut reading, s) = reading();
        let child = SessionId::new("c1");
        let grandchild = SessionId::new("g1");
        start_child(&mut reading, &s, "c1", "toolu_A");
        let started = start_child(&mut reading, &child, "g1", "toolu_B");
        let inside_a = |event: &Event| match event {
            Event::Update(Update::Within { parent, update }) if parent == "toolu_A" => Some((**update).clone()),
            _ => None,
        };
        assert!(matches!(inside_a(&started[0]), Some(Update::ToolCall { id, .. }) if id == "toolu_B"), "{started:?}");
        let news = inside_a(&started[1])
            .map(|update| task_of(&Event::Update(update)).map(|(call, _, _, status)| (call.to_owned(), status.to_owned())));
        assert_eq!(news, Some(Some(("toolu_B".to_owned(), "running".to_owned()))));

        let said = read(
            &mut reading,
            &grandchild,
            serde_json::json!({"sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": {"type": "text", "text": "Checked."}}),
        );
        assert!(matches!(&said[..], [Event::Update(Update::Within { parent, .. })] if parent == "toolu_B"));
        assert!(read(&mut reading, &s, idle("c1")).is_empty(), "the first waits on its own");

        let ended = read(&mut reading, &child, idle("g1"));
        let news = inside_a(&ended[0])
            .map(|update| task_of(&Event::Update(update)).map(|(call, _, _, status)| (call.to_owned(), status.to_owned())));
        assert_eq!(news, Some(Some(("toolu_B".to_owned(), "completed".to_owned()))));
        let reported = read(
            &mut reading,
            &child,
            serde_json::json!({"sessionUpdate": "session_message", "messageId": "toolu_B:reply", "senderSessionId": "g1", "recipientSessionId": "c1", "content": [{"type": "text", "text": "All verified."}]}),
        );
        assert!(
            matches!(inside_a(&reported[0]), Some(Update::ToolUpdate { id, output: Some(output), .. }) if id == "toolu_B" && output == "All verified.")
        );
        let done = read(&mut reading, &s, idle("c1"));
        assert_eq!(task_of(&done[0]), Some(("toolu_A", Some("c1"), Some("subagent"), "completed")));
    }

    /// Claude's Bash with `run_in_background`, as recorded: it completes at
    /// once, and its terminal ends, with no exit code, once the command does.
    #[test]
    fn a_command_left_in_the_background_runs_until_its_terminal_ends() {
        let (mut reading, s) = reading();
        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "name": "Bash", "rawInput": {}, "status": "pending", "title": "Terminal", "kind": "execute", "content": [{"type": "terminal", "terminalId": "t1"}]}),
        );
        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "rawInput": {"command": "sleep 8 && echo BG_DONE", "run_in_background": true}, "title": "sleep 8 && echo BG_DONE"}),
        );
        let answer = base64::engine::general_purpose::STANDARD
            .encode("Command running in background with ID: bjc3wn7du. Output is being written to: /tmp/x.output");
        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "terminal_update", "terminalId": "t1", "output": {"data": answer}}),
        );
        let left = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed"}),
        );
        assert_eq!(task_of(&left[0]), Some(("t1", Some("bjc3wn7du"), Some("background"), "running")));
        let ended =
            read(&mut reading, &s, serde_json::json!({"sessionUpdate": "terminal_update", "terminalId": "t1", "exitStatus": {}}));
        assert_eq!(task_of(&ended[0]), Some(("t1", Some("bjc3wn7du"), Some("background"), "completed")));
        assert!(
            read(&mut reading, &s, serde_json::json!({"sessionUpdate": "terminal_update", "terminalId": "t1", "exitStatus": {}}))
                .is_empty()
        );

        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "t2", "name": "Bash", "rawInput": {"command": "ls"}, "status": "pending", "title": "ls", "kind": "execute", "content": [{"type": "terminal", "terminalId": "t2"}]}),
        );
        let foreground = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": "t2", "status": "completed"}),
        );
        assert!(foreground.iter().all(|event| task_of(event).is_none()), "a command that ran to its end left nothing running");
    }

    /// Claude's adapter (0.100) offers to cancel a command once it's in the
    /// background, then ends its terminal, as recorded.
    #[test]
    fn a_background_command_offered_for_cancelling_still_ends_with_its_terminal() {
        let (mut reading, s) = reading();
        let call = "toolu_01LxgWy5fqDauekeM8sQe2Gs";
        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "name": "Bash", "rawInput": {}, "status": "pending", "title": "Terminal", "kind": "execute", "content": [{"type": "terminal", "terminalId": call}]}),
        );
        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "rawInput": {"command": "sleep 60 && echo BG_DONE", "run_in_background": true}}),
        );
        let answer = base64::engine::general_purpose::STANDARD
            .encode("Command running in background with ID: bdc2fjmnu. Output is being written to: /tmp/x.output");
        read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "terminal_update", "terminalId": call, "output": {"data": answer}}),
        );
        let left = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "completed"}),
        );
        assert_eq!(task_of(&left[0]), Some((call, Some("bdc2fjmnu"), Some("background"), "running")));
        let offered = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "terminal_update", "terminalId": call, "command": "sleep 60 && echo BG_DONE", "capabilities": {"cancel": {}}}),
        );
        assert!(offered.is_empty());
        let ended = read(
            &mut reading,
            &s,
            serde_json::json!({"sessionUpdate": "terminal_update", "terminalId": call, "exitStatus": {}, "capabilities": null}),
        );
        assert_eq!(task_of(&ended[0]), Some((call, Some("bdc2fjmnu"), Some("background"), "completed")));
    }

    #[test]
    fn another_session_s_updates_are_left_out() {
        let (mut reading, _) = reading();
        let subagent = SessionId::new("child");
        assert!(
            read(&mut reading, &subagent, serde_json::json!({"sessionUpdate": "state_update", "state": "running"})).is_empty()
        );
    }

    #[test]
    fn the_mode_is_a_setting_and_its_change_is_heard() {
        let mode = Setting {
            id: "mode".into(),
            name: "Mode".into(),
            category: Some("mode".into()),
            current: "default".into(),
            choices: vec![
                crate::Choice { id: "default".into(), name: "Manual".into(), description: None },
                crate::Choice { id: "plan".into(), name: "Plan".into(), description: None },
            ],
        };
        let mut reading = Reading::default();
        let modes = reading.begin(&SessionId::new("s1"), std::slice::from_ref(&mode)).expect("modes");
        assert_eq!(modes.current, "default");
        assert_eq!(reading.mode_setting().as_deref(), Some("mode"));
        let changed = reading.settings(vec![Setting { current: "plan".into(), ..mode.clone() }]);
        assert!(matches!(&changed[..], [Event::ModeChanged(mode), Event::Settings(_)] if mode == "plan"));
        let same = reading.settings(vec![Setting { current: "plan".into(), ..mode }]);
        assert!(matches!(&same[..], [Event::Settings(_)]));
    }
}
