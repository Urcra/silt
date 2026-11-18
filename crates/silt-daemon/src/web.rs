//! The web UI's server (ADR 0011): the built `web/` app and a JSON API on
//! 127.0.0.1, guarded by a per-start token and a `Host` check. Every API
//! call becomes a socket [`Request`], so the UI and the CLI share one path.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path, Query, Request as HttpRequest, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response as HttpResponse};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::core::Event;
use crate::protocol::{BusEvent, BusSubscription, CommentOn, Request, Response, SearchFilters};
use crate::voice::{self, Voice};

#[derive(Clone)]
struct Web {
    events: mpsc::UnboundedSender<Event>,
    bus: broadcast::Sender<BusEvent>,
    token: String,
    port: u16,
    codes: OneTimeCodes,
    voice: Arc<Voice>,
    /// The CDN files artifacts load through Silt, when the person turned
    /// that on (ADR 0071).
    vendor: Arc<silt_vendor::Vendor>,
    /// What each frame key opens (ADR 0074).
    frames: Arc<Mutex<crate::frames::FrameKeys>>,
    /// Each room socket's way to its browser, by peer (ADR 0081).
    room_peers: Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>,
    /// Each server's one socket for every view's room here.
    room_mux: crate::room_mux::Upstreams,
}

/// Codes that each stand in for the token once, shortly. Shared between the
/// daemon, which hands them out, and the web server, which trades them in.
type OneTimeCodes = Arc<Mutex<HashMap<String, Instant>>>;

/// The most a page's save or added file may send: the largest asset, as base64.
const PAGE_FILE_BODY_BYTES: usize = 28 << 20;

/// How long a one-time code may wait for its browser.
const CODE_TTL: Duration = Duration::from_secs(60);

/// A running web UI.
#[derive(Debug, Clone)]
pub(crate) struct WebInfo {
    pub(crate) port: u16,
    pub(crate) token: String,
    codes: OneTimeCodes,
}

impl WebInfo {
    pub(crate) fn url(&self, project: Option<&std::path::Path>) -> String {
        format!("http://127.0.0.1:{}/{}#token={}", self.port, project_query(project), self.token)
    }

    /// A URL for a browser's command line, which other users can read: it
    /// carries a code the page trades for the token once, within a minute.
    pub(crate) fn one_time_url(&self, project: Option<&std::path::Path>) -> String {
        let code = uuid::Uuid::new_v4().simple().to_string();
        let mut codes = self.codes.lock().expect("lock");
        codes.retain(|_, made| made.elapsed() < CODE_TTL);
        codes.insert(code.clone(), Instant::now());
        format!("http://127.0.0.1:{}/{}#code={code}", self.port, project_query(project))
    }
}

/// `?project=<clone>`, so the UI opens on a new thread in it (ADR 0044).
fn project_query(project: Option<&std::path::Path>) -> String {
    let Some(project) = project else {
        return String::new();
    };
    let encoded: String = project
        .to_string_lossy()
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => char::from(byte).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect();
    format!("?project={encoded}")
}

/// Starts the web server on 127.0.0.1 (`SILT_WEB_PORT`, or any free port).
pub(crate) async fn start(events: mpsc::UnboundedSender<Event>, bus: broadcast::Sender<BusEvent>) -> std::io::Result<WebInfo> {
    let port = std::env::var("SILT_WEB_PORT").ok().and_then(|port| port.parse().ok()).unwrap_or(0);
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let codes = OneTimeCodes::default();
    let web = Web {
        events,
        bus,
        token: token.clone(),
        port,
        codes: codes.clone(),
        voice: Arc::new(Voice::new()),
        vendor: Arc::new(silt_vendor::Vendor::new(&crate::paths::data_dir().join("vendor"))),
        frames: Arc::default(),
        room_peers: Arc::default(),
        room_mux: Arc::default(),
    };
    let api = Router::new()
        .route("/api/events", get(events_socket))
        .route("/api/rooms", get(rooms_socket))
        .route("/api/status", get(status))
        .route("/api/threads", get(threads))
        .route("/api/threads/{id}/history", get(history))
        .route("/api/threads/{id}/show", get(show))
        .route("/api/threads/{id}/turns", get(turns))
        .route("/api/threads/{id}/unwritten", get(unwritten))
        .route("/api/threads/{id}/commits", get(commits))
        .route("/api/graph", get(graph))
        .route("/api/threads/{id}/diff", get(diff))
        .route("/api/threads/{id}/diff/stats", get(diff_stats))
        .route("/api/threads/{id}/diff/turns", get(diff_turns))
        .route("/api/threads/{id}/seen", get(thread_seen).put(mark_thread_seen))
        .route("/api/threads/{id}/comments", get(comments).post(add_comment))
        .route("/api/threads/{id}/comments/sent", post(comments_sent))
        .route("/api/threads/{id}/comments/anchor", post(anchor_lines))
        .route("/api/threads/{id}/comments/place", post(place_anchors))
        .route("/api/threads/{id}/comments/{comment}/resolve", post(resolve_comment))
        .route("/api/threads/{id}/comments/{comment}/replies", post(reply))
        .route("/api/search", get(search))
        .route("/api/threads/{id}/replica", get(replica).post(browser_deltas))
        .route("/api/threads/{id}/git/{oid}", get(git_text))
        .route("/api/threads/{id}/artifacts/{blob}", get(artifact_content))
        .route("/api/threads/{id}/artifacts/{artifact}/frame", post(make_frame))
        .route("/api/threads/{id}/artifacts/{artifact}/grants", get(artifact_grants).put(set_artifact_grant))
        .route("/api/threads/{id}/artifacts/{artifact}/data", post(page_data))
        .route(
            "/api/threads/{id}/artifacts/{artifact}/publish",
            post(publish_artifact).layer(DefaultBodyLimit::max(PAGE_FILE_BODY_BYTES)),
        )
        .route("/api/threads/{id}/viewer", get(page_viewer))
        .route("/api/artifacts", get(artifact_listing))
        .route("/api/threads/{id}/artifacts/{artifact}/thumbnail", get(artifact_thumbnail))
        .route("/api/threads/{id}/artifacts/{artifact}/pin", axum::routing::put(pin_artifact))
        .route("/api/threads/{id}/artifacts/{artifact}/archive", axum::routing::put(archive_artifact))
        .route("/api/threads/{id}/artifacts/{artifact}/files", post(page_files))
        .route("/api/threads/{id}/artifacts/{artifact}/doc", post(doc_data))
        .route("/api/threads/{id}/artifacts/{artifact}/mcp", post(page_mcp))
        .route("/api/page-connectors", get(page_connectors).put(set_page_connectors))
        .route("/api/page-connectors/import", post(import_page_connectors))
        .route("/api/page-connectors/{name}/tools", get(connector_tools))
        .route("/api/page-connectors/{name}/secret", put(set_connector_secret))
        .route("/api/threads/{id}/artifacts/{artifact}/sheet", post(sheet_data))
        .route("/api/threads/{id}/artifacts/{artifact}/sheet.xlsx", get(sheet_xlsx))
        .route(
            "/api/threads/{id}/artifacts/{artifact}/assets",
            post(page_assets).layer(DefaultBodyLimit::max(PAGE_FILE_BODY_BYTES)),
        )
        .route("/api/threads/{id}/artifacts/{artifact}/sample", post(sample))
        .route("/api/threads/{id}/artifacts/{artifact}/versions", post(save_edited_version))
        .route("/api/threads/{id}/artifacts/{artifact}/assets/{asset}", get(page_asset_content))
        .route(
            "/api/threads/{id}/artifacts/{artifact}/pdf",
            post(artifact_pdf).layer(DefaultBodyLimit::max(PAGE_FILE_BODY_BYTES)),
        )
        .route("/api/threads/{id}/artifacts/{artifact}/sample/{ask}/tools/{call}", post(sample_tool_result))
        .route("/api/threads/{id}/artifacts/{artifact}/sample/{ask}/stop", post(stop_sample))
        .route("/api/threads/{id}/live", get(live))
        .route("/api/threads/{id}/presence", post(presence))
        .route("/api/agents", get(agents))
        .route("/api/agents/start", post(agent_start).layer(DefaultBodyLimit::max(PROMPT_BODY_BYTES)))
        .route("/api/agents/{agent}/usage", get(agent_usage))
        .route("/api/clones/branches", get(branches))
        .route("/api/cloud", get(cloud_launches).post(cloud_start))
        .route("/api/threads/{id}/cloud/send", post(cloud_send))
        .route("/api/projects", get(projects).post(add_clone))
        .route("/api/orgs", get(orgs))
        .route("/api/orgs/change", post(org_change))
        .route("/api/me/notifications", get(notification_settings).put(notification_setting))
        .route("/api/devices", get(devices))
        .route("/api/pair", post(pair_phone))
        .route("/api/threads/{id}/links", get(thread_links).post(thread_link))
        .route("/api/threads/{id}/links/{link}/revoke", post(thread_link_revoke))
        .route("/api/devices/{id}", axum::routing::patch(device_change))
        .route("/api/devices/{id}/revoke", post(device_revoke))
        .route("/api/threads/{id}/state", axum::routing::patch(thread_state))
        .route("/api/projects/clone", post(clone_project))
        .route("/api/projects/new", post(new_project))
        .route("/api/found", get(found_folders))
        .route("/api/found/import", post(import_found))
        .route("/api/projects/home", axum::routing::put(project_home))
        .route("/api/chats/settings", get(chat_settings).put(set_chat_settings))
        .route("/api/models", get(models))
        .route("/api/models/pull", post(model_pull))
        .route("/api/models/search", get(model_search))
        .route("/api/models/files", get(model_files))
        .route("/api/models/runtime", post(runtime_pull))
        .route("/api/models/{id}", delete(model_remove))
        .route("/api/threads/{id}/erase", post(thread_erase))
        .route("/api/workspaces", get(workspaces).put(set_workspaces))
        .route("/api/design-systems", get(design_systems))
        .route("/api/design-systems/catalog", get(design_catalog))
        .route("/api/design-systems/system", get(design_system))
        .route("/api/design-systems/asset", get(design_asset))
        .route("/api/design-systems/start", post(design_start))
        .route("/api/design-systems/keep", post(keep_design_system))
        .route("/api/threads/{id}/design-system", get(thread_design))
        .route("/api/artifact-settings", get(artifact_settings).put(set_artifact_settings))
        .route("/api/projects/design-systems", get(project_design_systems).put(set_project_design_system))
        .route("/api/workspaces/assign", post(assign_project))
        .route("/api/threads/{id}/make-project", post(make_project))
        .route("/api/threads/{id}/agent/prompt", post(agent_prompt).layer(DefaultBodyLimit::max(PROMPT_BODY_BYTES)))
        .route("/api/threads/{id}/agent/cancel", post(agent_cancel))
        .route("/api/threads/{id}/agent/tasks/{call}/stop", post(agent_stop_task))
        .route("/api/threads/{id}/agent/answer", post(agent_answer))
        .route("/api/threads/{id}/agent/reply", post(agent_reply))
        .route("/api/threads/{id}/agent/sign-in", post(agent_sign_in))
        .route("/api/threads/{id}/agent/sign-in/code", post(agent_sign_in_code))
        .route("/api/threads/{id}/agent/sign-in/cancel", post(agent_sign_in_cancel))
        .route("/api/threads/{id}/pull-request", post(pull_request).get(pull_request_blocker))
        .route("/api/threads/{id}/agent/stop", post(agent_stop))
        .route("/api/threads/{id}/agent/steer", post(agent_steer))
        .route("/api/threads/{id}/agent/preset", post(agent_preset))
        .route("/api/threads/{id}/agent/mode", post(agent_mode))
        .route("/api/threads/{id}/agent/setting", post(agent_setting))
        .route("/api/threads/{id}/queue", get(queue).post(queue_add))
        .route("/api/threads/{id}/queue/{item}", put(queue_edit).delete(queue_remove))
        .route("/api/threads/{id}/queue/{item}/move", post(queue_move))
        .route("/api/threads/{id}/queue/{item}/send", post(queue_send))
        .route("/api/inbox", get(inbox))
        .route("/api/inbox/read", post(inbox_read))
        .route("/api/approvals", get(approvals))
        .route("/api/approvals/{id}", post(approval_answer))
        .route("/api/grants/{id}", delete(grant_revoke))
        .route("/api/threads/{thread}/folders", delete(folder_revoke))
        .route("/api/threads/{id}/access", get(access))
        .route("/api/threads/{id}/devices/release", post(release_device))
        .route("/api/plugins", get(plugins))
        .route("/api/plugins/{name}", put(plugin_put).delete(plugin_remove))
        .route("/api/mrs", get(merge_requests))
        .route("/api/mrs/{id}", get(merge_request))
        .route("/api/mrs/{id}/approve", post(approve))
        .route("/api/mrs/{id}/decline", post(decline))
        .route("/api/mrs/{id}/request-changes", post(mr_request_changes))
        .route("/api/mrs/{id}/seen", put(mr_seen))
        .route("/api/mrs/{id}/guide", post(mr_guide))
        .route("/api/threads/{id}", delete(thread_purge))
        .route("/api/threads/{id}/fork", post(fork))
        .route("/api/threads/{id}/resume", post(resume))
        .route("/api/threads/{id}/pull", post(pull))
        .route("/api/threads/{id}/revert", post(revert))
        .route("/api/threads/{id}/mrs", post(mr_create))
        .route("/api/threads/{id}/visibility", post(visibility))
        .route("/api/threads/{id}/share", post(share))
        .route("/api/threads/{id}/trash", post(thread_delete))
        .route("/api/threads/{id}/restore", post(thread_restore))
        .route("/api/threads/{id}/files/history", get(file_history))
        .route("/api/threads/{id}/files/at", get(file_at))
        .route("/api/threads/{id}/in-flight", get(in_flight))
        .route("/api/threads/{id}/search", get(thread_search))
        .route("/api/trash", get(trash))
        .route("/api/voice", get(voice_status))
        .route("/api/voice/model", post(voice_model))
        .route("/api/voice/dictate", get(voice_dictate))
        .layer(middleware::from_fn_with_state(web.clone(), authenticate));
    let app = api
        .route("/api/exchange", post(exchange))
        // An artifact's frame has an opaque origin and no token; this serves
        // only pinned files of the CDN allowlist.
        .route("/vendor/{host}/{*path}", get(vendor_file))
        // A frame has no token either: its key is the whole permission, and
        // serves only its own version (ADR 0074).
        .route("/f/{key}/", get(serve_frame))
        .route("/f/{key}/{*path}", get(frame_file))
        .fallback(asset)
        .layer(middleware::from_fn_with_state(web.clone(), local_host_only))
        .with_state(web);
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::warn!(%error, "the web UI stopped");
        }
    });
    Ok(WebInfo { port, token, codes })
}

