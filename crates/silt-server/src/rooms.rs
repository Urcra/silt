//! Rooms in the hosted app (ADR 0081): a socket per view, joined only by
//! someone who can read the thread, relayed through the shared hub. Nothing
//! is kept; a room goes when its last peer leaves.

use std::collections::HashMap;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, State};
use axum::response::Response;
use axum::routing::get;
use serde::Deserialize;
use serde_json::json;
use silt_proto::rooms::{Deliveries, Hub, PeerInfo, RoomKey, Sent};
use tokio::sync::mpsc;

use crate::Server;
use crate::app::Browser;

/// How often an open socket checks that its person can still read the thread.
const RECHECK: std::time::Duration = std::time::Duration::from_secs(10);

/// The hub, and each socket's way to its browser.
#[derive(Default)]
pub(crate) struct Rooms {
    hub: Hub,
    peers: HashMap<String, mpsc::Sender<String>>,
}

impl Rooms {
    fn deliver(&self, deliveries: Deliveries) {
        for (peer, heard) in deliveries {
            // A view that stops reading loses what it can't keep up with,
            // rather than growing the server's memory.
            if let Some(sender) = self.peers.get(&peer) {
                let _ = sender.try_send(serde_json::to_string(&heard).unwrap_or_default());
            }
        }
    }
}

pub(crate) fn routes() -> Router<Server> {
    Router::new().route("/app/api/rooms", get(socket))
}

async fn socket(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    headers: axum::http::HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    use axum::response::IntoResponse as _;
    // A browser sends no custom header on a WebSocket's handshake, so the
    // cross-site check the API makes is made on its origin here.
    if !crate::pages::same_origin(&server, &headers) {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    upgrade.max_message_size(64 * 1024).on_upgrade(move |socket| serve(socket, server, browser.user))
}

/// A view on one of the person's machines, whose daemon relays it here, so
/// a room spans every machine viewing the page (ADR 0081).
pub(crate) async fn device_socket(
    State(server): State<Server>,
    Extension(caller): Extension<crate::Caller>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let user = caller.user;
    upgrade.max_message_size(64 * 1024).on_upgrade(move |socket| serve(socket, server, user))
}

/// What the thread's agent says to a page's room, from the device it runs
/// on: it joins once as the agent, and hears what the room kept.
#[derive(Deserialize)]
pub(crate) struct AgentSays {
    thread: String,
    artifact: String,
    #[serde(default)]
    emit: Option<(String, serde_json::Value)>,
}

/// The thread's agent speaks in, or listens to, a page's room here, for a
/// thread whose views meet in this server's rooms (ADR 0081).
pub(crate) async fn agent(
    State(server): State<Server>,
    Extension(caller): Extension<crate::Caller>,
    axum::Json(says): axum::Json<AgentSays>,
) -> Result<axum::Json<serde_json::Value>, crate::ApiError> {
    use silt_proto::rooms::Heard;
    let artifact: silt_types::ArtifactId = says.artifact.parse().map_err(|_| crate::bad_request("not an artifact id"))?;
    let page = crate::browse::page_of(&server, caller.user, &says.thread, artifact).await?;
    let config = page.declared.get("room").cloned().ok_or_else(|| crate::bad_request("this page doesn't declare room"))?;
    let key = RoomKey { thread: page.thread.to_string(), artifact: says.artifact, name: String::new() };
    let peer = format!("agent-{}", page.thread.short());
    let now = crate::now_ms();
    let mut rooms = server.shared.page_rooms.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !rooms.hub.present(&key, now).iter().any(|present| present.info.peer == peer) {
        let info = PeerInfo { peer: peer.clone(), user: page.me.clone(), kind: "agent".into() };
        let deliveries = rooms.hub.join(key.clone(), info, page.level, silt_proto::rooms::topic_levels(&config), now);
        rooms.deliver(deliveries);
    }
    if let Some((topic, data)) = says.emit {
        let sent = rooms.hub.sent(&key, &peer, Sent::Emit { topic, data }, now);
        if let Some((_, Heard::Refused { reason })) = sent.iter().find(|(to, _)| *to == peer) {
            return Err(crate::bad_request(format!("the room refused it: {reason}")));
        }
        rooms.deliver(sent);
    }
    let kept: Vec<serde_json::Value> =
        rooms.hub.kept(&key).into_iter().map(|(topic, data, from)| json!({"topic": topic, "data": data, "from": from})).collect();
    let peers = rooms.hub.present(&key, now);
    Ok(axum::Json(json!({ "messages": kept, "peers": peers })))
}

#[derive(Deserialize)]
struct Join {
    thread: String,
    artifact: String,
    #[serde(default)]
    name: String,
}

async fn serve(mut socket: WebSocket, server: Server, user: crate::UserId) {
    let Some(Ok(Message::Text(first))) = socket.recv().await else { return };
    let Ok(join) = serde_json::from_str::<Join>(&first) else { return };
    let refuse = |reason: &str| Message::Text(json!({"event": "refused", "reason": reason}).to_string().into());
    let Ok(artifact) = join.artifact.parse::<silt_types::ArtifactId>() else {
        let _ = socket.send(refuse("not an artifact")).await;
        return;
    };
    let Ok(page) = crate::browse::page_of(&server, user, &join.thread, artifact).await else {
        let _ = socket.send(refuse("not_found")).await;
        return;
    };
    let Some(config) = page.declared.get("room").cloned() else {
        let _ = socket.send(refuse("this page doesn't declare room")).await;
        return;
    };
    let key = RoomKey { thread: page.thread.to_string(), artifact: join.artifact, name: join.name };
    let peer = uuid::Uuid::new_v4().simple().to_string();
    let (sender, mut heard) = mpsc::channel(silt_proto::rooms::QUEUED);
    {
        let mut rooms = server.shared.page_rooms.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        rooms.peers.insert(peer.clone(), sender);
        let info = PeerInfo { peer: peer.clone(), user: page.me.clone(), kind: "person".into() };
        let deliveries = rooms.hub.join(key.clone(), info, page.level, silt_proto::rooms::topic_levels(&config), crate::now_ms());
        rooms.deliver(deliveries);
    }
    if socket.send(Message::Text(json!({"event": "joined", "peer": peer}).to_string().into())).await.is_ok() {
        // Someone who stops being able to read the thread stops hearing its room.
        let mut recheck = tokio::time::interval(RECHECK);
        recheck.tick().await;
        loop {
            tokio::select! {
                _ = recheck.tick() => {
                    if crate::browse::page_of(&server, user, &join.thread, artifact).await.is_err() {
                        let _ = socket.send(Message::Text(json!({"event": "refused", "reason": "not_found"}).to_string().into())).await;
                        break;
                    }
                }
                message = socket.recv() => match message {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(sent) = serde_json::from_str::<Sent>(&text) else { continue };
                        let mut rooms = server.shared.page_rooms.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        let deliveries = rooms.hub.sent(&key, &peer, sent, crate::now_ms());
                        rooms.deliver(deliveries);
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(_)) | None => break,
                },
                out = heard.recv() => match out {
                    Some(text) => if socket.send(Message::Text(text.into())).await.is_err() { break },
                    None => break,
                },
            }
        }
    }
    let mut rooms = server.shared.page_rooms.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    rooms.peers.remove(&peer);
    let deliveries = rooms.hub.leave(&key, &peer, crate::now_ms());
    rooms.deliver(deliveries);
}
