//! One socket per server for every view's room on this machine (ADR 0081):
//! each view's frames go up tagged with a channel, and what the server says
//! comes back to the view on that channel. A server without the shared
//! socket gets one per view, as before.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest as _};

/// The rooms one server's shared socket carries, and the way up it.
#[derive(Clone)]
pub(crate) struct Upstream {
    up: mpsc::Sender<String>,
    views: Arc<Mutex<HashMap<u32, mpsc::Sender<String>>>>,
    next: Arc<std::sync::atomic::AtomicU32>,
}

/// Each server's shared socket, by its address.
pub(crate) type Upstreams = Arc<Mutex<HashMap<String, Upstream>>>;

/// The most rooms one shared socket carries, as the server takes them.
const ROOMS: usize = silt_proto::rooms::MUX_ROOMS;

/// One view's place on a shared socket: what it sends goes up on its
/// channel, and what's heard on it arrives on `heard`.
pub(crate) struct Channel {
    upstream: Upstream,
    ch: u32,
    pub(crate) heard: mpsc::Receiver<String>,
}

impl Channel {
    pub(crate) async fn send(&self, sent: &str) -> bool {
        let Ok(sent) = serde_json::from_str::<serde_json::Value>(sent) else { return true };
        self.upstream.up.send(json!({"ch": self.ch, "sent": sent}).to_string()).await.is_ok()
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.upstream.views.lock().expect("lock").remove(&self.ch);
        let _ = self.upstream.up.try_send(json!({"ch": self.ch, "leave": true}).to_string());
    }
}

/// Joins a view to its room through the server's shared socket, opening it
/// when there's none; `None` when the server has no shared socket or can't
/// be reached, so the view relays on a socket of its own.
pub(crate) async fn join(upstreams: &Upstreams, server: &str, token: &str, join: &str) -> Option<Channel> {
    let Ok(join) = serde_json::from_str::<serde_json::Value>(join) else { return None };
    let existing = upstreams.lock().expect("lock").get(server).cloned();
    let upstream = match existing {
        Some(upstream) if !upstream.up.is_closed() => upstream,
        _ => {
            let upstream = connect(upstreams.clone(), server, token).await?;
            upstreams.lock().expect("lock").insert(server.to_owned(), upstream.clone());
            upstream
        }
    };
    let (sender, heard) = mpsc::channel(silt_proto::rooms::QUEUED);
    let ch = {
        let mut views = upstream.views.lock().expect("lock");
        if views.len() >= ROOMS {
            return None;
        }
        let ch = upstream.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        views.insert(ch, sender);
        ch
    };
    let channel = Channel { upstream, ch, heard };
    channel.upstream.up.send(json!({"ch": ch, "join": join}).to_string()).await.ok()?;
    Some(channel)
}

async fn connect(upstreams: Upstreams, server: &str, token: &str) -> Option<Upstream> {
    let address = format!("{}/v1/rooms/mux", server.trim_end_matches('/').replacen("http", "ws", 1));
    let mut request = address.into_client_request().ok()?;
    let headers = request.headers_mut();
    headers.insert("authorization", format!("Bearer {token}").parse().ok()?);
    headers.insert(silt_proto::PROTOCOL_HEADER, silt_proto::PROTOCOL.to_string().parse().ok()?);
    let (socket, _) = tokio_tungstenite::connect_async(request).await.ok()?;
    let (mut writer, mut reader) = socket.split();
    let (up, mut going_up) = mpsc::channel::<String>(silt_proto::rooms::QUEUED * 4);
    let upstream = Upstream { up, views: Arc::default(), next: Arc::default() };
    let views = upstream.views.clone();
    let server = server.to_owned();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                said = going_up.recv() => match said {
                    Some(text) => if writer.send(tungstenite::Message::Text(text.into())).await.is_err() { break },
                    None => break,
                },
                heard = reader.next() => match heard {
                    Some(Ok(tungstenite::Message::Text(text))) => {
                        let Ok(framed) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                        let ch = framed["ch"].as_u64().and_then(|ch| u32::try_from(ch).ok());
                        let view = ch.and_then(|ch| views.lock().expect("lock").get(&ch).cloned());
                        // A view that stops reading loses what it can't keep up with.
                        if let Some(view) = view {
                            let _ = view.try_send(framed["heard"].to_string());
                        }
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(_)) | None => break,
                },
            }
        }
        // The socket closed: every view on it hears so, and the next view opens another.
        upstreams.lock().expect("lock").remove(&server);
        let gone = json!({"event": "refused", "reason": "unavailable: the server can't be reached"}).to_string();
        for (_, view) in views.lock().expect("lock").drain() {
            let _ = view.try_send(gone.clone());
        }
    });
    Some(upstream)
}
