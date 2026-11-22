//! The scripted agent speaking ACP v2, with `FAKE_ACP_V2` set, in the shapes
//! Claude's adapter sends: each prompt makes it say it works, think, run a
//! command whose output comes over its terminal, ask permission to edit
//! `src/main.rs`, append `FAKE_ACP_LINE` when allowed, answer and say it
//! stopped. A prompt saying `Later` makes it go on by itself once it
//! stopped, as Claude does when a background task ends, and stop again.
//! `Subagent` makes it run a subagent in a session of its own, as Claude's
//! Agent tool does, and `Aside` one left running past the turn until it is
//! cancelled; `Background` makes it leave a command running, which
//! ends after the turn and sets the agent going again, unless cancelled.
//! `Keep` makes it leave a real process running in the background, `silt-fake-agent
//! keep-running <cwd>`, whose end it reports, as Claude does, and goes on about.
//! `Unheard` makes it leave a command no process runs, whose end it never reports.
//! `Present` makes it put a plan to the person, as Claude's ExitPlanMode
//! does, by asking leave to quit its plan mode. Its session offers the commands
//! `count` and `compact` as it opens, and drops `compact` on a prompt.
//! `Suggest` makes it predict the next prompt once it stopped, as Claude's
//! adapter does, to a client that says it takes one.
//! `Which model`, `Long`, `FAKE_ACP_SLOW_SETTINGS` and `FAKE_ACP_STEERING`
//! work as over v1.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::v2::schema as v2;
use agent_client_protocol::v2::{ConnectionTo, Responder};
use agent_client_protocol::{Agent, Client, Result, Stdio};
use base64::Engine;
use serde_json::{Value, json};

