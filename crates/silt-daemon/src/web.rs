//! The web UI's server (ADR 0011): the built `web/` app and a JSON API on
//! 127.0.0.1, guarded by a per-start token and a `Host` check. Every API
//! call becomes a socket [`Request`], so the UI and the CLI share one path.

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::{Path, Query, Request as HttpRequest, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response as HttpResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use crate::core::Event;
use crate::protocol::{CommentOn, Request, Response, SearchFilters};

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Assets;

#[derive(Clone)]
struct Web {
    events: mpsc::UnboundedSender<Event>,
    token: String,
    port: u16,
}

/// A running web UI.
#[derive(Debug, Clone)]
pub(crate) struct WebInfo {
    pub(crate) port: u16,
    pub(crate) token: String,
}

impl WebInfo {
    pub(crate) fn url(&self) -> String {
        format!("http://127.0.0.1:{}/#token={}", self.port, self.token)
    }
}

/// Starts the web server on 127.0.0.1 (`SILT_WEB_PORT`, or any free port).
pub(crate) async fn start(events: mpsc::UnboundedSender<Event>) -> std::io::Result<WebInfo> {
    let port = std::env::var("SILT_WEB_PORT").ok().and_then(|port| port.parse().ok()).unwrap_or(0);
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let web = Web {
        events,
        token: token.clone(),
        port,
    };
    let api = Router::new()
        .route("/api/status", get(status))
        .route("/api/threads", get(threads))
        .route("/api/threads/{id}/history", get(history))
        .route("/api/threads/{id}/show", get(show))
        .route("/api/threads/{id}/turns", get(turns))
        .route("/api/threads/{id}/diff", get(diff))
        .route("/api/threads/{id}/comments", get(comments).post(add_comment))
        .route("/api/threads/{id}/comments/{comment}/resolve", post(resolve_comment))
        .route("/api/search", get(search))
        .route("/api/mrs", get(merge_requests))
        .route("/api/mrs/{id}", get(merge_request))
        .route("/api/mrs/{id}/approve", post(approve))
        .route("/api/mrs/{id}/decline", post(decline))
        .layer(middleware::from_fn_with_state(web.clone(), authenticate));
    let app = api
        .fallback(asset)
        .layer(middleware::from_fn_with_state(web.clone(), local_host_only))
        .with_state(web);
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::warn!(%error, "the web UI stopped");
        }
    });
    Ok(WebInfo { port, token })
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
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if token == Some(web.token.as_str()) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, Json(json!({"error": "open the UI with `silt open`"}))).into_response()
    }
}

async fn asset(request: HttpRequest) -> HttpResponse {
    let path = request.uri().path().trim_start_matches('/');
    let file = match Assets::get(path) {
        Some(file) if !path.is_empty() => file,
        _ => match Assets::get("index.html") {
            Some(file) => file,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    "the web UI was not built; run `pnpm -C web build` and rebuild silt",
                )
                    .into_response();
            }
        },
    };
    let mime = file.metadata.mimetype().to_owned();
    ([(header::CONTENT_TYPE, mime)], Body::from(file.data.into_owned())).into_response()
}

/// Sends a request through the daemon's event loop.
async fn ask(web: &Web, request: Request) -> HttpResponse {
    let (reply, answer) = oneshot::channel();
    if web.events.send(Event::Request(request, reply)).is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "the daemon is stopping"}))).into_response();
    }
    match answer.await {
        Ok(Response::Ok { data }) => Json(data).into_response(),
        Ok(Response::Error { message }) => (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "the daemon stopped"}))).into_response(),
    }
}

async fn status(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Status).await
}

async fn threads(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::Threads).await
}

async fn history(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    let request = Request::HistoryRead {
        thread,
        offset: 0,
        limit: usize::MAX,
    };
    ask(&web, request).await
}

async fn show(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Show { thread }).await
}

async fn turns(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Turns { thread }).await
}

#[derive(Deserialize)]
struct DiffQuery {
    turn: Option<usize>,
    base: Option<String>,
}

async fn diff(State(web): State<Web>, Path(thread): Path<String>, Query(query): Query<DiffQuery>) -> HttpResponse {
    let request = Request::Diff {
        thread,
        turn: query.turn,
        base: query.base.filter(|base| !base.is_empty()),
    };
    ask(&web, request).await
}

/// The web UI has no clone of its own; comments that need one say so.
fn no_clone() -> std::path::PathBuf {
    std::path::PathBuf::from("/")
}

async fn comments(State(web): State<Web>, Path(thread): Path<String>) -> HttpResponse {
    ask(&web, Request::Comments { thread, cwd: no_clone() }).await
}

#[derive(Deserialize)]
struct NewComment {
    on: CommentOn,
    text: String,
}

async fn add_comment(State(web): State<Web>, Path(thread): Path<String>, Json(body): Json<NewComment>) -> HttpResponse {
    let request = Request::Comment {
        thread,
        on: body.on,
        text: body.text,
        cwd: no_clone(),
    };
    ask(&web, request).await
}

async fn resolve_comment(State(web): State<Web>, Path((thread, comment)): Path<(String, String)>) -> HttpResponse {
    let request = Request::ResolveComment {
        thread,
        comment,
        cwd: no_clone(),
    };
    ask(&web, request).await
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
            status: text("status"),
            branch: text("branch"),
            file: text("file"),
            since: number("since"),
            until: number("until"),
        },
    };
    ask(&web, request).await
}

async fn merge_requests(State(web): State<Web>) -> HttpResponse {
    ask(&web, Request::MrList { thread: None }).await
}

async fn merge_request(State(web): State<Web>, Path(mr): Path<String>) -> HttpResponse {
    ask(&web, Request::MrShow { mr }).await
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