#[derive(Deserialize)]
struct Exchange {
    code: String,
}

/// Trades a one-time code from `silt open` for the token.
async fn exchange(State(web): State<Web>, Json(body): Json<Exchange>) -> HttpResponse {
    let fresh = web.codes.lock().expect("lock").remove(&body.code).is_some_and(|made| made.elapsed() < CODE_TTL);
    if fresh {
        Json(json!({ "token": web.token })).into_response()
    } else {
        (StatusCode::UNAUTHORIZED, Json(json!({"error": "this link was used already or is too old; run `silt open` again"})))
            .into_response()
    }
}

/// Refuses requests addressed to any other host, so a web page elsewhere
/// can't reach the API through DNS rebinding.
async fn local_host_only(State(web): State<Web>, request: HttpRequest, next: Next) -> HttpResponse {
    let host = request.headers().get(header::HOST).and_then(|value| value.to_str().ok());
    let allowed = [format!("127.0.0.1:{}", web.port), format!("localhost:{}", web.port)];
    if host.is_some_and(|host| allowed.iter().any(|allowed| allowed == host)) {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "unexpected host").into_response()
    }
}

async fn authenticate(State(web): State<Web>, request: HttpRequest, next: Next) -> HttpResponse {
    let header_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    // The browser's WebSocket can't send headers, so the event bus carries
    // the token in the query instead.
    let query_token = request.uri().query().and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("token=")));
    if header_token.or(query_token).is_some_and(|given| same_token(given, &web.token)) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, Json(json!({"error": "open the UI with `silt open`"}))).into_response()
    }
}

/// Compares without stopping at the first difference, so response timing
/// doesn't reveal how much of a guess was right.
fn same_token(given: &str, expected: &str) -> bool {
    let difference = given.bytes().zip(expected.bytes()).fold(0u8, |difference, (a, b)| difference | (a ^ b));
    given.len() == expected.len() && std::hint::black_box(difference) == 0
}

async fn asset(request: HttpRequest) -> HttpResponse {
    let path = request.uri().path().trim_start_matches('/');
    let file = match silt_web::Assets::get(path) {
        Some(file) if !path.is_empty() => file,
        _ => match silt_web::Assets::get("index.html") {
            Some(file) => file,
            None => {
                return (StatusCode::NOT_FOUND, "the web UI was not built; run `pnpm -C web build` and rebuild silt")
                    .into_response();
            }
        },
    };
    let mime = file.metadata.mimetype().to_owned();
    ([(header::CONTENT_TYPE, mime)], Body::from(file.data.into_owned())).into_response()
}

/// Sends a request through the daemon's event loop.
async fn ask(web: &Web, request: Request) -> HttpResponse {
    match answer(web, request).await {
        Ok(data) => Json(data).into_response(),
        Err(failed) => failed.into_response(),
    }
}

/// A request the daemon didn't carry out.
struct Failed {
    status: StatusCode,
    message: String,
}

impl IntoResponse for Failed {
    fn into_response(self) -> HttpResponse {
        let refusal = silt_proto::refusal_of(&self.message);
        (self.status, Json(json!({"error": self.message, "refusal": refusal}))).into_response()
    }
}

async fn answer(web: &Web, request: Request) -> Result<serde_json::Value, Failed> {
    let unavailable = |message: &str| Failed { status: StatusCode::SERVICE_UNAVAILABLE, message: message.to_owned() };
    let (reply, answer) = oneshot::channel();
    if web.events.send(Event::Request(request, reply)).is_err() {
        return Err(unavailable("the daemon is stopping"));
    }
    match answer.await {
        Ok(Response::Ok { data }) => Ok(data),
        Ok(Response::Error { message }) => Err(Failed { status: StatusCode::BAD_REQUEST, message }),
        Err(_) => Err(unavailable("the daemon stopped")),
    }
}

/// Refuses a page's call of `capability` unless the person allowed what the
/// artifact's latest version declares of it. The page holding the frame
/// asks them; this stands behind it, for a document the frame navigated to
/// (ADR 0072).
async fn consented(web: &Web, thread: &str, artifact: &str, capability: &str) -> Result<(), Failed> {
    let grants = answer(web, Request::ArtifactGrants { thread: thread.to_owned(), artifact: artifact.to_owned() }).await?;
    if grants.get(capability).and_then(serde_json::Value::as_str) == Some("granted") {
        return Ok(());
    }
    Err(Failed {
        status: StatusCode::FORBIDDEN,
        message: format!("not_granted: the person hasn't allowed this page {capability}"),
    })
}