#[derive(Default)]
struct State {
    cwd: HashMap<String, PathBuf>,
    mode: HashMap<String, String>,
    model: HashMap<String, String>,
    /// The client takes suggested prompts.
    suggestions: bool,
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

fn say(connection: &ConnectionTo<Client>, session: &v2::SessionId, update: Value) -> Result<()> {
    connection.send_notification(v2::UpdateSessionNotification::new(session.clone(), parse::<v2::SessionUpdate>(update)?))
}

pub async fn main() -> Result<()> {
    let state = Arc::new(Mutex::new(State::default()));
    let (created, resumed, configured, prompted, initialized) =
        (state.clone(), state.clone(), state.clone(), state.clone(), state);
    let stopping = Arc::new(AtomicBool::new(false));
    let cancelling = stopping.clone();
    Agent
        .v2()
        .name("silt-fake-agent")
        .on_receive_notification(
            async move |_cancel: v2::CancelSessionNotification, _connection: ConnectionTo<Client>| {
                cancelling.store(true, Ordering::SeqCst);
                crate::steering::cancel();
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: v2::InitializeRequest, responder: Responder<v2::InitializeResponse>, _connection: ConnectionTo<Client>| {
                initialized.lock().expect("lock").suggestions =
                    request.capabilities.session.as_ref().is_some_and(|session| session.prompt_suggestions.is_some());
                let mut response = json!({
                    "protocolVersion": request.protocol_version,
                    "info": {"name": "fake-agent", "version": "2.0"},
                    "capabilities": {"session": {"prompt": {"image": {}, "embeddedContext": {}}}},
                    "authMethods": crate::sign_in_methods("methodId")
                });
                if let Some(meta) = crate::steering::meta() {
                    response["_meta"] = meta;
                }
                responder.respond(parse(response)?)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: v2::NewSessionRequest, responder: Responder<v2::NewSessionResponse>, _connection: ConnectionTo<Client>| {
                let mut state = created.lock().expect("lock");
                let id = format!("fake-v2-{}", state.cwd.len() + 1);
                state.cwd.insert(id.clone(), request.cwd.into_inner());
                responder.respond(parse(json!({"sessionId": id, "configOptions": options("code", "fast"), "availableCommands": commands(false)}))?)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: v2::ResumeSessionRequest, responder: Responder<v2::ResumeSessionResponse>, _connection: ConnectionTo<Client>| {
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
            async move |request: v2::SetSessionConfigOptionRequest, responder: Responder<v2::SetSessionConfigOptionResponse>, connection: ConnectionTo<Client>| {
                let value = serde_json::to_value(&request.value)?.get("value").and_then(Value::as_str).unwrap_or_default().to_owned();
                let (id, config) = (request.session_id.to_string(), request.config_id.to_string());
                if !matches!((config.as_str(), value.as_str()), ("mode", "code" | "plan") | ("model", "fast" | "smart")) {
                    return responder.respond_with_error(agent_client_protocol::Error::invalid_params().data("no such setting"));
                }
                let configured = configured.clone();
                connection.spawn(async move {
                    crate::settle().await;
                    let mut state = configured.lock().expect("lock");
                    let kept = if config == "mode" { &mut state.mode } else { &mut state.model };
                    kept.insert(id.clone(), value);
                    let mode = state.mode.get(&id).cloned().unwrap_or_else(|| "code".into());
                    let model = state.model.get(&id).cloned().unwrap_or_else(|| "fast".into());
                    responder.respond(parse(json!({"configOptions": options(&mode, &model)}))?)
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: v2::PromptRequest, responder: Responder<v2::PromptResponse>, connection: ConnectionTo<Client>| {
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
                    if prompt.contains("Which model") {
                        let model = state.lock().expect("lock").model.get(&session.to_string()).cloned().unwrap_or_else(|| "fast".into());
                        say(&worker, &session, json!({"sessionUpdate": "agent_message_chunk", "messageId": "model-1", "content": {"type": "text", "text": format!("Model: {model}")}}))?;
                        return say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn"}));
                    }
                    // As Claude's adapter ends a turn its expired sign-in failed.
                    if !crate::signed_in() {
                        say(&worker, &session, json!({"sessionUpdate": "agent_message_chunk", "messageId": "failed-1", "content": {"type": "text", "text": "Failed to authenticate."}}))?;
                        let refused = json!({"code": -32000, "message": "Authentication required"});
                        return say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "error", "error": refused}));
                    }
                    say(&worker, &session, json!({"sessionUpdate": "agent_thought_chunk", "messageId": "thought-1", "content": {"type": "text", "text": "Thinking about the prompt."}}))?;
                    say(&worker, &session, json!({"sessionUpdate": "usage_update", "used": 53_000, "size": 200_000}))?;
                    if prompt.contains("Long") {
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "long-1", "name": "Bash", "rawInput": {"command": "cargo build"}, "status": "in_progress", "title": "cargo build", "kind": "execute", "content": [{"type": "terminal", "terminalId": "long-1"}]}))?;
                        let mut heard = 0;
                        let ran = crate::steering::long(|steer| {
                            heard += 1;
                            say(&worker, &session, json!({"sessionUpdate": "agent_message_chunk", "messageId": format!("heard-{heard}"), "content": {"type": "text", "text": format!("Heard while the command ran: {steer}")}}))
                        })
                        .await?;
                        let killed = matches!(ran, crate::steering::Ran::Killed);
                        let (status, output) = if killed { ("failed", "The command was killed.") } else { ("completed", "The command ran to its end.") };
                        let data = base64::engine::general_purpose::STANDARD.encode(output);
                        say(&worker, &session, json!({"sessionUpdate": "terminal_update", "terminalId": "long-1", "output": {"data": data}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "long-1", "status": status}))?;
                        if killed {
                            return say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "cancelled"}));
                        }
                        say(&worker, &session, json!({"sessionUpdate": "agent_message_chunk", "messageId": "reply-1", "content": {"type": "text", "text": format!("Done: {prompt}")}}))?;
                        return say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn"}));
                    }
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
                        say(&worker, &session, json!({"sessionUpdate": "subagent_update", "sessionId": "fake-child-1", "title": "Count lines", "description": "general-purpose", "capabilities": {"cancel": {}}, "state": {"state": "running"}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "session_message", "messageId": "agent-1", "senderSessionId": session.to_string(), "recipientSessionId": "fake-child-1", "content": [{"type": "text", "text": "Count the lines of src/main.rs."}]}))?;
                        say(&worker, &child, json!({"sessionUpdate": "agent_message_chunk", "messageId": "child-1", "content": {"type": "text", "text": "Counted the lines."}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "subagent_update", "sessionId": "fake-child-1", "state": {"state": "idle", "stopReason": "end_turn"}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "session_message", "messageId": "agent-1:reply", "senderSessionId": "fake-child-1", "recipientSessionId": session.to_string(), "content": [{"type": "text", "text": "2 lines"}]}))?;
                    }
                    let aside = prompt.contains("Aside");
                    if aside {
                        say(&worker, &session, json!({"sessionUpdate": "subagent_update", "sessionId": "fake-child-2", "title": "general-purpose", "description": "Watch the tests", "capabilities": {"cancel": {}}, "state": {"state": "running"}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "session_message", "messageId": "aside-1", "senderSessionId": session.to_string(), "recipientSessionId": "fake-child-2", "content": [{"type": "text", "text": "Watch the tests until told to stop."}]}))?;
                    }
                    let background = prompt.contains("Background");
                    if background {
                        let answer = base64::engine::general_purpose::STANDARD.encode("Command running in background with ID: bgfake1.");
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "bg-1", "name": "Bash", "rawInput": {"command": "sleep 1 && echo BG", "run_in_background": true}, "status": "pending", "title": "sleep 1 && echo BG", "kind": "execute", "content": [{"type": "terminal", "terminalId": "bg-1"}]}))?;
                        say(&worker, &session, json!({"sessionUpdate": "terminal_update", "terminalId": "bg-1", "output": {"data": answer}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "bg-1", "status": "completed"}))?;
                    }
                    if prompt.contains("Unheard") {
                        let answer = base64::engine::general_purpose::STANDARD.encode("Command running in background with ID: bgunheard1.");
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "unheard-1", "name": "Bash", "rawInput": {"command": "sleep 1 && echo UNHEARD", "run_in_background": true}, "status": "pending", "title": "sleep 1 && echo UNHEARD", "kind": "execute", "content": [{"type": "terminal", "terminalId": "unheard-1"}]}))?;
                        say(&worker, &session, json!({"sessionUpdate": "terminal_update", "terminalId": "unheard-1", "output": {"data": answer}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "unheard-1", "status": "completed"}))?;
                    }
                    if prompt.contains("Keep") {
                        let command = format!("keep-running {}", cwd.display());
                        let answer = base64::engine::general_purpose::STANDARD.encode("Command running in background with ID: bgkeep1.");
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "keep-1", "name": "Bash", "rawInput": {"command": command, "run_in_background": true}, "status": "pending", "title": command, "kind": "execute", "content": [{"type": "terminal", "terminalId": "keep-1"}]}))?;
                        let program = std::env::current_exe().map_err(agent_client_protocol::Error::into_internal_error)?;
                        let mut kept = tokio::process::Command::new(program)
                            .arg("keep-running")
                            .arg(&cwd)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .spawn()
                            .map_err(agent_client_protocol::Error::into_internal_error)?;
                        say(&worker, &session, json!({"sessionUpdate": "terminal_update", "terminalId": "keep-1", "output": {"data": answer}}))?;
                        say(&worker, &session, json!({"sessionUpdate": "tool_call_update", "toolCallId": "keep-1", "status": "completed"}))?;
                        let (ended, session, cancelled) = (worker.clone(), session.clone(), cancelled.clone());
                        worker.spawn(async move {
                            let _ = kept.wait().await;
                            cancelled.store(false, Ordering::SeqCst);
                            say(&ended, &session, json!({"sessionUpdate": "terminal_update", "terminalId": "keep-1", "exitStatus": {}}))?;
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            say(&ended, &session, json!({"sessionUpdate": "state_update", "state": "running"}))?;
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            if cancelled.load(Ordering::SeqCst) {
                                return say(&ended, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "cancelled"}));
                            }
                            say(&ended, &session, json!({"sessionUpdate": "agent_message_chunk", "messageId": "later-keep", "content": {"type": "text", "text": "The kept command ended."}}))?;
                            say(&ended, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn"}))
                        })?;
                    }
                    let reply = if allowed { format!("Done: {prompt}") } else { "I was not allowed to edit.".to_owned() };
                    say(&worker, &session, json!({"sessionUpdate": "agent_message_chunk", "messageId": "reply-1", "content": {"type": "text", "text": reply}}))?;
                    say(&worker, &session, json!({"sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn", "usage": {"inputTokens": 1_600, "outputTokens": 240, "totalTokens": 1_840}}))?;
                    if prompt.contains("Suggest") && state.lock().expect("lock").suggestions {
                        let suggestion = json!([{"prompt": [{"type": "text", "text": "Add a test for it"}]}]);
                        say(&worker, &session, json!({"sessionUpdate": "prompt_suggestions_update", "suggestions": suggestion}))?;
                    }
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
                    if aside {
                        while !cancelled.load(Ordering::SeqCst) {
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                        return say(&worker, &session, json!({"sessionUpdate": "subagent_update", "sessionId": "fake-child-2", "state": {"state": "idle", "stopReason": "cancelled"}}));
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
        .on_receive_request(
            async move |request: agent_client_protocol::UntypedMessage, responder: Responder<Value>, _connection: ConnectionTo<Client>| {
                match crate::steering::answer(&request.method, &request.params) {
                    Ok(answer) => responder.respond(answer),
                    Err(error) => responder.respond_with_error(error),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}
