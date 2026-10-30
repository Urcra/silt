//! The scripted agent speaking ACP v2, with `FAKE_ACP_V2` set, in the shapes
//! Claude's adapter sends: each prompt makes it say it works, think, run a
//! command whose output comes over its terminal, ask permission to edit
//! `src/main.rs`, append `FAKE_ACP_LINE` when allowed, answer and say it
//! stopped. A prompt saying `Later` makes it go on by itself once it
//! stopped, as Claude does when a background task ends, and stop again.
//! `Subagent` makes it run a subagent in a session of its own, as Claude's
//! Agent tool does; `Background` makes it leave a command running, which
//! ends after the turn and sets the agent going again, unless cancelled.
//! `Present` makes it put a plan to the person, as Claude's ExitPlanMode
//! does, by asking leave to quit its plan mode. Its session offers the commands
//! `count` and `compact` as it opens, and drops `compact` on a prompt.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v2;
use agent_client_protocol::{Agent, Client, Responder, Result, Stdio, V2ConnectionTo};
use base64::Engine;
use serde_json::{Value, json};

#[derive(Default)]
struct State {
    cwd: HashMap<String, PathBuf>,
    mode: HashMap<String, String>,
    model: HashMap<String, String>,
}

/// The commands it offers, tagged as Claude's adapter tags them over v2;
/// `compact` only until the first prompt, as the adapter drops commands that
/// work only in a terminal once its first prompt shows which they are.
fn commands(prompted: bool) -> Value {
    let count = json!({"type": "command", "name": "count", "description": "Count what you name", "input": {"type": "text", "hint": "<what to count>"}});
    let compact = json!({"type": "command", "name": "compact", "description": "Summarize the conversation so far"});
    if prompted { json!([count]) } else { json!([count, compact]) }
}

fn options(mode: &str, model: &str) -> Value {
    json!([
        {"configId": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": mode,
         "options": [{"value": "code", "name": "Code"}, {"value": "plan", "name": "Plan"}]},
        {"configId": "model", "name": "Model", "category": "model", "type": "select", "currentValue": model,
         "options": [{"value": "fast", "name": "Fast"}, {"value": "smart", "name": "Smart"}]}
    ])
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(agent_client_protocol::Error::into_internal_error)
}

fn say(connection: &V2ConnectionTo<Client>, session: &v2::SessionId, update: Value) -> Result<()> {
    connection.send_notification(v2::UpdateSessionNotification::new(session.clone(), parse::<v2::SessionUpdate>(update)?))
}