/// Asks the request `make` builds for this machine's clone of `thread`, as
/// requests the CLI makes from a working directory need one.
async fn ask_in_clone(web: &Web, thread: &str, make: impl FnOnce(PathBuf) -> Request) -> HttpResponse {
    let clone = match answer(web, Request::CloneOf { thread: thread.to_owned() }).await {
        Ok(clone) => clone,
        Err(failed) => return failed.into_response(),
    };
    let Ok(clone) = serde_json::from_value::<PathBuf>(clone) else {
        let message = "unexpected answer from the daemon".to_owned();
        return Failed { status: StatusCode::INTERNAL_SERVER_ERROR, message }.into_response();
    };
    ask(web, make(clone)).await
}

/// The browser's event bus (ADR 0021).
async fn events_socket(State(web): State<Web>, upgrade: WebSocketUpgrade) -> HttpResponse {
    let bus = web.bus.subscribe();
    upgrade.on_upgrade(move |socket| serve_events(socket, bus))
}

async fn rooms_socket(State(web): State<Web>, upgrade: WebSocketUpgrade) -> HttpResponse {
    upgrade.max_message_size(64 * 1024).on_upgrade(move |socket| serve_room(socket, web))
}

#[derive(Deserialize)]
struct Join {
    thread: String,
    artifact: String,
    #[serde(default)]
    name: String,
}

/// Hands each delivery to its peer's socket.
fn deliver(web: &Web, deliveries: &serde_json::Value) {
    let peers = web.room_peers.lock().expect("lock");
    for pair in deliveries.as_array().into_iter().flatten() {
        let (Some(peer), Some(heard)) = (pair.get(0).and_then(serde_json::Value::as_str), pair.get(1)) else { continue };
        // A view that stops reading loses what it can't keep up with,
        // rather than growing without end.
        if let Some(sender) = peers.get(peer) {
            let _ = sender.try_send(heard.to_string());
        }
    }
}

/// One view in one room (ADR 0081): it joins with its first message, then
/// what it sends goes through the hub, and what others say comes back.
async fn serve_room(mut socket: WebSocket, web: Web) {
    let Some(Ok(Message::Text(first))) = socket.recv().await else { return };
    let Ok(join) = serde_json::from_str::<Join>(&first) else { return };
    // A thread that syncs with a server has its room there, spanning every
    // machine viewing the page and the hosted app.
    if let Ok(route) = answer(&web, Request::RoomRoute { thread: join.thread.clone() }).await
        && let (Some(url), Some(token)) = (route["url"].as_str(), route["token"].as_str())
    {
        // Every view here shares one socket to the server; one of its own where the server has none.
        match crate::room_mux::join(&web.room_mux, url, token, &first).await {
            Some(channel) => relay_on(socket, channel).await,
            None => relay_room(socket, url, token, first.to_string()).await,
        }
        return;
    }
    let key = silt_proto::rooms::RoomKey { thread: join.thread, artifact: join.artifact, name: join.name };
    let peer = uuid::Uuid::new_v4().simple().to_string();
    let (sender, mut heard) = mpsc::channel(silt_proto::rooms::QUEUED);
    web.room_peers.lock().expect("lock").insert(peer.clone(), sender);
    let mut bus = web.bus.subscribe();
    match answer(&web, Request::RoomJoin { key: key.clone(), peer: peer.clone() }).await {
        Ok(deliveries) => {
            let _ = socket.send(Message::Text(json!({"event": "joined", "peer": peer}).to_string().into())).await;
            deliver(&web, &deliveries);
        }
        Err(failed) => {
            let _ = socket.send(Message::Text(json!({"event": "refused", "reason": failed.message}).to_string().into())).await;
            web.room_peers.lock().expect("lock").remove(&peer);
            return;
        }
    }
    loop {
        tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => {
                    let Ok(sent) = serde_json::from_str::<silt_proto::rooms::Sent>(&text) else { continue };
                    if let Ok(deliveries) = answer(&web, Request::RoomSend { key: key.clone(), peer: peer.clone(), sent }).await {
                        deliver(&web, &deliveries);
                    }
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => break,
            },
            out = heard.recv() => match out {
                Some(text) => if socket.send(Message::Text(text.into())).await.is_err() { break },
                None => break,
            },
            event = bus.recv() => {
                // What the thread's agent said, for this view if it's addressed here.
                if let Ok(BusEvent::Room { deliveries }) = event {
                    let mine: Vec<&serde_json::Value> = deliveries.as_array().into_iter().flatten().filter(|pair| pair.get(0).and_then(serde_json::Value::as_str) == Some(peer.as_str())).collect();
                    for pair in mine {
                        if let Some(heard) = pair.get(1) && socket.send(Message::Text(heard.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }
    web.room_peers.lock().expect("lock").remove(&peer);
    if let Ok(deliveries) = answer(&web, Request::RoomLeave { key, peer }).await {
        deliver(&web, &deliveries);
    }
}

/// Relays one view's room on the server's shared socket, both ways until
/// either side closes.
async fn relay_on(mut socket: WebSocket, mut channel: crate::room_mux::Channel) {
    loop {
        tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => if !channel.send(&text).await { break },
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => break,
            },
            heard = channel.heard.recv() => match heard {
                Some(text) => if socket.send(Message::Text(text.into())).await.is_err() { break },
                None => break,
            },
        }
    }
}

/// Relays one view's room to its server's `/v1/rooms`, as this device,
/// both ways until either side closes.
async fn relay_room(mut socket: WebSocket, server: &str, token: &str, join: String) {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest as _};
    let refuse = |reason: &str| Message::Text(json!({"event": "refused", "reason": reason}).to_string().into());
    let address = format!("{}/v1/rooms", server.trim_end_matches('/').replacen("http", "ws", 1));
    let Ok(mut request) = address.into_client_request() else {
        let _ = socket.send(refuse("unavailable")).await;
        return;
    };
    let headers = request.headers_mut();
    if let (Ok(auth), Ok(version)) = (format!("Bearer {token}").parse(), silt_proto::PROTOCOL.to_string().parse()) {
        headers.insert("authorization", auth);
        headers.insert(silt_proto::PROTOCOL_HEADER, version);
    }
    let Ok((mut upstream, _)) = tokio_tungstenite::connect_async(request).await else {
        let _ = socket.send(refuse("unavailable: the server can't be reached")).await;
        return;
    };
    if upstream.send(tungstenite::Message::Text(join.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => {
                    if upstream.send(tungstenite::Message::Text(text.to_string().into())).await.is_err() { break }
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => break,
            },
            heard = upstream.next() => match heard {
                Some(Ok(tungstenite::Message::Text(text))) => {
                    if socket.send(Message::Text(text.to_string().into())).await.is_err() { break }
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => break,
            },
        }
    }
    let _ = upstream.close(None).await;
}

/// Sends the socket the events of the topics it subscribed to, until it
/// closes.
async fn serve_events(mut socket: WebSocket, mut bus: broadcast::Receiver<BusEvent>) {
    let mut topics: HashSet<String> = HashSet::new();
    loop {
        let event = tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(change) = serde_json::from_str::<BusSubscription>(&text) {
                        topics.extend(change.subscribe);
                        topics.retain(|topic| !change.unsubscribe.contains(topic));
                    }
                    continue;
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => return,
            },
            event = bus.recv() => match event {
                Ok(event) if event.topic().is_none_or(|topic| topics.contains(&topic)) => event,
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => BusEvent::Lagged,
                Err(broadcast::error::RecvError::Closed) => return,
            },
        };
        let Ok(text) = serde_json::to_string(&event) else {
            continue;
        };
        if socket.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }
}

async fn status(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Status).await
}

// The voice routes don't become socket `Request`s: streamed audio has no CLI
// caller (ADR 0061).
async fn voice_status(State(web): State<Web>) -> HttpResponse {
    Json(web.voice.status()).into_response()
}

async fn voice_model(State(web): State<Web>) -> HttpResponse {
    Json(web.voice.start_download()).into_response()
}

async fn voice_dictate(State(web): State<Web>, upgrade: WebSocketUpgrade) -> HttpResponse {
    upgrade.max_message_size(voice::MOST_FRAME_BYTES).on_upgrade(move |socket| voice::dictate(socket, web.voice))
}

async fn threads(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Threads).await
}

/// A page of a thread's history; all of it unless asked for less.
#[derive(Deserialize)]
struct HistoryQuery {
    offset: Option<usize>,
    limit: Option<usize>,
}

async fn history(State(web): State<Web>, Path(thread): Path<String>, Query(page): Query<HistoryQuery>) -> HttpResponse {
    let request = Request::HistoryRead { thread, offset: page.offset.unwrap_or(0), limit: page.limit.unwrap_or(usize::MAX) };
    ask(&web, request).await
}

async fn show(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Show { thread }).await
}

async fn turns(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Turns { thread }).await
}

async fn unwritten(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Unwritten { thread }).await
}

async fn commits(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Commits { thread }).await
}

async fn graph(State(web): State<Web>, Query(query): Query<silt_proto::view::GraphQuery>) -> HttpResponse {
    ask(&web, Request::Graph { query }).await
}

