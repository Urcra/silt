//! `silt agent local` as an ACP agent: sessions it keeps in files of its
//! own so they load again, a model setting from the gateway's models, and
//! modes from Chat, where it only talks, to full access.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, EmbeddedResourceResource, Implementation,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse,
    PermissionOption, PermissionOptionKind, PromptCapabilities, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionConfigOption, SessionConfigOptionCategory, SessionConfigOptionValue,
    SessionConfigSelectOption, SessionId, SessionMode, SessionModeState, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, SetSessionModeRequest, SetSessionModeResponse, StopReason,
    TextContent, ToolCall, ToolCallContent, ToolCallLocation, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
    UsageUpdate,
};
use agent_client_protocol::{Agent, Client, Error, Stdio};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use crate::conversation::{Call, Conversation, Entry, Message, Role, is_session_id};
use crate::model::{Ended, Gateway, Listed};
use crate::tools::{self, Kind, Leeway, Planned};

pub const CHAT: &str = "chat";
/// A new session's mode until the daemon chooses one by its preset.
pub const FIRST_MODE: &str = "default";
/// Each mode's id, name and what it lets `local` do, as Claude Code's ids
/// say, so the daemon maps them to presets (ADR 0048).
pub const MODES: [(&str, &str, &str); 4] = [
    (CHAT, "Chat", "Only talks: no tools"),
    ("default", "Ask for everything", "Reads freely; asks before edits and commands"),
    ("acceptEdits", "Ask before commands", "Reads and edits freely; asks before commands"),
    ("bypassPermissions", "Full access", "Asks only before touching a path outside the project"),
];
/// Model calls in one turn, at most, so a model stuck calling tools ends.
const MOST_STEPS: usize = 40;
const NO_MODEL: &str = "No local model is downloaded yet. Download one in Settings › Local models, or with `silt model pull`.";

struct Session {
    cwd: PathBuf,
    file: PathBuf,
    conversation: Conversation,
    cancel: Option<watch::Sender<bool>>,
}

struct State {
    home: PathBuf,
    gateway: Option<Gateway>,
    sessions: HashMap<String, Session>,
}

type Shared = Arc<Mutex<State>>;

fn sessions_dir(home: &Path) -> PathBuf {
    home.join("sessions")
}

fn text(content: impl Into<String>) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(content.into())))
}

fn notify(connection: &ConnectionTo<Client>, session: &SessionId, update: SessionUpdate) -> agent_client_protocol::Result<()> {
    connection.send_notification(SessionNotification::new(session.clone(), update))
}

fn modes(current: &str) -> SessionModeState {
    let available = MODES.iter().map(|(id, name, description)| SessionMode::new(*id, *name).description(*description)).collect();
    SessionModeState::new(current.to_owned(), available)
}

fn settings(models: &[Listed], current: &str) -> Vec<SessionConfigOption> {
    if models.is_empty() {
        return Vec::new();
    }
    let choices: Vec<SessionConfigSelectOption> = models
        .iter()
        .map(|model| {
            SessionConfigSelectOption::new(
                model.id.clone(),
                if model.name.is_empty() { model.id.clone() } else { model.name.clone() },
            )
        })
        .collect();
    vec![SessionConfigOption::select("model", "Model", current.to_owned(), choices).category(SessionConfigOptionCategory::Model)]
}

/// The model a session uses: its own choice while it's still here, else
/// `SILT_LOCAL_MODEL`'s, else the first.
fn chosen<'a>(models: &'a [Listed], wanted: Option<&str>) -> Option<&'a Listed> {
    let preferred = std::env::var("SILT_LOCAL_MODEL").ok();
    [wanted, preferred.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|id| models.iter().find(|model| model.id == id))
        .or_else(|| models.first())
}

async fn listed(gateway: Option<&Gateway>) -> Vec<Listed> {
    match gateway {
        Some(gateway) => gateway.models().await.unwrap_or_default(),
        None => Vec::new(),
    }
}

async fn append(file: &Path, entry: &Entry) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut out = tokio::fs::OpenOptions::new().create(true).append(true).open(file).await?;
    let mut line = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
    line.push(b'\n');
    out.write_all(&line).await
}

/// A prompt's text, with any files it carries set out after it.
fn prompt_text(blocks: &[ContentBlock]) -> String {
    let mut text = String::new();
    let mut files = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text(part) => text.push_str(&part.text),
            ContentBlock::Resource(resource) => {
                if let EmbeddedResourceResource::TextResourceContents(file) = &resource.resource {
                    files.push(format!("<file uri=\"{}\">\n{}\n</file>", file.uri, file.text));
                }
            }
            _ => {}
        }
    }
    if files.is_empty() { text } else { format!("{text}\n\n{}", files.join("\n\n")) }
}

