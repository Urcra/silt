//! A scripted ACP agent for tests. Each prompt makes it think, ask
//! permission to edit `src/main.rs`, append `FAKE_ACP_LINE` (default
//! `fn acp() {}`) when allowed, dropping its first line first when the
//! prompt says `Rewrite`, or write `FAKE_ACP_CONTENT` over it, and answer. It can load the sessions it
//! made in the same process, replaying what was said, and refuses others.
//! It has a `code` and a `plan` mode, where it only answers with a plan,
//! and a model setting, and takes images and files, which its answer
//! names, and offers the commands `count` and `compact` once a session
//! opens, as Copilot does; `FAKE_ACP_NO_MODES` leaves all that out. A prompt saying
//! `Ask me` asks a multiple-choice question instead and answers with the
//! choice; `Ask two` asks two as Claude's adapter does, each followed by an
//! optional box for an answer of one's own, and none required. A prompt saying `Ponder` makes it think for a while between its
//! edit and a read. A prompt saying `Later` makes it say more once the prompt was
//! answered, as an agent going on by itself, without saying it works or
//! stops, as v1 can't. `Show me`, `Guide` and `Artifact` call Silt's own tools (see `show`). `Detached` makes it leave a shell running and read
//! it once the turn is over, as Copilot does. With `FAKE_ACP_CRASH` set it dies on a prompt; with
//! `FAKE_ACP_HANG` it never answers at all, as an agent stuck starting.
//! It speaks ACP v1, as Copilot does, answering a client that asks for v2
//! with v1; with `FAKE_ACP_V2` it speaks v2 instead, as Claude's adapter
//! does (see `v2`). With `FAKE_ACP_SIGN_IN` naming a file, it needs a sign-in
//! until that file exists, refusing a new session over v1, as Copilot does,
//! and a prompt over v2, as Claude's adapter does. Its one sign-in method
//! runs it again with `--fake-login`, which prints a link, asks for the code
//! from it, as Claude's sign-in does, and makes the file when given `good`;
//! given anything else, it says so and waits, as Claude's does.

mod mcp;
mod v2;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, CreateElicitationRequest, EmbeddedResourceResource, Implementation,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse,
    PermissionOption, PermissionOptionKind, PromptCapabilities, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionConfigOption, SessionConfigOptionValue, SessionConfigSelectOption, SessionId, SessionMode,
    SessionModeState, SessionNotification, SessionUpdate, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse,
    SetSessionModeRequest, SetSessionModeResponse, StopReason, TextContent, ToolCall, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind, Usage, UsageUpdate,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Result, Stdio};