#[derive(Deserialize)]
struct DiffQuery {
    turn: Option<usize>,
    /// With `turn`, the last of a range of turns.
    to_turn: Option<usize>,
    base: Option<String>,
    /// A page of the changed files, as the changes panel shows them.
    offset: Option<usize>,
    limit: Option<usize>,
    /// One file, as a diff too large to read whole is read.
    path: Option<String>,
    /// `hide` leaves out what git itself rewrote (ADR 0069).
    #[serde(default)]
    git: Option<String>,
    /// `hide` leaves out other agents' edits (ADR 0089).
    #[serde(default)]
    others: Option<String>,
}

impl DiffQuery {
    fn hide(&self) -> silt_log::Hide {
        silt_proto::view::GitQuery { git: self.git.clone(), others: self.others.clone() }.hide()
    }
}

async fn diff(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<DiffQuery>) -> HttpResponse {
    let files = query.limit.map(|limit| crate::protocol::FilePage { offset: query.offset.unwrap_or(0), limit });
    let silt_log::Hide { git: hide_git, others: hide_others } = query.hide();
    let base = query.base.filter(|base| !base.is_empty());
    let path = query.path.filter(|path| !path.is_empty());
    ask(&web, Request::Diff { thread, turn: query.turn, to_turn: query.to_turn, base, files, path, hide_git, hide_others }).await
}

async fn diff_stats(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<DiffQuery>) -> HttpResponse {
    let silt_log::Hide { git: hide_git, others: hide_others } = query.hide();
    let base = query.base.filter(|base| !base.is_empty());
    ask(&web, Request::DiffStats { thread, turn: query.turn, to_turn: query.to_turn, base, hide_git, hide_others }).await
}

async fn thread_seen(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<DiffQuery>) -> HttpResponse {
    let silt_log::Hide { git: hide_git, others: hide_others } = query.hide();
    let base = query.base.filter(|base| !base.is_empty());
    ask(&web, Request::ThreadSeen { thread, turn: query.turn, to_turn: query.to_turn, base, hide_git, hide_others }).await
}

async fn mark_thread_seen(
    State(web): State<Web>,
    Path(thread): Path<String>,
    Json(mark): Json<silt_proto::view::MarkSeen>,
) -> HttpResponse {
    ask(&web, Request::MarkThreadSeen { thread, mark }).await
}

async fn diff_turns(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<DiffQuery>) -> HttpResponse {
    let files = query.limit.map(|limit| crate::protocol::FilePage { offset: query.offset.unwrap_or(0), limit });
    let silt_log::Hide { git: hide_git, others: hide_others } = query.hide();
    let base = query.base.filter(|base| !base.is_empty());
    let path = query.path.filter(|path| !path.is_empty());
    ask(&web, Request::DiffTurns { thread, turn: query.turn, to_turn: query.to_turn, base, files, path, hide_git, hide_others })
        .await
}

/// The web UI has no clone of its own; comments that need one say so.
fn no_clone() -> std::path::PathBuf {
    std::path::PathBuf::from("/")
}

#[derive(Deserialize)]
struct CommentsQuery {
    base: Option<String>,
    /// `hide` places them as a diff without git's rewrites shows the lines.
    #[serde(default)]
    git: Option<String>,
    /// `hide`, and without other agents' edits.
    #[serde(default)]
    others: Option<String>,
}

async fn comments(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<CommentsQuery>) -> HttpResponse {
    let silt_log::Hide { git: hide_git, others: hide_others } =
        silt_proto::view::GitQuery { git: query.git, others: query.others }.hide();
    let base = query.base.filter(|base| !base.is_empty());
    ask(&web, Request::Comments { thread, base, hide_git, hide_others, cwd: no_clone() }).await
}

#[derive(Deserialize)]
struct SentComments {
    comments: Vec<String>,
}

async fn comments_sent(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<SentComments>) -> HttpResponse {
    ask(&web, Request::CommentsSent { thread, comments: body.comments, cwd: no_clone() }).await
}

#[derive(Deserialize)]
struct NewComment {
    on: CommentOn,
    text: String,
}

async fn add_comment(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<NewComment>) -> HttpResponse {
    let request = Request::Comment { thread, on: body.on, text: body.text, cwd: no_clone() };
    ask(&web, request).await
}

async fn anchor_lines(
    State(web): State<Web>,
    Path(thread): Path<String>,
    Json(on): Json<silt_proto::view::CommentOn>,
) -> HttpResponse {
    ask(&web, Request::AnchorLines { thread, on, cwd: no_clone() }).await
}

async fn place_anchors(
    State(web): State<Web>,
    Path(thread): Path<String>,
    Json(place): Json<silt_proto::view::PlaceAnchors>,
) -> HttpResponse {
    ask(&web, Request::PlaceAnchors { thread, place, cwd: no_clone() }).await
}

async fn resolve_comment(State(web): State<Web>, Path((thread, comment)): Path<(String, String)>) -> HttpResponse {
    let request = Request::ResolveComment { thread, comment, cwd: no_clone() };
    ask(&web, request).await
}

#[derive(Deserialize)]
struct NewReply {
    text: String,
}

async fn reply(
    State(web): State<Web>,
    Path((thread, comment)): Path<(String, String)>,
    Json(body): Json<NewReply>,
) -> HttpResponse {
    ask(&web, Request::Reply { thread, comment, text: body.text, cwd: no_clone() }).await
}

async fn search(State(web): State<Web>, Query(query): Query<HashMap<String, String>>) -> HttpResponse {
    let text = |name: &str| query.get(name).filter(|value| !value.is_empty()).cloned();
    let number = |name: &str| text(name).and_then(|value| value.parse().ok());
    let request = Request::Search {
        query: text("q").unwrap_or_default(),
        limit: 50,
        filters: SearchFilters {
            repo: text("repo"),
            author: text("author"),
            status: text("status").and_then(|status| status.parse().ok()),
            branch: text("branch"),
            file: text("file"),
            since: number("since"),
            until: number("until"),
        },
    };
    ask(&web, request).await
}

#[derive(Deserialize)]
struct AfterQuery {
    #[serde(default)]
    after: usize,
}

async fn replica(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<AfterQuery>) -> HttpResponse {
    ask(&web, Request::Replica { thread, after: query.after }).await
}

#[derive(Deserialize)]
struct BrowserDeltas {
    deltas: String,
}

async fn browser_deltas(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<BrowserDeltas>) -> HttpResponse {
    ask(&web, Request::BrowserDeltas { thread, deltas: body.deltas }).await
}

async fn git_text(State(web): State<Web>, Path((thread, oid)): Path<(String, String)>) -> HttpResponse {
    ask(&web, Request::GitText { thread, oid }).await
}

async fn artifact_content(State(web): State<Web>, Path((thread, blob)): Path<(String, String)>) -> HttpResponse {
    ask(&web, Request::ArtifactContent { thread, blob }).await
}

async fn artifact_settings(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::ArtifactSettings).await
}

async fn set_artifact_settings(State(web): State<Web>, Json(settings): Json<crate::protocol::ArtifactSettings>) -> HttpResponse {
    ask(&web, Request::SetArtifactSettings { settings }).await
}

#[derive(Deserialize)]
struct FrameAsk {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    theme: Option<String>,
    /// The origin of the page that will hold the frame.
    #[serde(default)]
    parent: Option<String>,
    /// What the holding page kept for this viewer: their storage, and the
    /// last version's state (ADR 0072).
    #[serde(default)]
    boot: Option<serde_json::Value>,
}

/// A read or write of an artifact's page data, as its page asks through the
/// bridge (ADR 0072).
async fn page_data(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(request): Json<silt_proto::page::PageRequest>,
) -> HttpResponse {
    ask(&web, Request::PageData { thread, artifact, asked: request, as_level: None }).await
}

#[derive(Deserialize)]
struct Publish {
    html: String,
    #[serde(default)]
    base: Option<u32>,
}

/// A version a page saves itself as, through the `artifact` capability.
async fn publish_artifact(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(publish): Json<Publish>,
) -> HttpResponse {
    ask(&web, Request::PublishArtifact { thread, artifact, html: publish.html, base: publish.base }).await
}

#[derive(Deserialize)]
struct SampleAsk {
    #[serde(default)]
    ask: Option<String>,
    input: serde_json::Value,
    #[serde(default)]
    images: Vec<silt_proto::artifact::SampleImage>,
    #[serde(default)]
    tools: Vec<silt_proto::artifact::SampleTool>,
    #[serde(default)]
    model: silt_proto::artifact::SampleModel,
}

/// A page's question for this machine's agent, its answer streamed on the
/// bus as it comes (ADR 0072).
async fn sample(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(asked): Json<SampleAsk>,
) -> HttpResponse {
    if let Err(refused) = consented(&web, &thread, &artifact, "sample").await {
        return refused.into_response();
    }
    let request = Request::Sample {
        thread,
        artifact: Some(artifact),
        ask: asked.ask,
        input: asked.input,
        images: asked.images,
        tools: asked.tools,
        model: asked.model,
    };
    ask(&web, request).await
}

/// One of the files people added to a page, for a view Silt draws itself,
/// such as a whiteboard's images (ADR 0081).
async fn page_asset_content(
    State(web): State<Web>,
    Path((thread, artifact, asset)): Path<(String, String, String)>,
) -> HttpResponse {
    ask(&web, Request::PageAsset { thread, artifact, asset }).await
}

#[derive(Deserialize)]
struct DocPage {
    html: String,
}

/// A doc or a deck printed to a PDF by this machine's headless browser
/// (ADRs 0076, 0077).
async fn artifact_pdf(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(page): Json<DocPage>,
) -> HttpResponse {
    ask(&web, Request::ArtifactPdf { thread, artifact, html: page.html }).await
}

