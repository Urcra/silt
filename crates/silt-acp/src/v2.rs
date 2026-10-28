//! A session with an agent that speaks ACP v2, as Claude's adapter does: a
//! prompt is answered once the agent takes it in, and the agent says when
//! it works and when it stops, for work it takes up by itself between
//! prompts too, such as Claude's once a background task ends.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v2::{
    self, CancelSessionNotification, ClientCapabilities, ContentBlock, CreateElicitationRequest, CreateElicitationResponse,
    ElicitationCapabilities, ElicitationFormCapabilities, Implementation, InitializeRequest, McpServer, McpServerStdio,
    NewSessionRequest, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    RequestPermissionSubject, ResumeSessionRequest, SelectedPermissionOutcome, SessionConfigValueId, SessionId, SessionUpdate,
    SetSessionConfigOptionRequest, StateUpdate, ToolCallContent, ToolCallStatus, ToolCallUpdate, UpdateSessionNotification,
};
use agent_client_protocol::schema::{MaybeUndefined, ProtocolVersion};
use agent_client_protocol::{Agent, Client, ConnectTo, Responder, V2ConnectionTo};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;

use crate::host::{Command, Event, HostError, Permission, Question, Shared, StopReason, Update, answered, label};
use crate::modes::{Setting, modes_in, settings_v2};
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
            .map(|(command, args)| McpServer::Stdio(McpServerStdio::new("silt", command.clone()).args(args.clone())))
            .collect();

        let updates = events.clone();
        let read = reading.clone();
        let asked = events.clone();
        let parked = waiting.clone();
        Client
            .v2()
            .name(crate::host::NAME)
            .on_receive_notification(
                async move |notification: UpdateSessionNotification, _connection: V2ConnectionTo<Agent>| {
                    for event in read.lock().expect("lock").read(&notification.session_id, notification.update) {
                        let _ = updates.send(event);
                    }
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_request(
                async move |request: RequestPermissionRequest, responder, _connection: V2ConnectionTo<Agent>| {
                    let id = next_id.fetch_add(1, Ordering::SeqCst);
                    let permission = permission(id, request);
                    parked.lock().expect("lock").insert(id, responder);
                    let _ = asked.send(Event::Permission(permission));
                    Ok(())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: CreateElicitationRequest, responder, _connection: V2ConnectionTo<Agent>| {
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
            .connect_with(agent, async move |connection: V2ConnectionTo<Agent>| {
                let capabilities = ClientCapabilities::new()
                    .elicitation(ElicitationCapabilities::new().form(ElicitationFormCapabilities::new()));
                let request = InitializeRequest::new(
                    ProtocolVersion::V2,
                    Implementation::new(crate::host::NAME, env!("CARGO_PKG_VERSION")),
                )
                .capabilities(capabilities);
                let init = connection.send_request(request).block_task().await?;
                let Some(mut commands) = commands.lock().expect("lock").take() else {
                    return Ok(());
                };
                let takes = init.capabilities.session.as_ref().and_then(|session| session.prompt.as_ref());
                let accepts = Accepts {
                    image: takes.is_some_and(|takes| takes.image.is_some()),
                    embedded: takes.is_some_and(|takes| takes.embedded_context.is_some()),
                };
                let resumed = match start.load.clone() {
                    Some(id) => connection
                        .send_request(
                            ResumeSessionRequest::new(SessionId::new(id.clone()), start.cwd.clone())
                                .mcp_servers(mcp_servers.clone()),
                        )
                        .block_task()
                        .await
                        // A session made on another machine can't be resumed
                        // here; the daemon primes a new one instead.
                        .ok()
                        .map(|response| (SessionId::new(id), response.config_options)),
                    None => None,
                };
                let loaded = resumed.is_some();
                let (session, options) = match resumed {
                    Some(resumed) => resumed,
                    None => {
                        let created = connection
                            .send_request(NewSessionRequest::new(start.cwd.clone()).mcp_servers(mcp_servers.clone()))
                            .block_task()
                            .await?;
                        (created.session_id, created.config_options)
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
                });
                while let Some(command) = commands.recv().await {
                    match command {
                        Command::Prompt { text, attachments } => {
                            let prompt = blocks_v2(&text, &attachments, accepts)
                                .map_err(agent_client_protocol::Error::into_internal_error)?;
                            let failed = events.clone();
                            // Sent here rather than in the task, so a mode set
                            // just before reaches the agent first.
                            let sent = connection.send_request(PromptRequest::new(session.clone(), prompt));
                            connection.spawn(async move {
                                // Taken in; the agent says when it is done.
                                if let Err(error) = sent.block_task().await {
                                    let _ = failed.send(Event::Failed(Arc::new(HostError::Protocol(error))));
                                    let _ = failed.send(Event::TurnEnded { reason: StopReason::Error });
                                }
                                Ok(())
                            })?;
                        }
                        Command::Cancel => {
                            connection.send_notification(CancelSessionNotification::new(session.clone()))?;
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
                            let Some(setting) = reading.lock().expect("lock").mode_setting() else {
                                let _ = events.send(Event::Refused("the agent has no modes".to_owned()));
                                continue;
                            };
                            let request = SetSessionConfigOptionRequest::new(
                                session.clone(),
                                setting,
                                SessionConfigValueId::new(mode.clone()),
                            );
                            let (told, read) = (events.clone(), reading.clone());
                            let sent = connection.send_request(request);
                            connection.spawn(async move {
                                match sent.block_task().await {
                                    Ok(response) => {
                                        for event in read.lock().expect("lock").settings(settings_v2(response.config_options)) {
                                            let _ = told.send(event);
                                        }
                                    }
                                    Err(error) => {
                                        let _ = told.send(Event::Refused(format!("the agent kept its mode: {error}")));
                                    }
                                }
                                Ok(())
                            })?;
                        }
                        Command::SetSetting { id, value } => {
                            let request =
                                SetSessionConfigOptionRequest::new(session.clone(), id, SessionConfigValueId::new(value));
                            let (told, read) = (events.clone(), reading.clone());
                            let sent = connection.send_request(request);
                            connection.spawn(async move {
                                match sent.block_task().await {
                                    Ok(response) => {
                                        for event in read.lock().expect("lock").settings(settings_v2(response.config_options)) {
                                            let _ = told.send(event);
                                        }
                                    }
                                    Err(error) => {
                                        let _ = told.send(Event::Refused(format!("the agent kept its setting: {error}")));
                                    }
                                }
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

    /// What an update of `session` says, as Silt's events. Updates of
    /// other sessions, such as a subagent's, are left out.
    pub(crate) fn read(&mut self, session: &SessionId, update: SessionUpdate) -> Vec<Event> {
        if self.session.as_ref() != Some(session) {
            return Vec::new();
        }
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
                let MaybeUndefined::Value(output) = terminal.output else {
                    return Vec::new();
                };
                let Some(call) = self.terminal_call(&terminal.terminal_id.to_string()) else {
                    return Vec::new();
                };
                self.print(&call, &decoded(&output.data), false)
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
            SessionUpdate::ConfigOptionUpdate(update) => self.settings(settings_v2(update.config_options)),
            SessionUpdate::UsageUpdate(usage) => vec![Event::Context { used: usage.used, size: usage.size }],
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
        if status.is_some() || output.is_some() || (events.is_empty() && (title.is_some() || input.is_some())) {
            events.push(Event::Update(Update::ToolUpdate { id, status, output, title, input }));
        }
        events
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
        vec![Event::Update(Update::ToolUpdate { id: call.to_owned(), status: None, output, title: None, input: None })]
    }
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