pub async fn main() -> Result<()> {
    let state = Arc::new(Mutex::new(State::default()));
    let (created, resumed, configured, prompted) = (state.clone(), state.clone(), state.clone(), state);
    let stopping = Arc::new(AtomicBool::new(false));
    let cancelling = stopping.clone();
    Agent
        .v2()
        .name("silt-fake-agent")
        .on_receive_notification(
            async move |_cancel: v2::CancelSessionNotification, _connection: V2ConnectionTo<Client>| {
                cancelling.store(true, Ordering::SeqCst);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: v2::InitializeRequest, responder: Responder<v2::InitializeResponse>, _connection: V2ConnectionTo<Client>| {
                responder.respond(parse(json!({
                    "protocolVersion": request.protocol_version,
                    "info": {"name": "fake-agent", "version": "2.0"},
                    "capabilities": {"session": {"prompt": {"image": {}, "embeddedContext": {}}}}
                }))?)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: v2::NewSessionRequest, responder: Responder<v2::NewSessionResponse>, _connection: V2ConnectionTo<Client>| {
                let mut state = created.lock().expect("lock");
                let id = format!("fake-v2-{}", state.cwd.len() + 1);
                state.cwd.insert(id.clone(), request.cwd.into_inner());
                responder.respond(parse(json!({"sessionId": id, "configOptions": options("code", "fast"), "availableCommands": commands(false)}))?)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: v2::ResumeSessionRequest, responder: Responder<v2::ResumeSessionResponse>, _connection: V2ConnectionTo<Client>| {
                let mut state = resumed.lock().expect("lock");
                let id = request.session_id.to_string();
                if !state.cwd.contains_key(&id) {
                    return responder.respond_with_error(agent_client_protocol::Error::invalid_params().data("no such session"));
                }
                state.cwd.insert(id.clone(), request.cwd.into_inner());
                let mode = state.mode.get(&id).cloned().unwrap_or_else(|| "code".into());
                let model = state.model.get(&id).cloned().unwrap_or_else(|| "fast".into());
                responder.respond(parse(json!({"configOptions": options(&mode, &model), "availableCommands": commands(false)}))?)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: v2::SetSessionConfigOptionRequest, responder: Responder<v2::SetSessionConfigOptionResponse>, _connection: V2ConnectionTo<Client>| {
                let value = serde_json::to_value(&request.value)?.get("value").and_then(Value::as_str).unwrap_or_default().to_owned();
                let (id, config) = (request.session_id.to_string(), request.config_id.to_string());
                let mut state = configured.lock().expect("lock");
                match (config.as_str(), value.as_str()) {
                    ("mode", "code" | "plan") => state.mode.insert(id.clone(), value),
                    ("model", "fast" | "smart") => state.model.insert(id.clone(), value),
                    _ => return responder.respond_with_error(agent_client_protocol::Error::invalid_params().data("no such setting")),
                };
                let mode = state.mode.get(&id).cloned().unwrap_or_else(|| "code".into());
                let model = state.model.get(&id).cloned().unwrap_or_else(|| "fast".into());
                responder.respond(parse(json!({"configOptions": options(&mode, &model)}))?)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: v2::PromptRequest, responder: Responder<v2::PromptResponse>, connection: V2ConnectionTo<Client>| {
                responder.respond(parse(json!({"messageId": "user-1"}))?)?;
                let state = prompted.clone();
                let cancelled = stopping.clone();
                let worker = connection.clone();
                connection.spawn(async move {
                    let session = request.session_id.clone();
                    let prompt: String = request
                        .prompt
                        .iter()
                        .filter_map(|block| match block {
                            v2::ContentBlock::Text(text) => Some(text.text.clone()),
                            _ => None,
                        })
                        .collect();
                    let cwd = state.lock().expect("lock").cwd.get(&session.to_string()).cloned().unwrap_or_default();
                    say(&worker, &session, json!({"sessionUpdate": "available_commands_update", "availableCommands": commands(true)}))?;
                    say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "running"}))?;
                    say(&worker, &session, json!({"sessionUpdate": "agent_thought_chunk", "messageId": "thought-1", "content": {"type": "text", "text": "Thinking about the prompt."}}))?;
                    say(&worker, &session, json!({"sessionUpdate": "usage_update", "used": 53_000, "size": 200_000}))?;
                    say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "run-1", "name": "Bash", "rawInput": {}, "status": "pending", "title": "Terminal", "kind": "execute", "content": [{"type": "terminal", "terminalId": "run-1"}]}))?;
                    say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "run-1", "rawInput": {"command": "cat src/main.rs"}, "title": "cat src/main.rs"}))?;
                    let printed = tokio::fs::read_to_string(cwd.join("src/main.rs")).await.unwrap_or_default();
                    let data = base64::engine::general_purpose::STANDARD.encode(printed);
                    say(&worker, &session, json!({"sessionUpdate": "terminal_update", "terminalId": "run-1", "output": {"data": data}}))?;
                    say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "run-1", "status": "completed"}))?;
                    say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "edit-1", "name": "Edit", "rawInput": {"path": "src/main.rs"}, "status": "pending", "title": "Edit src/main.rs", "kind": "edit"}))?;
                    let asked: v2::RequestPermissionRequest = parse(json!({
                        "sessionId": session.to_string(),
                        "title": "Edit src/main.rs",
                        "subject": {"type": "tool_call", "toolCall": {"toolCallId": "edit-1", "kind": "edit", "title": "Edit src/main.rs"}},
                        "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"}, {"optionId": "reject", "name": "Reject", "kind": "reject_once"}]
                    }))?;
                    let answer = serde_json::to_value(worker.send_request(asked).block_task().await?)?;
                    let allowed = answer["outcome"]["optionId"] == "allow";
                    if allowed {
                        crate::edit_main(&cwd, &prompt).await;
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                    let status = if allowed { "completed" } else { "failed" };
                    say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "edit-1", "status": status}))?;
                    if prompt.contains("Present") {
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "plan-1", "name": "ExitPlanMode", "rawInput": {"plan": "1. Count the lines."}, "status": "pending", "title": "Ready to code?", "kind": "switch_mode"}))?;
                        let asked: v2::RequestPermissionRequest = parse(json!({
                            "sessionId": session.to_string(),
                            "title": "Ready to code?",
                            "subject": {"type": "tool_call", "toolCall": {"toolCallId": "plan-1", "kind": "switch_mode", "title": "Ready to code?"}},
                            "options": [{"optionId": "exit-plan-accept-edits", "name": "Yes, and auto-accept edits", "kind": "allow_always"}, {"optionId": "reject", "name": "No, keep planning", "kind": "reject_once"}]
                        }))?;
                        let answer = serde_json::to_value(worker.send_request(asked).block_task().await?)?;
                        let status = if answer["outcome"]["optionId"] == "reject" { "failed" } else { "completed" };
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "plan-1", "status": status}))?;
                    }
                    if prompt.contains("Subagent") {
                        let child = v2::SessionId::new("fake-child-1");
                        say(&worker, &session, json!({"sessionUpdate": "subagent_update", "sessionId": "fake-child-1", "title": "general-purpose", "description": "Count lines", "capabilities": {"cancel": {}}, "state": {"state": "running"}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "session_message", "messageId": "agent-1", "senderSessionId": session.to_string(), "recipientSessionId": "fake-child-1", "content": [{"type": "text", "text": "Count the lines of src/main.rs."}]}))?;
                        say(&worker, &child, json!({"sessionUpdate": "agent_message_chunk", "messageId": "child-1", "content": {"type": "text", "text": "Counted the lines."}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "subagent_update", "sessionId": "fake-child-1", "state": {"state": "idle", "stopReason": "end_turn"}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "session_message", "messageId": "agent-1:reply", "senderSessionId": "fake-child-1", "recipientSessionId": session.to_string(), "content": [{"type": "text", "text": "2 lines"}]}))?;
                    }
                    let background = prompt.contains("Background");
                    if background {
                        let answer = base64::engine::general_purpose::STANDARD.encode("Command running in background with ID: bgfake1.");
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "bg-1", "name": "Bash", "rawInput": {"command": "sleep 1 && echo BG", "run_in_background": true}, "status": "pending", "title": "sleep 1 && echo BG", "kind": "execute", "content": [{"type": "terminal", "terminalId": "bg-1"}]}))?;
                        say(&worker, &session, json!({"sessionUpdate": "terminal_update", "terminalId": "bg-1", "output": {"data": answer}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "bg-1", "status": "completed"}))?;
                    }
                    let reply = if allowed { format!("Done: {prompt}") } else { "I was not allowed to edit.".to_owned() };
                    say(&worker, &session, json!({"sessionUpdate": "agent_message_chunk", "messageId": "reply-1", "content": {"type": "text", "text": reply}}))?;
                    say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn", "usage": {"inputTokens": 1_600, "outputTokens": 240, "totalTokens": 1_840}}))?;
                    if background {
                        // The command ends once the turn has; the agent goes on about it unless stopped.
                        tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
                        cancelled.store(false, Ordering::SeqCst);
                        say(&worker, &session, json!({"sessionUpdate": "terminal_update", "terminalId": "bg-1", "exitStatus": {}}))?;
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "running"}))?;
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        if cancelled.load(Ordering::SeqCst) {
                            return say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "cancelled"}));
                        }
                        say(&worker, &session, json!({"sessionUpdate": "agent_message_chunk", "messageId": "later-bg", "content": {"type": "text", "text": "The background command printed BG."}}))?;
                        return say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn"}));
                    }
                    if prompt.contains("Later") {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "running"}))?;
                        say(&worker, &session, json!({"sessionUpdate": "agent_message_chunk", "messageId": "later-1", "content": {"type": "text", "text": "Went on by itself."}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 20, "totalTokens": 30}}))?;
                    }
                    Ok(())
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}