#[derive(Deserialize)]
struct SaveVersion {
    #[serde(default)]
    note: Option<String>,
}

/// Keeps a doc, a sheet or a whiteboard as it stands as its next version
/// (ADR 0077).
async fn save_edited_version(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(save): Json<SaveVersion>,
) -> HttpResponse {
    ask(&web, Request::SaveEditedVersion { thread, artifact, note: save.note }).await
}

/// A page stops asking its question, which ends its agent's run.
async fn stop_sample(State(web): State<Web>, Path((thread, artifact, ask_name)): Path<(String, String, String)>) -> HttpResponse {
    ask(&web, Request::StopSample { thread, artifact, ask: ask_name }).await
}

/// A page's answer to the agent's call of one of the tools it gave a
/// question (ADR 0072).
async fn sample_tool_result(
    State(web): State<Web>,
    Path((thread, artifact, ask_name, call)): Path<(String, String, String, u32)>,
    Json(result): Json<crate::protocol::SampleToolResult>,
) -> HttpResponse {
    ask(&web, Request::SampleToolResult { thread, artifact, ask: ask_name, call, result }).await
}

/// Adds, lists or deletes the files people added to a page (ADR 0074).
async fn page_assets(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(asked): Json<silt_proto::page::AssetRequest>,
) -> HttpResponse {
    ask(&web, Request::PageAssets { thread, artifact, asked }).await
}

/// A page's call of one of the viewer's connectors' tools (ADR 0080).
async fn page_mcp(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(asked): Json<crate::connectors::McpAsk>,
) -> HttpResponse {
    if let Err(refused) = consented(&web, &thread, &artifact, "mcp").await {
        return refused.into_response();
    }
    ask(&web, Request::PageMcp { thread, artifact, asked }).await
}

async fn page_connectors(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::PageConnectors).await
}

async fn set_page_connectors(
    State(web): State<Web>,
    Json(connectors): Json<Vec<crate::connectors::PageConnector>>,
) -> HttpResponse {
    ask(&web, Request::SetPageConnectors { connectors }).await
}

async fn import_page_connectors(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::ImportPageConnectors).await
}

async fn connector_tools(State(web): State<Web>, Path(name): Path<String>) -> HttpResponse {
    ask(&web, Request::ConnectorTools { name }).await
}

#[derive(serde::Deserialize)]
struct ConnectorSecret {
    header: String,
    #[serde(default)]
    value: Option<String>,
}

async fn set_connector_secret(
    State(web): State<Web>,
    Path(name): Path<String>,
    Json(secret): Json<ConnectorSecret>,
) -> HttpResponse {
    ask(&web, Request::SetConnectorSecret { name, header: secret.header, value: secret.value }).await
}

/// A doc's text, or edits of it (ADR 0077).
async fn doc_data(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(asked): Json<silt_proto::edited::DocRequest>,
) -> HttpResponse {
    ask(&web, Request::DocData { thread, artifact, asked }).await
}

/// A sheet's cells and values, or changes to it (ADR 0077).
async fn sheet_data(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(asked): Json<silt_proto::edited::SheetRequest>,
) -> HttpResponse {
    ask(&web, Request::SheetData { thread, artifact, asked }).await
}

/// A sheet as an `.xlsx` workbook, formulas included (ADR 0077).
async fn sheet_xlsx(State(web): State<Web>, Path((thread, artifact)): Path<(String, String)>) -> HttpResponse {
    let mut sheets = Vec::new();
    let (mut sheet, mut count) = (0, 1);
    while sheet < count {
        let asked = silt_proto::edited::SheetRequest::Read { sheet, range: None };
        let request = Request::SheetData { thread: thread.clone(), artifact: artifact.clone(), asked };
        let view = match answer(&web, request).await {
            Ok(view) => view,
            Err(failed) => return failed.into_response(),
        };
        let Ok(view) = serde_json::from_value::<silt_proto::edited::SheetView>(view) else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        count = view.sheets;
        let name = view.names.get(usize::from(sheet)).cloned().unwrap_or_else(|| format!("Sheet{}", sheet + 1));
        sheets.push((name, view));
        sheet += 1;
    }
    match silt_proto::edited::xlsx_book(&sheets) {
        Ok(bytes) => xlsx_response(bytes),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(crate) fn xlsx_response(bytes: Vec<u8>) -> HttpResponse {
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    );
    headers.insert(header::CONTENT_DISPOSITION, header::HeaderValue::from_static("attachment; filename=\"sheet.xlsx\""));
    response
}

/// The thread's files, read by a page that declares `files` (ADR 0078).
async fn page_files(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(asked): Json<silt_proto::page::FilesRequest>,
) -> HttpResponse {
    if let Err(refused) = consented(&web, &thread, &artifact, "files").await {
        return refused.into_response();
    }
    ask(&web, Request::PageFiles { thread, artifact, asked }).await
}

/// Every artifact in the threads here, for the Artifacts page (ADR 0075).
async fn artifact_listing(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::ArtifactListing).await
}

async fn artifact_thumbnail(State(web): State<Web>, Path((thread, artifact)): Path<(String, String)>) -> HttpResponse {
    ask(&web, Request::ArtifactThumbnail { thread, artifact }).await
}

#[derive(Deserialize)]
struct Pinned {
    pinned: bool,
}

async fn pin_artifact(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(pin): Json<Pinned>,
) -> HttpResponse {
    ask(&web, Request::PinArtifact { thread, artifact, pinned: pin.pinned }).await
}

#[derive(Deserialize)]
struct Archived {
    archived: bool,
}

async fn archive_artifact(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(archive): Json<Archived>,
) -> HttpResponse {
    ask(&web, Request::ArchiveArtifact { thread, artifact, archived: archive.archived }).await
}

async fn page_viewer(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::PageViewer { thread }).await
}

async fn artifact_grants(State(web): State<Web>, Path((thread, artifact)): Path<(String, String)>) -> HttpResponse {
    ask(&web, Request::ArtifactGrants { thread, artifact }).await
}

#[derive(Deserialize)]
struct GrantChange {
    capability: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    version: Option<u32>,
}

async fn set_artifact_grant(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(change): Json<GrantChange>,
) -> HttpResponse {
    ask(
        &web,
        Request::SetArtifactGrant {
            thread,
            artifact,
            capability: change.capability,
            state: change.state,
            version: change.version,
        },
    )
    .await
}

/// A key that opens one version of an HTML artifact in a frame (ADR 0074).
async fn make_frame(
    State(web): State<Web>,
    Path((thread, artifact)): Path<(String, String)>,
    Json(ask): Json<FrameAsk>,
) -> HttpResponse {
    let request = Request::ArtifactVersion { thread: thread.clone(), id: artifact.clone(), version: ask.version };
    let read = match answer(&web, request).await {
        Ok(read) => read,
        Err(failed) => return failed.into_response(),
    };
    if read["kind"] != "html" {
        return Failed { status: StatusCode::BAD_REQUEST, message: "only an HTML artifact opens in a frame".into() }
            .into_response();
    }
    let grant = crate::frames::FrameGrant {
        thread,
        artifact,
        version: ask.version,
        theme: ask.theme.as_deref().and_then(silt_design::frame::Theme::parse),
        parent: ask.parent.filter(|origin| silt_design::frame::is_origin(origin)),
        boot: crate::frames::boot_of(ask.boot),
    };
    let key = crate::frames::new_key();
    web.frames.lock().expect("lock").insert(key.clone(), grant, "");
    let expires_ms = silt_proto::frames::FRAME_TTL.as_millis() as u64;
    Json(json!({ "path": format!("/f/{key}/"), "expires_in_ms": expires_ms })).into_response()
}

/// The page a frame key opens, composed as every surface composes it, with
/// its policy as a header that also makes its origin opaque.
async fn serve_frame(State(web): State<Web>, Path(key): Path<String>, request: HttpRequest) -> HttpResponse {
    let not_found = || (StatusCode::NOT_FOUND, "this frame's key expired or never existed").into_response();
    // The viewer's storage goes into the page once, and isn't kept with the key after (ADR 0072).
    let Some(grant) = web.frames.lock().expect("lock").open(&key, |grant| grant.boot = None) else {
        return not_found();
    };
    let request_version =
        Request::ArtifactVersion { thread: grant.thread.clone(), id: grant.artifact.clone(), version: grant.version };
    let Ok(read) = answer(&web, request_version).await else {
        return not_found();
    };
    let Ok(read) = serde_json::from_value::<crate::protocol::ArtifactRead>(read) else {
        return not_found();
    };
    use base64::Engine as _;
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&read.data) else {
        return not_found();
    };
    let proxied = answer(&web, Request::ArtifactSettings).await.is_ok_and(|settings| settings["proxy"] == true);
    let host = request.headers().get(header::HOST).and_then(|value| value.to_str().ok()).unwrap_or("127.0.0.1");
    let vendor = proxied.then(|| format!("http://{host}/vendor/"));
    let parts = silt_design::frame::FrameParts {
        style: read.style.as_deref(),
        theme: grant.theme,
        probe: true,
        vendor: vendor.as_deref(),
        parent: grant.parent.as_deref(),
        policy_meta: false,
        boot: grant.boot.as_deref(),
        frozen: None,
    };
    let page = silt_design::frame::compose(&String::from_utf8_lossy(&bytes), &parts);
    let files = format!("http://{host}/f/{key}/");
    framed_response(page, &silt_design::frame::policy_with_files(vendor.as_deref(), &files))
}