fn internal(error: impl std::fmt::Display) -> Error {
    Error::internal_error().data(error.to_string())
}

pub async fn serve(home: PathBuf) -> agent_client_protocol::Result<()> {
    let state: Shared = Arc::new(Mutex::new(State { home, gateway: Gateway::from_env(), sessions: HashMap::new() }));
    let (created, loading, moded, configured, prompted, cancelled) =
        (state.clone(), state.clone(), state.clone(), state.clone(), state.clone(), state.clone());
    Agent
        .v1()
        .name("silt-local")
        .on_receive_request(
            async move |_initialize: InitializeRequest, responder, _connection| {
                responder.respond(
                    InitializeResponse::new(ProtocolVersion::V1)
                        .agent_capabilities(
                            AgentCapabilities::new()
                                .load_session(true)
                                .prompt_capabilities(PromptCapabilities::new().embedded_context(true)),
                        )
                        .agent_info(Implementation::new("silt-local", env!("CARGO_PKG_VERSION"))),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _connection| {
                let (home, gateway) = {
                    let state = created.lock().expect("lock");
                    (state.home.clone(), state.gateway.clone())
                };
                let models = listed(gateway.as_ref()).await;
                let id = uuid::Uuid::new_v4().simple().to_string();
                let file = sessions_dir(&home).join(format!("{id}.jsonl"));
                let mut conversation = Conversation::default();
                if let Some(model) = chosen(&models, None) {
                    let entry = Entry::Model { id: model.id.clone() };
                    append(&file, &entry).await.map_err(internal)?;
                    conversation.apply(entry);
                }
                let current = conversation.model.clone().unwrap_or_default();
                let session = Session { cwd: request.cwd, file, conversation, cancel: None };
                created.lock().expect("lock").sessions.insert(id.clone(), session);
                responder.respond(
                    NewSessionResponse::new(SessionId::new(id))
                        .modes(modes(FIRST_MODE))
                        .config_options(settings(&models, &current)),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: LoadSessionRequest, responder, connection| {
                let id = request.session_id.to_string();
                if !is_session_id(&id) {
                    return responder.respond_with_error(Error::invalid_params().data("no such session"));
                }
                let (home, gateway) = {
                    let state = loading.lock().expect("lock");
                    (state.home.clone(), state.gateway.clone())
                };
                let file = sessions_dir(&home).join(format!("{id}.jsonl"));
                let Ok(lines) = tokio::fs::read_to_string(&file).await else {
                    return responder.respond_with_error(Error::invalid_params().data("no such session"));
                };
                let conversation = Conversation::replay(&lines);
                for message in conversation.said.iter().filter(|message| !message.text.is_empty()) {
                    let chunk = text(message.text.clone());
                    let update = match message.role {
                        Role::User => SessionUpdate::UserMessageChunk(chunk),
                        Role::Assistant => SessionUpdate::AgentMessageChunk(chunk),
                        Role::Tool => continue,
                    };
                    notify(&connection, &request.session_id, update)?;
                }
                let models = listed(gateway.as_ref()).await;
                let current = chosen(&models, conversation.model.as_deref()).map(|model| model.id.clone()).unwrap_or_default();
                let mode = conversation.mode.clone().unwrap_or_else(|| FIRST_MODE.to_owned());
                let session = Session { cwd: request.cwd, file, conversation, cancel: None };
                loading.lock().expect("lock").sessions.insert(id, session);
                responder.respond(LoadSessionResponse::new().modes(modes(&mode)).config_options(settings(&models, &current)))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionModeRequest, responder, _connection| {
                let mode = request.mode_id.to_string();
                if !MODES.iter().any(|(id, _, _)| *id == mode) {
                    return responder.respond_with_error(Error::invalid_params().data("no such mode"));
                }
                let file = {
                    let mut state = moded.lock().expect("lock");
                    let Some(session) = state.sessions.get_mut(&request.session_id.to_string()) else {
                        return responder.respond_with_error(Error::invalid_params().data("no such session"));
                    };
                    session.conversation.apply(Entry::Mode { id: mode.clone() });
                    session.file.clone()
                };
                append(&file, &Entry::Mode { id: mode }).await.map_err(internal)?;
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
                let gateway = configured.lock().expect("lock").gateway.clone();
                let models = listed(gateway.as_ref()).await;
                if request.config_id.to_string() != "model" || !models.iter().any(|model| model.id == value) {
                    return responder.respond_with_error(Error::invalid_params().data("no such setting"));
                }
                let file = {
                    let mut state = configured.lock().expect("lock");
                    let Some(session) = state.sessions.get_mut(&request.session_id.to_string()) else {
                        return responder.respond_with_error(Error::invalid_params().data("no such session"));
                    };
                    session.conversation.apply(Entry::Model { id: value.clone() });
                    session.file.clone()
                };
                append(&file, &Entry::Model { id: value.clone() }).await.map_err(internal)?;
                responder.respond(SetSessionConfigOptionResponse::new(settings(&models, &value)))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |cancel: CancelNotification, _connection: ConnectionTo<Client>| {
                if let Some(session) = cancelled.lock().expect("lock").sessions.get(&cancel.session_id.to_string())
                    && let Some(cancel) = &session.cancel
                {
                    let _ = cancel.send(true);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection| {
                let state = prompted.clone();
                let worker = connection.clone();
                connection.spawn(async move {
                    let stop = turn(&state, &worker, &request).await?;
                    responder.respond(PromptResponse::new(stop))
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}

/// One prompt, and the model's work on it: its answer, and the tools it
/// calls on the way when the mode offers them.
async fn turn(
    state: &Shared,
    connection: &ConnectionTo<Client>,
    request: &PromptRequest,
) -> agent_client_protocol::Result<StopReason> {
    let id = request.session_id.to_string();
    let said = prompt_text(&request.prompt);
    let (receiver, file, cwd, gateway, conversation) = {
        let mut state = state.lock().expect("lock");
        let gateway = state.gateway.clone();
        let Some(session) = state.sessions.get_mut(&id) else {
            return Err(Error::invalid_params().data("no such session"));
        };
        let (sender, receiver) = watch::channel(false);
        session.cancel = Some(sender);
        session.conversation.apply(Entry::Said(Message::said(Role::User, said.clone())));
        (receiver, session.file.clone(), session.cwd.clone(), gateway, session.conversation.clone())
    };
    append(&file, &Entry::Said(Message::said(Role::User, said))).await.map_err(internal)?;
    let models = listed(gateway.as_ref()).await;
    let (Some(gateway), Some(model)) = (gateway, chosen(&models, conversation.model.as_deref()).cloned()) else {
        notify(connection, &request.session_id, SessionUpdate::AgentMessageChunk(text(NO_MODEL)))?;
        return Ok(StopReason::EndTurn);
    };
    let leeway = Leeway::of_mode(conversation.mode.as_deref().unwrap_or(FIRST_MODE));
    let offered = (leeway != Leeway::None).then(tools::definitions);
    let session = request.session_id.clone();
    let mut conversation = conversation;
    let mut stop = StopReason::EndTurn;
    for _ in 0..MOST_STEPS {
        let streamed = gateway
            .stream(&conversation.request(&model.id, offered.as_ref()), receiver.clone(), |piece| {
                if let Some(thought) = &piece.thought {
                    let _ = notify(connection, &session, SessionUpdate::AgentThoughtChunk(text(thought.clone())));
                }
                if let Some(more) = &piece.text {
                    let _ = notify(connection, &session, SessionUpdate::AgentMessageChunk(text(more.clone())));
                }
            })
            .await;
        let (reply, calls, usage) = match streamed {
            Ok(Ended::Done { text, calls, usage }) => (text, calls, usage),
            Ok(Ended::Cancelled { text }) => {
                stop = StopReason::Cancelled;
                (text, Vec::new(), None)
            }
            Err(why) => {
                notify(connection, &session, SessionUpdate::AgentMessageChunk(text(format!("The local model failed: {why}"))))?;
                break;
            }
        };
        conversation =
            keep(state, &id, &file, Message { role: Role::Assistant, text: reply, calls: calls.clone(), answers: None }).await?;
        if let Some(usage) = usage {
            let used = usage.prompt + usage.completion;
            notify(connection, &session, SessionUpdate::UsageUpdate(UsageUpdate::new(used, model.context)))?;
            if conversation.should_compact(used, model.context)
                && let Some(ask) = conversation.compaction_request(&model.id)
                && let Ok(summary) = gateway.complete(&ask).await
            {
                let compacted = conversation.compacted(summary);
                append(&file, &compacted).await.map_err(internal)?;
                if let Some(kept) = state.lock().expect("lock").sessions.get_mut(&id) {
                    kept.conversation.apply(compacted.clone());
                }
                conversation.apply(compacted);
            }
        }
        if calls.is_empty() || stop == StopReason::Cancelled {
            break;
        }
        for call in &calls {
            let result = if *receiver.borrow() {
                tools::Outcome::failed("not run: the turn was cancelled")
            } else {
                use_tool(connection, &session, call, &cwd, leeway, receiver.clone()).await?
            };
            conversation = keep(state, &id, &file, Message::result(&call.id, result.text)).await?;
        }
        if *receiver.borrow() {
            stop = StopReason::Cancelled;
            break;
        }
    }
    if let Some(kept) = state.lock().expect("lock").sessions.get_mut(&id) {
        kept.cancel = None;
    }
    Ok(stop)
}

/// Keeps a message in the session and its file, and answers with the
/// conversation as it now is.
async fn keep(state: &Shared, id: &str, file: &Path, message: Message) -> agent_client_protocol::Result<Conversation> {
    let entry = Entry::Said(message);
    append(file, &entry).await.map_err(internal)?;
    let mut state = state.lock().expect("lock");
    let session = state.sessions.get_mut(id).ok_or_else(|| Error::invalid_params().data("no such session"))?;
    session.conversation.apply(entry);
    Ok(session.conversation.clone())
}

fn kind(kind: Kind) -> ToolKind {
    match kind {
        Kind::Read => ToolKind::Read,
        Kind::Search => ToolKind::Search,
        Kind::Edit => ToolKind::Edit,
        Kind::Execute => ToolKind::Execute,
    }
}

/// One tool call, shown to the person as it goes: asked for when the mode
/// says so, then run, then its result.
async fn use_tool(
    connection: &ConnectionTo<Client>,
    session: &SessionId,
    call: &Call,
    cwd: &Path,
    leeway: Leeway,
    cancel: watch::Receiver<bool>,
) -> agent_client_protocol::Result<tools::Outcome> {
    let planned: Planned = match tools::plan(call, cwd) {
        Ok(planned) => planned,
        Err(why) => {
            let shown = ToolCall::new(call.id.clone(), call.name.clone())
                .status(ToolCallStatus::Failed)
                .raw_output(serde_json::json!(why));
            notify(connection, session, SessionUpdate::ToolCall(shown))?;
            return Ok(tools::Outcome::failed(why));
        }
    };
    let shown = ToolCall::new(call.id.clone(), planned.title.clone())
        .kind(kind(planned.kind))
        .status(ToolCallStatus::Pending)
        .locations(planned.paths.iter().map(|path| ToolCallLocation::new(path.clone())).collect())
        .raw_input(planned.input.clone());
    notify(connection, session, SessionUpdate::ToolCall(shown))?;
    if planned.needs_asking(leeway) {
        let options = vec![
            PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
            PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
        ];
        let asked = RequestPermissionRequest::new(
            session.clone(),
            ToolCallUpdate::new(call.id.clone(), ToolCallUpdateFields::new().title(planned.title.clone())),
            options,
        );
        let answer = connection.send_request(asked).block_task().await?;
        let allowed =
            matches!(&answer.outcome, RequestPermissionOutcome::Selected(selected) if selected.option_id.to_string() == "allow");
        if !allowed {
            let refused = ToolCallUpdateFields::new().status(ToolCallStatus::Failed).raw_output(serde_json::json!("declined"));
            notify(connection, session, SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(call.id.clone(), refused)))?;
            return Ok(tools::Outcome::failed("The person declined this; don't try it again unasked."));
        }
    }
    let running = ToolCallUpdateFields::new().status(ToolCallStatus::InProgress);
    notify(connection, session, SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(call.id.clone(), running)))?;
    let outcome = tools::run(&planned, &call.name, cwd, cancel).await;
    let mut content: Vec<ToolCallContent> = Vec::new();
    if let Some((path, before, after)) = &outcome.diff {
        content.push(ToolCallContent::Diff(
            agent_client_protocol::v1::schema::Diff::new(path.clone(), after.clone()).old_text(before.clone()),
        ));
    } else {
        content.push(ToolCallContent::from(text_block(&outcome.text)));
    }
    let status = if outcome.failed { ToolCallStatus::Failed } else { ToolCallStatus::Completed };
    let done = ToolCallUpdateFields::new().status(status).content(content).raw_output(serde_json::json!(outcome.text));
    notify(connection, session, SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(call.id.clone(), done)))?;
    Ok(outcome)
}

fn text_block(said: &str) -> ContentBlock {
    ContentBlock::Text(TextContent::new(said.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> Listed {
        Listed { id: id.to_owned(), name: String::new(), context: 4096 }
    }

    #[test]
    fn a_sessions_model_is_its_own_while_its_here_else_the_first() {
        let models = [model("a"), model("b")];
        assert_eq!(chosen(&models, Some("b")).map(|model| model.id.as_str()), Some("b"));
        assert_eq!(chosen(&models, Some("gone")).map(|model| model.id.as_str()), Some("a"));
        assert_eq!(chosen(&[], Some("a")), None);
    }

    #[test]
    fn a_prompts_files_follow_its_text() {
        let file: ContentBlock = serde_json::from_value(serde_json::json!({
            "type": "resource", "resource": {"uri": "file:///app.log", "text": "ERROR boom"}
        }))
        .expect("a resource");
        let prompt = prompt_text(&[ContentBlock::Text(TextContent::new("Why?")), file]);
        assert_eq!(prompt, "Why?\n\n<file uri=\"file:///app.log\">\nERROR boom\n</file>");
    }
}