#[derive(Default)]
struct State {
    cwd: HashMap<String, PathBuf>,
    said: HashMap<String, Vec<(bool, String)>>,
    mode: HashMap<String, String>,
    model: HashMap<String, String>,
    /// The MCP servers each session was given.
    mcp: HashMap<String, Vec<agent_client_protocol::schema::v1::McpServer>>,
    /// The artifact each session made last, to add a version of.
    artifact: HashMap<String, String>,
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

/// Whether it is signed in: it needs no sign-in unless told to.
pub(crate) fn signed_in() -> bool {
    std::env::var_os("FAKE_ACP_SIGN_IN").is_none_or(|marker| std::path::Path::new(&marker).exists())
}

/// The sign-in it lists, when it needs one, by its id as v1 or v2 names it.
pub(crate) fn sign_in_methods(id: &str) -> serde_json::Value {
    if std::env::var_os("FAKE_ACP_SIGN_IN").is_none() {
        return serde_json::json!([]);
    }
    serde_json::json!([{id: "fake-login", "name": "Fake account", "description": "Sign in to the fake service",
                        "type": "terminal", "args": ["--fake-login"]}])
}

/// Its sign-in, as Claude's prints: a link, then a wait for the code the
/// link's page shows.
fn fake_login() -> std::process::ExitCode {
    use std::io::{BufRead, Write};
    print!(
        "Opening browser to sign in\u{2026}\r\nIf the browser didn't open, visit: https://example.invalid/oauth?state=fake\r\nPaste code here if prompted > "
    );
    let _ = std::io::stdout().flush();
    let mut code = String::new();
    let _ = std::io::stdin().lock().read_line(&mut code);
    let marker = std::env::var_os("FAKE_ACP_SIGN_IN").unwrap_or_default();
    if code.trim() == "good" && std::fs::write(&marker, "signed in").is_ok() {
        println!("\r\nLogin successful.");
        return std::process::ExitCode::SUCCESS;
    }
    // As Claude's sign-in does, it says so and waits rather than exit.
    println!("\r\nLogin failed: That code wasn't accepted.");
    loop {
        std::thread::park();
    }
}

fn main() -> std::process::ExitCode {
    if std::env::args().any(|arg| arg == "--fake-login") {
        return fake_login();
    }
    match agent() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn agent() -> Result<()> {
    // What `Keep` leaves running in the background, until it's ended.
    if std::env::args().nth(1).as_deref() == Some("keep-running") {
        std::future::pending::<()>().await;
    }
    if std::env::var_os("FAKE_ACP_HANG").is_some() {
        std::future::pending::<()>().await;
    }
    if std::env::var_os("FAKE_ACP_V2").is_some() {
        return v2::main().await;
    }
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
                // As Antigravity's `agy acp` does, it may list its terminal
                // sign-in only to a client that says, in v1, it runs one.
                let asked = serde_json::to_value(&initialize)
                    .ok()
                    .and_then(|raw| raw.pointer("/clientCapabilities/auth/terminal")?.as_bool())
                    == Some(true);
                let methods = if std::env::var_os("FAKE_ACP_SIGN_IN_IF_ASKED").is_some() && !asked {
                    serde_json::json!([])
                } else {
                    sign_in_methods("id")
                };
                responder.respond(
                    InitializeResponse::new(ProtocolVersion::V1)
                        .agent_capabilities(
                            AgentCapabilities::new()
                                .load_session(true)
                                .prompt_capabilities(PromptCapabilities::new().image(has_modes()).embedded_context(has_modes())),
                        )
                        .agent_info(Implementation::new("fake-agent", "1.0"))
                        .auth_methods(serde_json::from_value(methods).map_err(agent_client_protocol::Error::into_internal_error)?),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, connection| {
                if !signed_in() {
                    return responder.respond_with_error(agent_client_protocol::Error::auth_required());
                }
                let mut state = created.lock().expect("lock");
                let id = format!("fake-{}", state.cwd.len() + 1);
                state.cwd.insert(id.clone(), request.cwd);
                state.mcp.insert(id.clone(), request.mcp_servers);
                responder.respond(NewSessionResponse::new(SessionId::new(id.clone())).modes(modes("code")).config_options(settings("fast")))?;
                if !has_modes() {
                    return Ok(());
                }
                // As Copilot does, once the session is open.
                let commands = serde_json::json!({"sessionUpdate": "available_commands_update", "availableCommands": [
                    {"name": "count", "description": "Count what you name", "input": {"hint": "<what to count>"}},
                    {"name": "compact", "description": "Summarize the conversation so far"}
                ]});
                notify(&connection, &SessionId::new(id), serde_json::from_value(commands).map_err(agent_client_protocol::Error::into_internal_error)?)
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
                    state.mcp.insert(id.clone(), request.mcp_servers.clone());
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
                    if prompt.contains("Ask two") {
                        let asked: CreateElicitationRequest = serde_json::from_value(serde_json::json!({
                            "mode": "form",
                            "sessionId": session.to_string(),
                            "message": "Please answer the following questions.",
                            "requestedSchema": {
                                "type": "object",
                                "properties": {
                                    "question_0": {"type": "string", "title": "Unit", "description": "What should the progress bar count?", "oneOf": [
                                        {"const": "Bytes", "title": "Bytes"},
                                        {"const": "Chunks", "title": "Chunks"}
                                    ]},
                                    "question_0_custom": {"type": "string", "title": "Other", "description": "Type your own answer, or add a note to the option you chose above (optional)."},
                                    "question_1": {"type": "string", "title": "Place", "description": "Where should it show?", "oneOf": [
                                        {"const": "Header", "title": "Header"},
                                        {"const": "Footer", "title": "Footer"}
                                    ]},
                                    "question_1_custom": {"type": "string", "title": "Other", "description": "Type your own answer, or add a note to the option you chose above (optional)."}
                                }
                            }
                        }))
                        .expect("a form question");
                        let answer = serde_json::to_value(worker.send_request(asked).block_task().await?).unwrap_or_default();
                        let content = &answer["content"];
                        let reply = match (content["question_0"].as_str(), content["question_1"].as_str()) {
                            (Some(unit), Some(place)) if answer["action"] == "accept" => {
                                format!("Counting {unit} in the {place}; notes: {}.", content.as_object().map_or(0, |all| all.len() - 2))
                            }
                            _ => "You didn't choose.".to_owned(),
                        };
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
                        edit_main(&cwd, &prompt).await;
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                    let status = if allowed { ToolCallStatus::Completed } else { ToolCallStatus::Failed };
                    notify(
                        &worker,
                        &session,
                        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call-1", ToolCallUpdateFields::new().status(status))),
                    )?;
                    if prompt.contains("Ponder") {
                        // Thinks between two steps, long enough to be seen before the next step records it.
                        notify(&worker, &session, SessionUpdate::AgentThoughtChunk(text("Checking what the edit left.")))?;
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        let read = ToolCall::new("read-2", "Read src/main.rs").kind(ToolKind::Read).raw_input(serde_json::json!({"path": "src/main.rs"}));
                        notify(&worker, &session, SessionUpdate::ToolCall(read))?;
                        let read = ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(serde_json::json!("fn acp() {}"));
                        notify(&worker, &session, SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("read-2", read)))?;
                    }
                    if prompt.contains("Show me") || prompt.contains("Artifact") || prompt.contains("Guide") || prompt.contains("Read back") || prompt.contains("Doc edit") || prompt.contains("Sheet set") {
                        show(&worker, &session, &state, &cwd, &prompt).await?;
                    }
                    let reply = if allowed { format!("Done: {prompt}") } else { "I was not allowed to edit.".to_owned() };
                    notify(&worker, &session, SessionUpdate::AgentMessageChunk(text(reply.clone())))?;
                    let later = prompt.contains("Later");
                    if prompt.contains("Subagent") {
                        // As Copilot does: the subagent's calls are tagged with its own id, its words aren't.
                        let task = ToolCall::new("task-1", "Count lines").kind(ToolKind::Other).raw_input(
                            serde_json::json!({"name": "count", "agent_type": "explore", "description": "Count lines", "prompt": "Count the lines."}),
                        );
                        notify(&worker, &session, SessionUpdate::ToolCall(task))?;
                        let tagged = serde_json::json!({"github.com/copilot": {"agentId": "sub-1"}});
                        let mut count = ToolCall::new("count-1", "wc -l src/main.rs").kind(ToolKind::Execute);
                        count.meta = tagged.as_object().cloned();
                        notify(&worker, &session, SessionUpdate::ToolCall(count))?;
                        let mut counted = ToolCallUpdate::new(
                            "count-1",
                            ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(serde_json::json!("2 src/main.rs")),
                        );
                        counted.meta = tagged.as_object().cloned();
                        notify(&worker, &session, SessionUpdate::ToolCallUpdate(counted))?;
                        notify(&worker, &session, SessionUpdate::AgentMessageChunk(text("It has 2 lines.")))?;
                        let reported = ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(serde_json::json!("2 lines"));
                        notify(&worker, &session, SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("task-1", reported)))?;
                    }
                    let detached = prompt.contains("Detached");
                    if detached {
                        let shell = ToolCall::new("shell-1", "Run a command in the background")
                            .kind(ToolKind::Execute)
                            .raw_input(serde_json::json!({"command": "sleep 1 && echo DETACHED", "mode": "async", "detach": true}));
                        notify(&worker, &session, SessionUpdate::ToolCall(shell))?;
                        let started = ToolCallUpdateFields::new()
                            .status(ToolCallStatus::Completed)
                            .raw_output(serde_json::json!("<command started in detached background with shellId: 0>"));
                        notify(&worker, &session, SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("shell-1", started)))?;
                    }
                    state
                        .lock()
                        .expect("lock")
                        .said
                        .entry(session.to_string())
                        .or_default()
                        .extend([(true, prompt), (false, reply)]);
                    responder.respond(PromptResponse::new(StopReason::EndTurn).usage(Usage::new(1_840, 1_600, 240)))?;
                    if later {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        notify(&worker, &session, SessionUpdate::AgentMessageChunk(text("Went on by itself.")))?;
                    }
                    if detached {
                        // As Copilot does once the shell ends: it reads it first.
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        let read = ToolCall::new("read-1", "Reading shell output")
                            .kind(ToolKind::Read)
                            .raw_input(serde_json::json!({"shellId": "0", "delay": 0}));
                        notify(&worker, &session, SessionUpdate::ToolCall(read))?;
                        let output = "Output:\nDETACHED\n<detached command with shellId: 0 completed with exit code 0>";
                        let ended = ToolCallUpdateFields::new().status(ToolCallStatus::Completed).raw_output(serde_json::json!(output));
                        notify(&worker, &session, SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("read-1", ended)))?;
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        notify(&worker, &session, SessionUpdate::AgentMessageChunk(text("The detached shell printed DETACHED.")))?;
                    }
                    Ok(())
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}

/// Calls Silt's own tools as an agent the web UI runs would: `Show me`
/// shows an image with `silt_show`, `Guide` reads `silt_artifact_guide`,
/// `Read back` reads the last artifact with `silt_artifact_read`,
/// `Artifact` makes an artifact with `silt_artifact` (`FAKE_ACP_ARTIFACT`
/// its HTML, `FAKE_ACP_DESIGN_SYSTEM` its design system), `Artifact
/// again` adds a version of the last one, and `Artifact edit` adds one by
/// editing the last. Each call is reported as a tool
/// call, with what it answered as its output.
async fn show(
    worker: &ConnectionTo<Client>,
    session: &SessionId,
    state: &Arc<Mutex<State>>,
    cwd: &std::path::Path,
    prompt: &str,
) -> Result<()> {
    let id = session.to_string();
    let (servers, last) = {
        let state = state.lock().expect("lock");
        (state.mcp.get(&id).cloned().unwrap_or_default(), state.artifact.get(&id).cloned())
    };
    let (tool, arguments) = if prompt.contains("Show me") {
        let path = cwd.join("pixel.png");
        tokio::fs::write(&path, mcp::PIXEL_PNG).await.ok();
        ("silt_show", serde_json::json!({"path": "pixel.png", "caption": "One pixel"}))
    } else if prompt.contains("Guide") {
        ("silt_artifact_guide", serde_json::json!({"kind": "html"}))
    } else if prompt.contains("Doc edit")
        && let Some(last) = last.clone()
    {
        // The agent edits a doc people may be typing in (ADR 0077).
        ("silt_artifact_doc", serde_json::json!({"id": last, "edits": [{"old": "- ship", "new": "- ship on Friday"}]}))
    } else if prompt.contains("Sheet set")
        && let Some(last) = last.clone()
    {
        ("silt_artifact_sheet", serde_json::json!({"id": last, "set": {"B3": "=SUM(B1:B2)", "A3": "total"}, "read": "A1:B3"}))
    } else if prompt.contains("Read back")
        && let Some(last) = last.clone()
    {
        ("silt_artifact_read", serde_json::json!({"id": last}))
    } else if prompt.contains("Artifact deck more")
        && let Some(last) = last.clone()
    {
        // A slide added by a patch of the deck (ADR 0076).
        let slide = serde_json::json!({"title": "Next", "html": "<h1>What next</h1>", "notes": "Close here."});
        (
            "silt_artifact",
            serde_json::json!({"id": last, "patch": [{"op": "add", "path": "/slides/-", "value": slide}], "note": "A closing slide"}),
        )
    } else if prompt.contains("Artifact files edit")
        && let Some(last) = last.clone()
    {
        // A version that changes only one of its files (ADR 0074).
        let edit = serde_json::json!({"old": "3", "new": "4", "file": "data.json"});
        ("silt_artifact", serde_json::json!({"id": last, "edits": [edit], "note": "More"}))
    } else if prompt.contains("Artifact edit")
        && let Some(last) = last.clone()
    {
        // A version made by editing the last one in place (ADR 0073).
        let edit = serde_json::json!({"old": "Rescan benchmark", "new": "Rescan benchmark, edited"});
        (
            "silt_artifact",
            serde_json::json!({"id": last, "edits": [edit], "note": "Retitled", "description": "How long a rescan takes"}),
        )
    } else {
        // A page that counts its opens in the viewer's storage and asks for a
        // capability it didn't declare (ADR 0072).
        let counter = "<!doctype html><h1>Rescan benchmark</h1><p id=\"n\"></p><p id=\"cap\">db: asking</p><script>\
var n=Number(localStorage.getItem('n')||0)+1;localStorage.setItem('n',String(n));\
document.getElementById('n').textContent='Opened '+n+' times';\
silt.use('db').then(function(db){document.getElementById('cap').textContent=db?'db: yes':'db: none'});</script>";
        let html = if prompt.contains("Artifact counter") {
            counter.to_owned()
        } else {
            std::env::var("FAKE_ACP_ARTIFACT").unwrap_or_else(|_| "<h1>Rescan benchmark</h1>".into())
        };
        let mut arguments = serde_json::json!({"title": "Rescan benchmark", "kind": "html", "content": html});
        if prompt.contains("Artifact checklist") {
            // A page keeping its tasks in the thread's page data (ADR 0072).
            let page = "<!doctype html><h1>Checklist</h1><button id=\"add\">Add a task</button><p id=\"count\">tasks: asking</p><script>\
silt.use('db').then(function(db){if(!db){document.getElementById('count').textContent='no db';return}\
db.collection('tasks').onSnapshot(function(docs){document.getElementById('count').textContent='tasks: '+docs.length});\
document.getElementById('add').onclick=function(){db.collection('tasks').add({done:false})}});</script>";
            arguments = serde_json::json!({"title": "Checklist", "kind": "html", "content": page, "capabilities": {"db": {}}});
        }
        if prompt.contains("Artifact deck") {
            let deck = serde_json::json!({"aspect": "16:9", "style": "h1{font:64px sans-serif}", "slides": [
                {"title": "Rescans", "html": "<h1>Rescans got 3x faster</h1>", "notes": "Open with the headline."},
                {"title": "Why", "html": "<h1>One walk</h1>"}
            ]});
            arguments = serde_json::json!({"title": "Rescan talk", "kind": "slides", "content": deck.to_string()});
        }
        if prompt.contains("Artifact doc") {
            arguments = serde_json::json!({"title": "Launch plan", "kind": "doc", "content": "# Plan\n- ship\n"});
        }
        if prompt.contains("Artifact sheet") {
            let rows = serde_json::json!({"rows": [["apples", 3], ["pears", 5]]});
            arguments = serde_json::json!({"title": "Tracker", "kind": "sheet", "content": rows.to_string()});
        }
        if prompt.contains("Artifact tools") {
            let mcp = serde_json::json!({"servers": [{"server": "silt", "tools": ["silt_history_search"]}]});
            arguments = serde_json::json!({"title": "History finder", "kind": "html", "content": "<!doctype html><h1>Finder</h1>", "capabilities": {"mcp": mcp}});
        }
        if prompt.contains("Artifact board") {
            let items = serde_json::json!({"items": [
                {"id": "goal", "type": "block", "x": 0, "y": 0, "w": 160, "h": 100, "text": "Ship rooms", "color": "yellow"},
                {"id": "risk", "type": "block", "x": 240, "y": 0, "w": 160, "h": 100, "text": "Latency", "color": "pink"},
                {"id": "a1", "type": "arrow", "from": "goal", "to": "risk", "label": "watch"}
            ]});
            arguments = serde_json::json!({"title": "Planning board", "kind": "whiteboard", "content": items.to_string()});
        }
        // Arguments written out in a file, for an artifact whose content a test authored.
        if let Some(path) = prompt.split("Artifact from ").nth(1).map(str::trim)
            && let Ok(text) = std::fs::read_to_string(path)
            && let Ok(given) = serde_json::from_str::<serde_json::Value>(&text)
        {
            arguments = given;
        }
        if prompt.contains("Artifact numbers") {
            let page = serde_json::json!({"title": "Rescan benchmark", "sources": [
                {"id": "bench", "name": "silt-bench rescan", "kind": "command", "detail": "silt-bench run --only rescan", "at": "2026-11-15T09:00:00Z"}
            ], "blocks": [
                {"type": "stat", "label": "p50", "value": 412, "unit": "ms", "change": -0.62, "source": "bench"},
                {"type": "chart", "chart": "line", "title": "p50 by commit", "source": "bench", "series": [{"name": "p50", "points": [["a1", 1080], ["b2", 640], ["c3", 412]]}]}
            ]});
            arguments = serde_json::json!({"title": "Rescan numbers", "kind": "data", "content": page.to_string()});
        }
        if prompt.contains("Artifact boards") {
            let board = |id: &str, width: u32, height: u32| serde_json::json!({"id": id, "name": id, "width": width, "height": height, "html": format!("<p>{id}</p>")});
            let canvas = serde_json::json!({"boards": [board("Desktop", 1280, 800), board("Tablet", 834, 1194), board("Phone", 390, 844)]});
            arguments = serde_json::json!({"title": "Settings screen", "kind": "canvas", "content": canvas.to_string()});
        }
        if prompt.contains("Artifact bad deck") {
            arguments = serde_json::json!({"title": "Empty", "kind": "slides", "content": "{\"slides\": []}"});
        }
        if prompt.contains("Artifact files") {
            // A page built from files beside it, that takes uploads and reads the thread's files (ADR 0074, 0078).
            arguments = serde_json::json!({
                "title": "Report",
                "kind": "html",
                "content": "<!doctype html><link rel=\"stylesheet\" href=\"css/site.css\"><h1>Report</h1><p id=\"n\">n: asking</p><script>fetch('data.json').then(function(r){return r.json()}).then(function(d){document.getElementById('n').textContent='n: '+d.n})</script>",
                "files": {"data.json": {"content": "{\"n\": 3}"}, "css/site.css": {"content": "h1 { color: teal }"}},
                "capabilities": {"assets": {}, "files": {"paths": ["src/**"]}}
            });
        }
        if prompt.contains("Artifact saver") {
            // A page that saves itself and asks an agent (ADR 0072).
            arguments = serde_json::json!({
                "title": "Saver",
                "kind": "html",
                "content": "<!doctype html><h1>Saver</h1>",
                "capabilities": {"artifact": {}, "sample": {"images": true}, "user": {}}
            });
        }
        if prompt.contains("Artifact poll") {
            // Everyone reads the votes, and each writes their own (ADR 0072).
            let rules = serde_json::json!([
                {"path": "votes", "read": "read", "write": "owner"},
                {"path": "votes/{self}", "write": "comment"}
            ]);
            arguments = serde_json::json!({
                "title": "Lunch poll",
                "kind": "html",
                "content": "<!doctype html><h1>Lunch poll</h1>",
                "capabilities": {"db": {"rules": rules}}
            });
        }
        if let Ok(system) = std::env::var("FAKE_ACP_DESIGN_SYSTEM") {
            arguments["design_system"] = serde_json::json!(system);
        }
        if prompt.contains("again")
            && let Some(last) = last
        {
            arguments["id"] = serde_json::json!(last);
        }
        ("silt_artifact", arguments)
    };
    let call = ToolCall::new("show-1", tool).kind(ToolKind::Other).raw_input(arguments.clone());
    notify(worker, session, SessionUpdate::ToolCall(call))?;
    let cwd = cwd.to_owned();
    let answer = tokio::task::spawn_blocking(move || mcp::call(&servers, &cwd, tool, arguments))
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
    if let Ok(text) = &answer
        && let Some(made) = text.split_whitespace().nth(1).filter(|word| word.len() == 12)
    {
        state.lock().expect("lock").artifact.insert(id, made.to_owned());
    }
    let (status, text) = match answer {
        Ok(text) => (ToolCallStatus::Completed, text),
        Err(text) => (ToolCallStatus::Failed, text),
    };
    let fields = ToolCallUpdateFields::new().status(status).raw_output(serde_json::json!(text));
    notify(worker, session, SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("show-1", fields)))
}

/// The edit a prompt makes: `FAKE_ACP_CONTENT` replaces `src/main.rs`, else
/// `FAKE_ACP_LINE` is appended to it, after its first line is dropped when
/// the prompt says `Rewrite`.
pub(crate) async fn edit_main(cwd: &std::path::Path, prompt: &str) {
    let path = cwd.join("src/main.rs");
    let content = match std::env::var("FAKE_ACP_CONTENT") {
        Ok(content) => content,
        Err(_) => {
            let line = std::env::var("FAKE_ACP_LINE").unwrap_or_else(|_| "fn acp() {}".into());
            let before = tokio::fs::read_to_string(&path).await.unwrap_or_default();
            let kept = if prompt.contains("Rewrite") { before.split_once('\n').map_or("", |(_, rest)| rest) } else { &before };
            format!("{kept}{line}\n")
        }
    };
    // A chat's folder starts empty (ADR 0059).
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    tokio::fs::write(&path, content).await.ok();
}