/// A file beside a frame's version, or an asset someone added to its page
/// (ADR 0074), fetched by the frame from its own address.
async fn frame_file(State(web): State<Web>, Path((key, path)): Path<(String, String)>) -> HttpResponse {
    let not_found = || (StatusCode::NOT_FOUND, "no such file").into_response();
    let Some(grant) = web.frames.lock().expect("lock").get(&key) else {
        return not_found();
    };
    let request = match path.strip_prefix("_assets/") {
        Some(asset) if silt_proto::page::asset_id_ok(asset) => {
            Request::PageAsset { thread: grant.thread.clone(), artifact: grant.artifact.clone(), asset: asset.to_owned() }
        }
        Some(_) => return not_found(),
        None if silt_proto::artifact::file_path_ok(&path) => {
            Request::ArtifactFile { thread: grant.thread.clone(), id: grant.artifact.clone(), version: grant.version, path }
        }
        None => return not_found(),
    };
    let Ok(answer) = answer(&web, request).await else {
        return not_found();
    };
    let Ok(content) = serde_json::from_value::<silt_proto::view::ArtifactContent>(answer) else {
        return not_found();
    };
    use base64::Engine as _;
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&content.data) else {
        return not_found();
    };
    file_response(bytes, &content.media_type)
}

/// A frame's file: readable by the frame's opaque origin, never sniffed,
/// and run as nothing if opened as a page of its own.
fn file_response(bytes: Vec<u8>, media_type: &str) -> HttpResponse {
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = header::HeaderValue::from_str(media_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, header::HeaderValue::from_static("*"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, header::HeaderValue::from_static("nosniff"));
    headers.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    headers.insert(header::CONTENT_SECURITY_POLICY, header::HeaderValue::from_static(silt_design::frame::FILE_POLICY));
    headers.insert("cross-origin-resource-policy", header::HeaderValue::from_static("cross-origin"));
    response
}

fn framed_response(page: String, policy: &str) -> HttpResponse {
    let mut response = (StatusCode::OK, page).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, header::HeaderValue::from_static("text/html; charset=utf-8"));
    if let Ok(value) = header::HeaderValue::from_str(&format!("sandbox allow-scripts; {policy}")) {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, header::HeaderValue::from_static("nosniff"));
    headers.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    headers.insert(header::REFERRER_POLICY, header::HeaderValue::from_static("no-referrer"));
    headers.insert("cross-origin-resource-policy", header::HeaderValue::from_static("same-site"));
    response
}

/// A CDN file an artifact's frame loads through Silt (ADR 0071).
async fn vendor_file(State(web): State<Web>, Path((host, path)): Path<(String, String)>, request: HttpRequest) -> HttpResponse {
    let query = request.uri().query().map(str::to_owned);
    match web.vendor.get(&host, &format!("/{path}"), query.as_deref()).await {
        Ok(fetched) => vendor_response(fetched),
        Err(silt_vendor::VendorError::Refused(why)) => (StatusCode::FORBIDDEN, why).into_response(),
        Err(error) => (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
    }
}

fn vendor_response(fetched: silt_vendor::Fetched) -> HttpResponse {
    let mut response = (StatusCode::OK, fetched.bytes).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = header::HeaderValue::from_str(&fetched.content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, header::HeaderValue::from_static("*"));
    headers.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("public, max-age=31536000, immutable"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, header::HeaderValue::from_static("nosniff"));
    headers.insert("cross-origin-resource-policy", header::HeaderValue::from_static("cross-origin"));
    response
}

#[derive(Deserialize, Default)]
struct PresenceBody {
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    draft: String,
    #[serde(default)]
    leaving: bool,
}

async fn presence(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<PresenceBody>) -> HttpResponse {
    let request = Request::Presence { thread, file: body.file, line: body.line, draft: body.draft, leaving: body.leaving };
    ask(&web, request).await
}

/// The thread's live state now; what changes later comes on the event bus.
async fn live(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Live { thread }).await
}

async fn agents(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::AcpAgents).await
}

#[derive(Deserialize)]
struct UsageQuery {
    #[serde(default)]
    thread: Option<String>,
}

async fn agent_usage(State(web): State<Web>, Path(agent): Path<String>, Query(query): Query<UsageQuery>) -> HttpResponse {
    ask(&web, Request::AgentUsage { agent, thread: query.thread }).await
}

#[derive(Deserialize)]
struct AgentStart {
    #[serde(default)]
    thread: Option<String>,
    #[serde(default)]
    clone: Option<std::path::PathBuf>,
    agent: String,
    #[serde(default, flatten)]
    options: crate::protocol::StartOptions,
}

#[derive(Deserialize)]
struct Erase {
    #[serde(default)]
    confirm: bool,
}

async fn thread_erase(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<Erase>) -> HttpResponse {
    ask(&web, Request::ThreadErase { thread, confirm: body.confirm }).await
}

async fn models(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Models).await
}

#[derive(Deserialize)]
struct ModelSource {
    source: String,
}

async fn model_pull(State(web): State<Web>, Json(body): Json<ModelSource>) -> HttpResponse {
    ask(&web, Request::ModelPull { source: body.source }).await
}

#[derive(Deserialize)]
struct ModelQuery {
    #[serde(default)]
    q: String,
}

async fn model_search(State(web): State<Web>, Query(query): Query<ModelQuery>) -> HttpResponse {
    ask(&web, Request::ModelSearch { query: query.q }).await
}

#[derive(Deserialize)]
struct RepoQuery {
    repo: String,
}

async fn model_files(State(web): State<Web>, Query(query): Query<RepoQuery>) -> HttpResponse {
    ask(&web, Request::ModelFiles { repo: query.repo }).await
}

async fn runtime_pull(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::RuntimePull).await
}

async fn model_remove(State(web): State<Web>, Path(id): Path<String>) -> HttpResponse {
    ask(&web, Request::ModelRemove { model: id }).await
}

async fn chat_settings(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::ChatSettings).await
}

async fn set_chat_settings(State(web): State<Web>, Json(settings): Json<crate::protocol::ChatSettings>) -> HttpResponse {
    ask(&web, Request::SetChatSettings { settings }).await
}

async fn workspaces(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Workspaces).await
}

#[derive(Deserialize)]
struct DesignSystemsQuery {
    #[serde(default)]
    clone: Option<PathBuf>,
}

async fn design_systems(State(web): State<Web>, Query(query): Query<DesignSystemsQuery>) -> HttpResponse {
    ask(&web, Request::DesignSystems { clone: query.clone }).await
}

async fn design_catalog(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::DesignCatalog).await
}

#[derive(Deserialize)]
struct DesignSystemQuery {
    source: crate::protocol::DesignSource,
    name: String,
    #[serde(default)]
    clone: Option<PathBuf>,
    #[serde(default)]
    thread: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

async fn design_system(State(web): State<Web>, Query(query): Query<DesignSystemQuery>) -> HttpResponse {
    ask(&web, Request::DesignSystem { source: query.source, name: query.name, clone: query.clone, thread: query.thread }).await
}

async fn design_asset(State(web): State<Web>, Query(query): Query<DesignSystemQuery>) -> HttpResponse {
    let path = query.path.unwrap_or_default();
    ask(&web, Request::DesignAsset { source: query.source, name: query.name, clone: query.clone, thread: query.thread, path })
        .await
}

async fn design_start(State(web): State<Web>, Json(start): Json<crate::protocol::DesignStart>) -> HttpResponse {
    ask(&web, Request::DesignStart(start)).await
}

async fn thread_design(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::ThreadDesignSystem { thread }).await
}

#[derive(Deserialize)]
struct KeepDesignSystem {
    thread: String,
    name: String,
}

async fn keep_design_system(State(web): State<Web>, Json(body): Json<KeepDesignSystem>) -> HttpResponse {
    ask(&web, Request::KeepDesignSystem { thread: body.thread, name: body.name }).await
}

async fn project_design_systems(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::ProjectDesignSystems).await
}

#[derive(Deserialize)]
struct ProjectDesignSystem {
    repo: String,
    #[serde(default)]
    design_system: Option<String>,
}

async fn set_project_design_system(State(web): State<Web>, Json(body): Json<ProjectDesignSystem>) -> HttpResponse {
    ask(&web, Request::SetProjectDesignSystem { repo: body.repo, design_system: body.design_system }).await
}

async fn set_workspaces(State(web): State<Web>, Json(workspaces): Json<silt_workspace::Workspaces>) -> HttpResponse {
    ask(&web, Request::SetWorkspaces { workspaces }).await
}

#[derive(Deserialize)]
struct AssignProject {
    repo: String,
    workspace: Option<String>,
}

async fn assign_project(State(web): State<Web>, Json(body): Json<AssignProject>) -> HttpResponse {
    ask(&web, Request::AssignProject { repo: body.repo, workspace: body.workspace }).await
}

async fn make_project(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::MakeProject { thread }).await
}

async fn agent_start(State(web): State<Web>, Json(body): Json<AgentStart>) -> HttpResponse {
    let request = Request::AcpStart { thread: body.thread, clone: body.clone, agent: body.agent, options: body.options };
    ask(&web, request).await
}

#[derive(Deserialize)]
struct CloudStart {
    clone: std::path::PathBuf,
    task: String,
}

async fn cloud_start(State(web): State<Web>, Json(body): Json<CloudStart>) -> HttpResponse {
    ask(&web, Request::CloudStart { clone: body.clone, task: body.task }).await
}

async fn cloud_launches(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::CloudLaunches).await
}

#[derive(Deserialize)]
struct CloudSend {
    text: String,
}

