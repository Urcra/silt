//! A scripted ACP agent for tests. Each prompt makes it think, ask
//! permission to edit `src/main.rs`, append `FAKE_ACP_LINE` (default
//! `fn acp() {}`) when allowed, and answer. It can load the sessions it
//! made in the same process, replaying what was said, and refuses others.
//! With `FAKE_ACP_CRASH` set it dies on a prompt.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, Implementation, InitializeRequest, InitializeResponse,
    LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
    PermissionOptionKind, PromptRequest, PromptResponse, RequestPermissionOutcome, RequestPermissionRequest,
    SessionId, SessionNotification, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Result, Stdio};

#[derive(Default)]
struct State {
    cwd: HashMap<String, PathBuf>,
    said: HashMap<String, Vec<(bool, String)>>,
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
    Agent
        .builder()
        .name("silt-fake-agent")
        .on_receive_request(
            async move |initialize: InitializeRequest, responder, _connection| {
                responder.respond(
                    InitializeResponse::new(initialize.protocol_version)
                        .agent_capabilities(AgentCapabilities::new().load_session(true))
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
                responder.respond(NewSessionResponse::new(SessionId::new(id)))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: LoadSessionRequest, responder, connection| {
                let said = {
                    let mut state = loading.lock().expect("lock");
                    let Some(said) = state.said.get(&request.session_id.to_string()).cloned() else {
                        return responder.respond_with_error(agent_client_protocol::Error::invalid_params().data("no such session"));
                    };
                    state.cwd.insert(request.session_id.to_string(), request.cwd.clone());
                    said
                };
                for (by_user, line) in said {
                    let update = if by_user {
                        SessionUpdate::UserMessageChunk(text(line))
                    } else {
                        SessionUpdate::AgentMessageChunk(text(line))
                    };
                    notify(&connection, &request.session_id, update)?;
                }
                responder.respond(LoadSessionResponse::new())
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
                    let cwd = state.lock().expect("lock").cwd.get(&session.to_string()).cloned().unwrap_or_default();
                    if std::env::var_os("FAKE_ACP_CRASH").is_some() {
                        std::process::exit(9);
                    }
                    notify(&worker, &session, SessionUpdate::AgentThoughtChunk(text("Thinking about the prompt.")))?;
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
