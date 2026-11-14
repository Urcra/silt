//! `silt agent local` as an ACP agent: sessions it keeps in files of its
//! own so they load again, a model setting from the gateway's models, and
//! Chat mode, where it only talks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, EmbeddedResourceResource, Implementation,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse,
    PromptCapabilities, PromptRequest, PromptResponse, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigOptionValue, SessionConfigSelectOption, SessionId, SessionMode, SessionModeState, SessionNotification,
    SessionUpdate, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, SetSessionModeRequest, SetSessionModeResponse,
    StopReason, TextContent, UsageUpdate,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Error, Stdio};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use crate::conversation::{Conversation, Entry, Message, Role, is_session_id};
use crate::model::{Ended, Gateway, Listed};

pub const CHAT: &str = "chat";
const NO_MODEL: &str = "No local model is downloaded yet. Download one in Settings › Local models, or with `silt model pull`.";

struct Session {
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

fn modes() -> SessionModeState {
    SessionModeState::new(CHAT.to_owned(), vec![SessionMode::new(CHAT, "Chat").description("Only talks: no tools")])
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
    let (created, loading, configured, prompted, cancelled) =
        (state.clone(), state.clone(), state.clone(), state.clone(), state.clone());
    Agent
        .builder()
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
            async move |_request: NewSessionRequest, responder, _connection| {
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
                created.lock().expect("lock").sessions.insert(id.clone(), Session { file, conversation, cancel: None });
                responder.respond(
                    NewSessionResponse::new(SessionId::new(id)).modes(modes()).config_options(settings(&models, &current)),
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
                for message in &conversation.said {
                    let chunk = text(message.text.clone());
                    let update = match message.role {
                        Role::User => SessionUpdate::UserMessageChunk(chunk),
                        Role::Assistant => SessionUpdate::AgentMessageChunk(chunk),
                    };
                    notify(&connection, &request.session_id, update)?;
                }
                let models = listed(gateway.as_ref()).await;
                let current = chosen(&models, conversation.model.as_deref()).map(|model| model.id.clone()).unwrap_or_default();
                loading.lock().expect("lock").sessions.insert(id, Session { file, conversation, cancel: None });
                responder.respond(LoadSessionResponse::new().modes(modes()).config_options(settings(&models, &current)))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionModeRequest, responder, _connection| {
                if request.mode_id.to_string() != CHAT {
                    return responder.respond_with_error(Error::invalid_params().data("no such mode"));
                }
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

/// One prompt and the model's answer to it.
async fn turn(
    state: &Shared,
    connection: &ConnectionTo<Client>,
    request: &PromptRequest,
) -> agent_client_protocol::Result<StopReason> {
    let id = request.session_id.to_string();
    let said = prompt_text(&request.prompt);
    let (receiver, file, gateway, conversation) = {
        let mut state = state.lock().expect("lock");
        let gateway = state.gateway.clone();
        let Some(session) = state.sessions.get_mut(&id) else {
            return Err(Error::invalid_params().data("no such session"));
        };
        let (sender, receiver) = watch::channel(false);
        session.cancel = Some(sender);
        session.conversation.apply(Entry::Said(Message { role: Role::User, text: said.clone() }));
        (receiver, session.file.clone(), gateway, session.conversation.clone())
    };
    append(&file, &Entry::Said(Message { role: Role::User, text: said })).await.map_err(internal)?;
    let models = listed(gateway.as_ref()).await;
    let (Some(gateway), Some(model)) = (gateway, chosen(&models, conversation.model.as_deref()).cloned()) else {
        notify(connection, &request.session_id, SessionUpdate::AgentMessageChunk(text(NO_MODEL)))?;
        return Ok(StopReason::EndTurn);
    };
    let session = request.session_id.clone();
    let streamed = gateway
        .stream(&conversation.request(&model.id), receiver, |piece| {
            if let Some(thought) = &piece.thought {
                let _ = notify(connection, &session, SessionUpdate::AgentThoughtChunk(text(thought.clone())));
            }
            if let Some(more) = &piece.text {
                let _ = notify(connection, &session, SessionUpdate::AgentMessageChunk(text(more.clone())));
            }
        })
        .await;
    let (reply, usage, stop) = match streamed {
        Ok(Ended::Done { text, usage }) => (text, usage, StopReason::EndTurn),
        Ok(Ended::Cancelled { text }) => (text, None, StopReason::Cancelled),
        Err(why) => {
            notify(
                connection,
                &request.session_id,
                SessionUpdate::AgentMessageChunk(text(format!("The local model failed: {why}"))),
            )?;
            return Ok(StopReason::EndTurn);
        }
    };
    let entry = Entry::Said(Message { role: Role::Assistant, text: reply });
    append(&file, &entry).await.map_err(internal)?;
    let conversation = {
        let mut state = state.lock().expect("lock");
        let Some(session) = state.sessions.get_mut(&id) else {
            return Ok(stop);
        };
        session.cancel = None;
        session.conversation.apply(entry);
        session.conversation.clone()
    };
    if let Some(usage) = usage {
        let used = usage.prompt + usage.completion;
        notify(connection, &request.session_id, SessionUpdate::UsageUpdate(UsageUpdate::new(used, model.context)))?;
        if conversation.should_compact(used, model.context)
            && let Some(ask) = conversation.compaction_request(&model.id)
            && let Ok(summary) = gateway.complete(&ask).await
        {
            let compacted = conversation.compacted(summary);
            append(&file, &compacted).await.map_err(internal)?;
            if let Some(session) = state.lock().expect("lock").sessions.get_mut(&id) {
                session.conversation.apply(compacted);
            }
        }
    }
    Ok(stop)
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