async fn cloud_send(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<CloudSend>) -> HttpResponse {
    ask(&web, Request::CloudSend { thread, text: body.text }).await
}

#[derive(Deserialize)]
struct CloneQuery {
    path: std::path::PathBuf,
}

async fn branches(State(web): State<Web>, Query(query): Query<CloneQuery>) -> HttpResponse {
    ask(&web, Request::Branches { clone: query.path }).await
}

async fn thread_state(
    State(web): State<Web>,
    Path(thread): Path<String>,
    Json(change): Json<crate::protocol::StateChange>,
) -> HttpResponse {
    ask(&web, Request::ThreadState { thread, change }).await
}

/// Which server a request that belongs to no thread asks, on a machine
/// signed in to several (ADR 0032); the default when left out.
#[derive(Deserialize, Default)]
struct OnServer {
    #[serde(default)]
    server: Option<String>,
}

impl OnServer {
    fn ask(self, request: Request) -> Request {
        match self.server {
            Some(server) => Request::OnServer { server, asking: Box::new(request) },
            None => request,
        }
    }
}

async fn orgs(State(web): State<Web>, Query(on): Query<OnServer>) -> HttpResponse {
    ask(&web, on.ask(Request::Orgs)).await
}

async fn org_change(
    State(web): State<Web>,
    Query(on): Query<OnServer>,
    Json(change): Json<silt_proto::OrgChange>,
) -> HttpResponse {
    ask(&web, on.ask(Request::OrgChange { change })).await
}

async fn projects(State(web): State<Web>, Query(on): Query<OnServer>) -> HttpResponse {
    ask(&web, on.ask(Request::Projects)).await
}

async fn notification_settings(State(web): State<Web>, Query(on): Query<OnServer>) -> HttpResponse {
    ask(&web, on.ask(Request::NotificationSettings)).await
}

async fn notification_setting(
    State(web): State<Web>,
    Query(on): Query<OnServer>,
    Json(setting): Json<silt_proto::KindSetting>,
) -> HttpResponse {
    ask(&web, on.ask(Request::NotificationSetting { setting })).await
}

async fn thread_links(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::ThreadLinks { thread }).await
}

async fn thread_link(
    State(web): State<Web>,
    Path(thread): Path<String>,
    Json(link): Json<crate::protocol::NewLink>,
) -> HttpResponse {
    ask(&web, Request::ThreadLink { thread, link }).await
}

async fn thread_link_revoke(State(web): State<Web>, Path((thread, link)): Path<(String, i64)>) -> HttpResponse {
    ask(&web, Request::ThreadLinkRevoke { thread, link }).await
}

async fn pair_phone(State(web): State<Web>, Query(on): Query<OnServer>) -> HttpResponse {
    ask(&web, on.ask(Request::PairPhone)).await
}

async fn devices(State(web): State<Web>, Query(on): Query<OnServer>) -> HttpResponse {
    ask(&web, on.ask(Request::Devices)).await
}

async fn device_change(
    State(web): State<Web>,
    Path(id): Path<String>,
    Query(on): Query<OnServer>,
    Json(change): Json<crate::protocol::DeviceChange>,
) -> HttpResponse {
    ask(&web, on.ask(Request::DeviceChange { id, change })).await
}

async fn device_revoke(State(web): State<Web>, Path(id): Path<String>, Query(on): Query<OnServer>) -> HttpResponse {
    ask(&web, on.ask(Request::DeviceRevoke { id })).await
}

async fn add_clone(State(web): State<Web>, Json(body): Json<CloneQuery>) -> HttpResponse {
    ask(&web, Request::AddClone { path: body.path }).await
}

async fn new_project(State(web): State<Web>, Json(body): Json<CloneQuery>) -> HttpResponse {
    ask(&web, Request::NewProject { path: body.path }).await
}

async fn found_folders(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::FoundFolders).await
}

#[derive(Deserialize)]
struct ImportFound {
    #[serde(default)]
    paths: Vec<PathBuf>,
}

async fn import_found(State(web): State<Web>, Json(body): Json<ImportFound>) -> HttpResponse {
    ask(&web, Request::ImportFound { paths: body.paths }).await
}

#[derive(Deserialize)]
struct CloneProject {
    url: String,
    path: PathBuf,
}

async fn clone_project(State(web): State<Web>, Json(body): Json<CloneProject>) -> HttpResponse {
    ask(&web, Request::CloneProject { url: body.url, path: body.path }).await
}

#[derive(Deserialize)]
struct ProjectHome {
    repo: String,
    home: String,
}

async fn project_home(State(web): State<Web>, Json(body): Json<ProjectHome>) -> HttpResponse {
    ask(&web, Request::ProjectHome { repo: body.repo, home: body.home }).await
}

#[derive(Deserialize)]
struct AgentPrompt {
    text: String,
    #[serde(default)]
    plan: bool,
    #[serde(default)]
    attachments: Vec<crate::protocol::PromptAttachment>,
}

/// A prompt's body: its files, in base64, as large as `acp::decode` takes.
const PROMPT_BODY_BYTES: usize = 36 << 20;

async fn agent_prompt(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<AgentPrompt>) -> HttpResponse {
    ask(&web, Request::AcpPrompt { thread, text: body.text, plan: body.plan, attachments: body.attachments }).await
}

async fn agent_cancel(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::AcpCancel { thread }).await
}

async fn agent_stop_task(State(web): State<Web>, Path((thread, call)): Path<(String, String)>) -> HttpResponse {
    ask(&web, Request::AcpStopTask { thread, call }).await
}

#[derive(Deserialize)]
struct AgentAnswer {
    id: u64,
    #[serde(default)]
    option: Option<String>,
}

async fn agent_answer(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<AgentAnswer>) -> HttpResponse {
    let request = Request::AcpAnswer { thread, id: body.id, option: body.option };
    ask(&web, request).await
}

#[derive(Deserialize)]
struct AgentReply {
    id: u64,
    /// Each field's chosen values; none declines the question.
    #[serde(default)]
    answers: Option<std::collections::BTreeMap<String, Vec<String>>>,
}

async fn agent_reply(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<AgentReply>) -> HttpResponse {
    let answers = body.answers.map(|answers| answers.into_iter().collect());
    ask(&web, Request::AcpReply { thread, id: body.id, answers }).await
}

#[derive(Deserialize)]
struct AgentSignIn {
    method: String,
}

async fn agent_sign_in(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<AgentSignIn>) -> HttpResponse {
    ask(&web, Request::AcpSignIn { thread, method: body.method }).await
}

#[derive(Deserialize)]
struct AgentSignInCode {
    code: String,
}

async fn agent_sign_in_code(
    State(web): State<Web>,
    Path(thread): Path<String>,
    Json(body): Json<AgentSignInCode>,
) -> HttpResponse {
    ask(&web, Request::AcpSignInCode { thread, code: body.code }).await
}

async fn agent_sign_in_cancel(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::AcpSignInCancel { thread }).await
}

async fn pull_request(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::OpenPullRequest { thread }).await
}

async fn pull_request_blocker(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::PullRequestBlocker { thread }).await
}

async fn agent_steer(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<AgentPrompt>) -> HttpResponse {
    ask(&web, Request::AcpSteer { thread, text: body.text }).await
}

#[derive(Deserialize)]
struct PresetBody {
    preset: crate::protocol::Preset,
}

async fn agent_preset(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<PresetBody>) -> HttpResponse {
    ask(&web, Request::AgentPreset { thread, preset: body.preset }).await
}

#[derive(Deserialize)]
struct ModeBody {
    mode: String,
}

async fn agent_mode(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<ModeBody>) -> HttpResponse {
    ask(&web, Request::AcpMode { thread, mode: body.mode }).await
}

#[derive(Deserialize)]
struct SettingBody {
    id: String,
    value: String,
}

async fn agent_setting(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<SettingBody>) -> HttpResponse {
    ask(&web, Request::AcpSetting { thread, id: body.id, value: body.value }).await
}

async fn queue(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Queue { thread }).await
}

async fn queue_add(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<AgentPrompt>) -> HttpResponse {
    ask(&web, Request::QueueAdd { thread, text: body.text }).await
}

async fn queue_edit(
    State(web): State<Web>,
    Path((thread, id)): Path<(String, i64)>,
    Json(body): Json<AgentPrompt>,
) -> HttpResponse {
    ask(&web, Request::QueueEdit { thread, id, text: body.text }).await
}

async fn queue_remove(State(web): State<Web>, Path((thread, id)): Path<(String, i64)>) -> HttpResponse {
    ask(&web, Request::QueueRemove { thread, id }).await
}

#[derive(Deserialize)]
struct QueueMove {
    to: usize,
}

async fn queue_move(
    State(web): State<Web>,
    Path((thread, id)): Path<(String, i64)>,
    Json(body): Json<QueueMove>,
) -> HttpResponse {
    ask(&web, Request::QueueMove { thread, id, to: body.to }).await
}

async fn queue_send(State(web): State<Web>, Path((thread, id)): Path<(String, i64)>) -> HttpResponse {
    ask(&web, Request::QueueSend { thread, id }).await
}

async fn agent_stop(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::AcpStop { thread }).await
}

#[derive(Deserialize)]
struct InboxQuery {
    #[serde(default)]
    all: bool,
}

async fn inbox(State(web): State<Web>, Query(query): Query<InboxQuery>, Query(on): Query<OnServer>) -> HttpResponse {
    ask(&web, on.ask(Request::Inbox { all: query.all })).await
}

async fn inbox_read(State(web): State<Web>, Query(on): Query<OnServer>, Json(read): Json<silt_proto::InboxRead>) -> HttpResponse {
    ask(&web, on.ask(Request::InboxRead { read })).await
}

