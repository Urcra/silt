//! Who is looking at a thread right now, and their draft prompts (ADR 0014).
//! Kept in memory only: it matters for seconds, and a restart forgets it.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Extension, Query, State};
use serde::Deserialize;
use silt_proto::{AgentActivity, Presence, PresenceReply, PresenceUpdate};
use silt_types::{DeviceId, ThreadId};

use crate::{ApiError, Caller, Server, UserId, not_found};

const TTL: Duration = Duration::from_secs(15);
/// Longest draft or other text one viewer's presence carries.
const MAX_TEXT: usize = 8 * 1024;

#[derive(Debug)]
pub(crate) struct Entry {
    user: String,
    file: Option<String>,
    line: Option<u32>,
    draft: String,
    agent: Option<AgentActivity>,
    at: Instant,
}

/// Every viewer, by thread and device.
#[derive(Debug, Default)]
pub(crate) struct Board {
    entries: HashMap<(ThreadId, DeviceId), Entry>,
}

impl Board {
    fn others(&mut self, thread: ThreadId, except: DeviceId) -> Vec<Presence> {
        self.entries.retain(|_, entry| entry.at.elapsed() < TTL);
        let mut people: Vec<Presence> = self
            .entries
            .iter()
            .filter(|((on, device), _)| *on == thread && *device != except)
            .map(|((_, device), entry)| Presence {
                user: entry.user.clone(),
                device: device.to_string(),
                file: entry.file.clone(),
                line: entry.line,
                draft: entry.draft.clone(),
                agent: entry.agent.clone(),
                age_ms: u64::try_from(entry.at.elapsed().as_millis()).unwrap_or(u64::MAX),
            })
            .collect();
        people.sort_by(|a, b| a.user.cmp(&b.user).then(a.device.cmp(&b.device)));
        people
    }
}

async fn user_name(server: &Server, user: UserId) -> Result<String, ApiError> {
    Ok(crate::accounts::user_by_id(server.db(), user).await?.map(|user| user.name).unwrap_or_default())
}

pub(crate) async fn update(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(update): Json<PresenceUpdate>,
) -> Result<Json<PresenceReply>, ApiError> {
    if !server.viewer(caller.user).await?.sees_thread(update.thread) {
        return Err(not_found());
    }
    let user = user_name(&server, caller.user).await?;
    let mut board = server.shared.presence.lock().await;
    if update.leaving {
        board.entries.remove(&(update.thread, caller.device));
    } else {
        // Every viewer of the thread gets a copy on every heartbeat.
        let clip = |text: String| text.chars().take(MAX_TEXT).collect::<String>();
        board.entries.insert(
            (update.thread, caller.device),
            Entry {
                user,
                file: update.file.map(clip),
                line: update.line,
                draft: clip(update.draft),
                agent: update.agent.map(|agent| silt_proto::AgentActivity {
                    name: clip(agent.name),
                    state: agent.state,
                    streaming: clip(agent.streaming),
                }),
                at: Instant::now(),
            },
        );
    }
    Ok(Json(PresenceReply { people: board.others(update.thread, caller.device) }))
}

#[derive(Deserialize)]
pub(crate) struct ThreadQuery {
    thread: String,
}

pub(crate) async fn list(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Query(query): Query<ThreadQuery>,
) -> Result<Json<PresenceReply>, ApiError> {
    let thread: ThreadId = query.thread.parse().map_err(|_| crate::bad_request("invalid thread id"))?;
    if !server.viewer(caller.user).await?.sees_thread(thread) {
        return Err(not_found());
    }
    let people = server.shared.presence.lock().await.others(thread, caller.device);
    Ok(Json(PresenceReply { people }))
}
