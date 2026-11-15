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

/// The hub, and each socket's way to its browser.
#[derive(Default)]
pub(crate) struct Rooms {
    hub: Hub,
    peers: HashMap<String, mpsc::UnboundedSender<String>>,
}

impl Rooms {
    fn deliver(&self, deliveries: Deliveries) {
        for (peer, heard) in deliveries {
            if let Some(sender) = self.peers.get(&peer) {
                let _ = sender.send(serde_json::to_string(&heard).unwrap_or_default());
            }
        }
    }
}

pub(crate) fn routes() -> Router<Server> {
    Router::new().route("/app/api/rooms", get(socket))
}

async fn socket(State(server): State<Server>, Extension(browser): Extension<Browser>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.max_message_size(64 * 1024).on_upgrade(move |socket| serve(socket, server, browser))
}

#[derive(Deserialize)]
struct Join {
    thread: String,
    artifact: String,
    #[serde(default)]
    name: String,
}

async fn serve(mut socket: WebSocket, server: Server, browser: Browser) {
    let Some(Ok(Message::Text(first))) = socket.recv().await else { return };
    let Ok(join) = serde_json::from_str::<Join>(&first) else { return };
    let refuse = |reason: &str| Message::Text(json!({"event": "refused", "reason": reason}).to_string().into());
    let Ok(artifact) = join.artifact.parse::<silt_types::ArtifactId>() else {
        let _ = socket.send(refuse("not an artifact")).await;
        return;
    };
    let Ok(page) = crate::browse::page_of(&server, browser.user, &join.thread, artifact).await else {
        let _ = socket.send(refuse("not_found")).await;
        return;
    };
    let Some(config) = page.declared.get("room").cloned() else {
        let _ = socket.send(refuse("this page doesn't declare room")).await;
        return;
    };
    let key = RoomKey { thread: page.thread.to_string(), artifact: join.artifact, name: join.name };
    let peer = uuid::Uuid::new_v4().simple().to_string();
    let (sender, mut heard) = mpsc::unbounded_channel();
    {
        let mut rooms = server.shared.page_rooms.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        rooms.peers.insert(peer.clone(), sender);
        let info = PeerInfo { peer: peer.clone(), user: page.me.clone(), kind: "person".into() };
        let deliveries = rooms.hub.join(key.clone(), info, page.level, silt_proto::rooms::topic_levels(&config), crate::now_ms());
        rooms.deliver(deliveries);
    }
    if socket.send(Message::Text(json!({"event": "joined", "peer": peer}).to_string().into())).await.is_ok() {
        loop {
            tokio::select! {
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