#[derive(Deserialize)]
struct ApprovalsQuery {
    thread: Option<String>,
    #[serde(default)]
    all: bool,
}

async fn approvals(State(web): State<Web>, Query(query): Query<ApprovalsQuery>) -> HttpResponse {
    ask(&web, Request::Approvals { thread: query.thread, all: query.all }).await
}

#[derive(Deserialize)]
struct ApprovalBody {
    approve: bool,
    hash: String,
    #[serde(default)]
    grant: Option<crate::plugins::GrantScope>,
}

async fn approval_answer(State(web): State<Web>, Path(id): Path<u64>, Json(body): Json<ApprovalBody>) -> HttpResponse {
    ask(&web, Request::ApprovalAnswer { id, approve: body.approve, hash: body.hash, grant: body.grant }).await
}

async fn grant_revoke(State(web): State<Web>, Path(id): Path<u64>) -> HttpResponse {
    ask(&web, Request::GrantRevoke { id }).await
}

#[derive(Deserialize)]
struct FolderBody {
    path: String,
}

async fn folder_revoke(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<FolderBody>) -> HttpResponse {
    ask(&web, Request::FolderRevoke { thread, path: body.path }).await
}

#[derive(Deserialize)]
struct ReleaseBody {
    device: String,
}

async fn release_device(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<ReleaseBody>) -> HttpResponse {
    ask(&web, Request::ReleaseDevice { thread, device: body.device }).await
}

async fn access(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Access { thread }).await
}

async fn plugins(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Plugins).await
}

async fn plugin_put(
    State(web): State<Web>,
    Path(name): Path<String>,
    Json(plugin): Json<crate::plugins::Plugin>,
) -> HttpResponse {
    if plugin.name != name {
        return Failed { status: StatusCode::BAD_REQUEST, message: "the plugin's name isn't the one in its path".into() }
            .into_response();
    }
    ask(&web, Request::PluginPut { plugin: Box::new(plugin) }).await
}

async fn plugin_remove(State(web): State<Web>, Path(name): Path<String>) -> HttpResponse {
    ask(&web, Request::PluginRemove { name }).await
}

async fn merge_requests(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::MrList { thread: None }).await
}

async fn merge_request(
    State(web): State<Web>,
    Path(mr): Path<String>,
    Query(query): Query<silt_proto::view::GitQuery>,
) -> HttpResponse {
    let silt_log::Hide { git: hide_git, others: hide_others } = query.hide();
    ask(&web, Request::MrShow { mr, hide_git, hide_others }).await
}

async fn approve(State(web): State<Web>, Path(mr): Path<String>) -> HttpResponse {
    ask(&web, Request::MrApprove { mr }).await
}

#[derive(Deserialize, Default)]
struct Decline {
    reason: Option<String>,
}

async fn decline(State(web): State<Web>, Path(mr): Path<String>, body: Option<Json<Decline>>) -> HttpResponse {
    let reason = body.map(|Json(body)| body).unwrap_or_default().reason;
    ask(&web, Request::MrDecline { mr, reason }).await
}

#[derive(Deserialize)]
struct ChangesBody {
    reason: String,
}

async fn mr_request_changes(State(web): State<Web>, Path(mr): Path<String>, Json(body): Json<ChangesBody>) -> HttpResponse {
    ask(&web, Request::MrRequestChanges { mr, reason: body.reason }).await
}

#[derive(Deserialize)]
struct SeenBody {
    path: String,
    seen: bool,
}

async fn mr_seen(State(web): State<Web>, Path(mr): Path<String>, Json(body): Json<SeenBody>) -> HttpResponse {
    ask(&web, Request::MrSeen { mr, path: body.path, seen: body.seen }).await
}

#[derive(Deserialize, Default)]
struct GuideBody {
    #[serde(default)]
    text: Option<String>,
}

async fn mr_guide(State(web): State<Web>, Path(mr): Path<String>, body: Option<Json<GuideBody>>) -> HttpResponse {
    let GuideBody { text } = body.map(|Json(body)| body).unwrap_or_default();
    ask(&web, Request::MrGuide { mr, text }).await
}

#[derive(Deserialize, Default)]
struct Continue {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    turn: Option<usize>,
}

/// A fork in a checkout of its own, to start an agent in from the UI.
async fn fork(State(web): State<Web>, Path(thread): Path<String>, body: Option<Json<Continue>>) -> HttpResponse {
    let body = body.map(|Json(body)| body).unwrap_or_default();
    let request = |cwd| Request::Fork {
        thread: thread.clone(),
        cwd,
        here: false,
        agent: body.agent.clone(),
        private: body.private,
        turn: body.turn,
    };
    ask_in_clone(&web, &thread, request).await
}

async fn resume(State(web): State<Web>, Path(thread): Path<String>, body: Option<Json<Continue>>) -> HttpResponse {
    let agent = body.and_then(|Json(body)| body.agent);
    ask_in_clone(&web, &thread, |cwd| Request::Resume { thread: thread.clone(), cwd, here: false, agent }).await
}

async fn pull(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask_in_clone(&web, &thread, |cwd| Request::Pull { thread: Some(thread.clone()), cwd }).await
}

#[derive(Deserialize)]
struct RevertBody {
    turn: usize,
}

async fn revert(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<RevertBody>) -> HttpResponse {
    ask(&web, Request::Revert { thread: Some(thread), cwd: PathBuf::new(), turn: body.turn }).await
}

#[derive(Deserialize, Default)]
struct NewMr {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

async fn mr_create(State(web): State<Web>, Path(thread): Path<String>, body: Option<Json<NewMr>>) -> HttpResponse {
    let NewMr { title, summary } = body.map(|Json(body)| body).unwrap_or_default();
    ask_in_clone(&web, &thread, |cwd| Request::MrCreate { thread: Some(thread.clone()), cwd, title, summary }).await
}

#[derive(Deserialize)]
struct NewVisibility {
    visibility: silt_proto::Visibility,
}

async fn visibility(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<NewVisibility>) -> HttpResponse {
    let request = |cwd| Request::SetVisibility { cwd, thread: Some(thread.clone()), visibility: body.visibility };
    ask_in_clone(&web, &thread, request).await
}

#[derive(Deserialize)]
struct Share {
    user: String,
    #[serde(default)]
    remove: bool,
}

async fn share(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<Share>) -> HttpResponse {
    ask(&web, Request::ThreadShare { thread, user: body.user, remove: body.remove }).await
}

async fn trash(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Trash).await
}

async fn thread_delete(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::ThreadDelete { thread }).await
}

async fn thread_restore(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::ThreadRestore { thread }).await
}

async fn thread_purge(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::ThreadPurge { thread }).await
}

#[derive(Deserialize)]
struct PathQuery {
    path: String,
}

async fn file_history(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<PathQuery>) -> HttpResponse {
    ask(&web, Request::FileHistory { thread, path: query.path }).await
}

async fn file_at(
    State(web): State<Web>,
    Path(thread): Path<String>,
    Query(query): Query<silt_proto::view::FileQuery>,
) -> HttpResponse {
    ask(&web, Request::FileAt { thread, query }).await
}

/// Other threads of the thread's repository that changed the path.
async fn in_flight(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<PathQuery>) -> HttpResponse {
    let caller = Some(thread.clone());
    ask_in_clone(&web, &thread, |cwd| Request::InFlight { cwd, path: query.path, thread: caller }).await
}

#[derive(Deserialize)]
struct ThreadSearch {
    q: String,
    #[serde(default)]
    limit: Option<u32>,
}

async fn thread_search(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<ThreadSearch>) -> HttpResponse {
    ask(&web, Request::HistorySearch { thread, query: query.q, limit: query.limit.unwrap_or(50) }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_one_time_code_gives_the_token_once() {
        let (events, _inbox) = mpsc::unbounded_channel();
        let codes = OneTimeCodes::default();
        let info = WebInfo { port: 1, token: "secret".into(), codes: codes.clone() };
        let bus = broadcast::channel(1).0;
        let web = Web {
            events,
            bus,
            token: info.token.clone(),
            port: 1,
            codes,
            voice: Arc::new(Voice::new()),
            vendor: Arc::new(silt_vendor::Vendor::new(std::path::Path::new("vendor"))),
            frames: Arc::default(),
            room_peers: Arc::default(),
            room_mux: Arc::default(),
        };
        let url = info.one_time_url(None);
        assert!(!url.contains("secret"), "{url}");
        let code = url.split("code=").nth(1).expect("a code").to_owned();
        let trade = || exchange(State(web.clone()), Json(Exchange { code: code.clone() }));
        assert_eq!(trade().await.status(), StatusCode::OK);
        assert_eq!(trade().await.status(), StatusCode::UNAUTHORIZED, "a code works once");
    }

    #[test]
    fn an_address_names_the_project_it_opens_on() {
        assert_eq!(project_query(None), "");
        assert_eq!(project_query(Some(std::path::Path::new("/home/me/my code/app"))), "?project=/home/me/my%20code/app");
        assert_eq!(project_query(Some(std::path::Path::new("/a&b=c#d"))), "?project=/a%26b%3Dc%23d");
    }

    #[test]
    fn tokens_match_only_exactly() {
        assert!(same_token("abc123", "abc123"));
        assert!(!same_token("abc124", "abc123"));
        assert!(!same_token("abc12", "abc123"), "a prefix is not the token");
        assert!(!same_token("abc1234", "abc123"));
        assert!(!same_token("", "abc123"));
    }
}
