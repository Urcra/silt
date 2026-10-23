//! A scripted ACP agent for tests. Each prompt makes it think, ask
//! permission to edit `src/main.rs`, append `FAKE_ACP_LINE` (default
//! `fn acp() {}`) when allowed, and answer. It can load the sessions it
//! made in the same process, replaying what was said, and refuses others.
//! It has a `code` and a `plan` mode, where it only answers with a plan,
//! and a model setting, and takes images and files, which its answer
//! names; `FAKE_ACP_NO_MODES` leaves all that out. A prompt saying
//! `Ask me` asks a multiple-choice question instead and answers with the
//! choice. With `FAKE_ACP_CRASH` set it dies on a prompt.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, CreateElicitationRequest, EmbeddedResourceResource, Implementation,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse,
    PermissionOption, PermissionOptionKind, PromptCapabilities, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionConfigOption, SessionConfigOptionValue, SessionConfigSelectOption, SessionId, SessionMode,
    SessionModeState, SessionNotification, SessionUpdate, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse,
    SetSessionModeRequest, SetSessionModeResponse, StopReason, TextContent, ToolCall, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind, UsageUpdate,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Result, Stdio};

#[derive(Default)]
struct State {
    cwd: HashMap<String, PathBuf>,
    said: HashMap<String, Vec<(bool, String)>>,
    mode: HashMap<String, String>,
    model: HashMap<String, String>,
}

fn has_modes() -> bool {
    std::env::var_os("FAKE_ACP_NO_MODES").is_none()
}

fn modes(current: &str) -> Option<SessionModeState> {
    has_modes().then(|| {
        SessionModeState::new(
            current.to_owned(),
            vec![
                SessionMode::new("code", "Code").description("Edits files"),
                SessionMode::new("plan", "Plan").description("Only writes a plan"),
            ],
        )
    })
}

fn settings(model: &str) -> Option<Vec<SessionConfigOption>> {
    has_modes().then(|| {
        vec![SessionConfigOption::select(
            "model",
            "Model",
            model.to_owned(),
            vec![SessionConfigSelectOption::new("fast", "Fast"), SessionConfigSelectOption::new("smart", "Smart")],
        )]
    })
}

fn text(content: impl Into<String>) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(content.into())))
}

fn notify(connection: &ConnectionTo<Client>, session: &SessionId, update: SessionUpdate) -> Result<()> {
    connection.send_notification(SessionNotification::new(session.clone(), update))
}

#[tokio::main]
async fn main() -> Result<()> {
    let state = Arc::new(Mutex::new(State::default()));
    let created = state.clone();
    let loading = state.clone();
    let prompted = state.clone();
    let moded = state.clone();
    let configured = state.clone();
    Agent
        .builder()
        .name("silt-fake-agent")
        .on_receive_request(
            async move |initialize: InitializeRequest, responder, _connection| {
                responder.respond(
                    InitializeResponse::new(initialize.protocol_version)
                        .agent_capabilities(
                            AgentCapabilities::new()
                                .load_session(true)
                                .prompt_capabilities(PromptCapabilities::new().image(has_modes()).embedded_context(has_modes())),
                        )
                        .agent_info(Implementation::new("fake-agent", "1.0")),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _connection| {
                let mut state = created.lock().expect("lock");
                let id = format!("fake-{}", state.cwd.len() + 1);
                state.cwd.insert(id.clone(), request.cwd);
                responder.respond(NewSessionResponse::new(SessionId::new(id)).modes(modes("code")).config_options(settings("fast")))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionModeRequest, responder, _connection| {
                let mode = request.mode_id.to_string();
                if !has_modes() || !["code", "plan"].contains(&mode.as_str()) {
                    return responder.respond_with_error(agent_client_protocol::Error::invalid_params().data("no such mode"));
                }
                moded.lock().expect("lock").mode.insert(request.session_id.to_string(), mode);
                responder.respond(SetSessionModeResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionConfigOptionRequest, responder, _connection| {
                let value = match &request.value {
                    SessionConfigOptionValue::ValueId { value } => value.to_string(),
                    _ => String::new(),
                };
                if !has_modes() || request.config_id.to_string() != "model" || !["fast", "smart"].contains(&value.as_str()) {
                    return responder.respond_with_error(agent_client_protocol::Error::invalid_params().data("no such setting"));
                }
                configured.lock().expect("lock").model.insert(request.session_id.to_string(), value.clone());
                responder.respond(SetSessionConfigOptionResponse::new(settings(&value).unwrap_or_default()))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: LoadSessionRequest, responder, connection| {
                let (said, mode, model) = {
                    let mut state = loading.lock().expect("lock");
                    let id = request.session_id.to_string();
                    let Some(said) = state.said.get(&id).cloned() else {
                        return responder.respond_with_error(agent_client_protocol::Error::invalid_params().data("no such session"));
                    };
                    state.cwd.insert(id.clone(), request.cwd.clone());
                    let mode = state.mode.get(&id).cloned().unwrap_or_else(|| "code".into());
                    let model = state.model.get(&id).cloned().unwrap_or_else(|| "fast".into());
                    (said, mode, model)
                };
                for (by_user, line) in said {
                    let update = if by_user {
                        SessionUpdate::UserMessageChunk(text(line))
                    } else {
                        SessionUpdate::AgentMessageChunk(text(line))
                    };
                    notify(&connection, &request.session_id, update)?;
                }
                responder.respond(LoadSessionResponse::new().modes(modes(&mode)).config_options(settings(&model)))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection| {
                let state = prompted.clone();
                let worker = connection.clone();
                connection.spawn(async move {
                    let session = request.session_id.clone();
                    let prompt: String = request
                        .prompt
                        .iter()
                        .filter_map(|block| match block {
                            ContentBlock::Text(text) => Some(text.text.clone()),
                            _ => None,
                        })
                        .collect();
                    let attached: Vec<String> = request
                        .prompt
                        .iter()
                        .filter_map(|block| match block {
                            ContentBlock::Image(image) => Some(format!("an image ({})", image.mime_type)),
                            ContentBlock::Resource(resource) => Some(match &resource.resource {
                                EmbeddedResourceResource::TextResourceContents(text) => format!("{} saying {:?}", text.uri, text.text),
                                EmbeddedResourceResource::BlobResourceContents(blob) => blob.uri.clone(),
                                _ => "a resource".to_owned(),
                            }),
                            _ => None,
                        })
                        .collect();
                    let prompt = if attached.is_empty() { prompt } else { format!("{prompt} (with {})", attached.join(", ")) };
                    let (cwd, planning) = {
                        let state = state.lock().expect("lock");
                        let id = session.to_string();
                        (state.cwd.get(&id).cloned().unwrap_or_default(), state.mode.get(&id).is_some_and(|mode| mode == "plan"))
                    };
                    if std::env::var_os("FAKE_ACP_CRASH").is_some() {
                        std::process::exit(9);
                    }
                    notify(&worker, &session, SessionUpdate::AgentThoughtChunk(text("Thinking about the prompt.")))?;
                    notify(&worker, &session, SessionUpdate::UsageUpdate(UsageUpdate::new(53_000, 200_000)))?;
                    if planning {
                        let reply = format!("Plan: {prompt}");
                        notify(&worker, &session, SessionUpdate::AgentMessageChunk(text(reply.clone())))?;
                        state.lock().expect("lock").said.entry(session.to_string()).or_default().extend([(true, prompt), (false, reply)]);
                        return responder.respond(PromptResponse::new(StopReason::EndTurn));
                    }
                    if prompt.contains("Ask me") {
                        let asked: CreateElicitationRequest = serde_json::from_value(serde_json::json!({
                            "mode": "form",
                            "sessionId": session.to_string(),
                            "message": "What should the progress bar count?",
                            "requestedSchema": {
                                "type": "object",
                                "properties": {"count": {
                                    "type": "string",
                                    "title": "Progress",
                                    "oneOf": [
                                        {"const": "acked", "title": "Bytes the server acknowledged", "description": "Accurate, but moves in steps of one chunk"},
                                        {"const": "sent", "title": "Bytes sent on the socket", "description": "Smooth, but can run ahead of the server"},
                                        {"const": "chunks", "title": "Chunks completed", "description": "Simplest, coarse for small files"}
                                    ]
                                }},
                                "required": ["count"]
                            }
                        }))
                        .expect("a form question");
                        let answer = serde_json::to_value(worker.send_request(asked).block_task().await?).unwrap_or_default();
                        let reply = match answer["content"]["count"].as_str() {
                            Some(chosen) if answer["action"] == "accept" => format!("Counting {chosen}."),
                            _ => "You didn't choose.".to_owned(),
                        };
                        notify(&worker, &session, SessionUpdate::AgentMessageChunk(text(reply.clone())))?;
                        state.lock().expect("lock").said.entry(session.to_string()).or_default().extend([(true, prompt), (false, reply)]);
                        return responder.respond(PromptResponse::new(StopReason::EndTurn));
                    }
                    let call = ToolCall::new("call-1", "Edit src/main.rs")
                        .kind(ToolKind::Edit)
                        .raw_input(serde_json::json!({"path": "src/main.rs"}));
                    notify(&worker, &session, SessionUpdate::ToolCall(call))?;
                    let options = vec![
                        PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
                        PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
                    ];
                    let asked = RequestPermissionRequest::new(
                        session.clone(),
                        ToolCallUpdate::new("call-1", ToolCallUpdateFields::new().title("Edit src/main.rs")),
                        options,
                    );
                    let answer = worker.send_request(asked).block_task().await?;
                    let allowed = matches!(&answer.outcome, RequestPermissionOutcome::Selected(selected) if selected.option_id.to_string() == "allow");
                    if allowed {
                        let line = std::env::var("FAKE_ACP_LINE").unwrap_or_else(|_| "fn acp() {}".into());
                        let path = cwd.join("src/main.rs");
                        let mut content = tokio::fs::read_to_string(&path).await.unwrap_or_default();
                        content.push_str(&line);
                        content.push('\n');
                        tokio::fs::write(&path, content).await.ok();
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                    let status = if allowed { ToolCallStatus::Completed } else { ToolCallStatus::Failed };
                    notify(
                        &worker,
                        &session,
                        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call-1", ToolCallUpdateFields::new().status(status))),
                    )?;
                    let reply = if allowed { format!("Done: {prompt}") } else { "I was not allowed to edit.".to_owned() };
                    notify(&worker, &session, SessionUpdate::AgentMessageChunk(text(reply.clone())))?;
                    state
                        .lock()
                        .expect("lock")
                        .said
                        .entry(session.to_string())
                        .or_default()
                        .extend([(true, prompt), (false, reply)]);
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}
