//! The model gateway (ADR 0066). Agents reach a local model only through
//! the daemon, with their run's token, so `llama-server`'s own key never
//! leaves it. The gateway loads the model a request names, one at a time,
//! and stops the server once it has been idle.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use serde_json::{Value, json};
use silt_llm::{Launch, ModelId, Models, Schedule, Server, Step, Ticket};
use silt_store::Store;
use tokio::sync::{Mutex, oneshot};

/// How long a model stays loaded with nothing asking for it.
const IDLE: Duration = Duration::from_secs(10 * 60);
const TICK: Duration = Duration::from_secs(30);
/// Loading a large model from a slow disk takes a while.
const LOADING: Duration = Duration::from_secs(10 * 60);
const MOST_BODY_BYTES: usize = 64 << 20;

/// Where a loaded model is served, and the key it takes.
#[derive(Debug, Clone)]
struct Ready {
    url: String,
    key: String,
}

#[derive(Default)]
struct Running {
    schedule: Schedule,
    server: Option<Server>,
    waiters: HashMap<Ticket, oneshot::Sender<Result<Ready, String>>>,
    /// Why the last load failed, for the requests that waited on it.
    failure: Option<String>,
}

struct Gateway {
    store: Store,
    models: Models,
    program: Option<PathBuf>,
    client: reqwest::Client,
    running: Mutex<Running>,
}

/// Starts the gateway on loopback and answers with its address.
pub(crate) async fn start(store: Store, models: Models, program: Option<PathBuf>) -> std::io::Result<String> {
    let gateway = Arc::new(Gateway { store, models, program, client: reqwest::Client::new(), running: Mutex::default() });
    let app = Router::new()
        .route("/v1/models", get(list))
        .route("/v1/models/{id}", get(one))
        .route("/v1/chat/completions", post(forward))
        .route("/v1/messages", post(forward))
        .route("/v1/messages/count_tokens", post(forward))
        .with_state(gateway.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let url = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move { axum::serve(listener, app).await });
    tokio::spawn(watch(gateway));
    Ok(url)
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, axum::Json(json!({"type": "error", "error": {"type": "invalid_request_error", "message": message.into()}})))
        .into_response()
}

/// Whether the request carries a run's token, as a bearer or an API key.
async fn allowed(gateway: &Gateway, headers: &HeaderMap) -> bool {
    let bearer =
        headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer "));
    let api_key = headers.get("x-api-key").and_then(|value| value.to_str().ok());
    let Some(token) = bearer.or(api_key) else {
        return false;
    };
    let hash = blake3::hash(token.as_bytes());
    matches!(gateway.store.run_token(hash.as_bytes()).await, Ok(Some(_)))
}

/// What a model name in a request means here: `local:<id>` or `<id>`.
fn model_named(name: &str) -> Option<ModelId> {
    ModelId::new(name.strip_prefix("local:").unwrap_or(name)).ok()
}

fn described(model: &silt_llm::Installed) -> Value {
    json!({"id": model.id.as_str(), "object": "model", "name": model.name, "context": model.context, "owned_by": "local"})
}

async fn list(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    if !allowed(&gateway, &headers).await {
        return error(StatusCode::UNAUTHORIZED, "a run's token is needed");
    }
    let models: Vec<Value> = gateway.models.list().await.iter().map(described).collect();
    axum::Json(json!({"object": "list", "data": models})).into_response()
}

async fn one(State(gateway): State<Arc<Gateway>>, UrlPath(id): UrlPath<String>, headers: HeaderMap) -> Response {
    if !allowed(&gateway, &headers).await {
        return error(StatusCode::UNAUTHORIZED, "a run's token is needed");
    }
    let found = match model_named(&id) {
        Some(id) => gateway.models.get(&id).await,
        None => None,
    };
    match found {
        Some(model) => axum::Json(described(&model)).into_response(),
        None => error(StatusCode::NOT_FOUND, format!("there's no model {id} here")),
    }
}

/// Holds a request's place until its answer has been sent, or given up.
struct Place {
    gateway: Arc<Gateway>,
    ticket: Ticket,
}

impl Drop for Place {
    fn drop(&mut self) {
        let gateway = self.gateway.clone();
        let ticket = self.ticket;
        tokio::spawn(async move {
            let mut running = gateway.running.lock().await;
            let steps = running.schedule.done(ticket, Instant::now());
            gateway.apply(&mut running, steps);
        });
    }
}

async fn forward(State(gateway): State<Arc<Gateway>>, request: Request) -> Response {
    if !allowed(&gateway, request.headers()).await {
        return error(StatusCode::UNAUTHORIZED, "a run's token is needed");
    }
    let path = request.uri().path().to_owned();
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), MOST_BODY_BYTES).await else {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "the request is too large");
    };
    let Ok(mut body) = serde_json::from_slice::<Value>(&bytes) else {
        return error(StatusCode::BAD_REQUEST, "the request isn't JSON");
    };
    let named = body["model"].as_str().unwrap_or_default().to_owned();
    let Some(id) = model_named(&named).filter(|_| !named.is_empty()) else {
        return error(StatusCode::BAD_REQUEST, "name a local model, such as `local:gpt-oss-20b`");
    };
    if gateway.models.get(&id).await.is_none() {
        return error(StatusCode::NOT_FOUND, format!("there's no model {id} here; download it with `silt model pull`"));
    }
    body["model"] = Value::String(id.to_string());
    let (place, ready) = match gateway.place(id).await {
        Ok(placed) => placed,
        Err(why) => return error(StatusCode::SERVICE_UNAVAILABLE, why),
    };
    let sent = gateway.client.post(format!("{}{path}", ready.url)).bearer_auth(&ready.key).json(&body).send().await;
    let answer = match sent {
        Ok(answer) => answer,
        Err(failure) => return error(StatusCode::BAD_GATEWAY, format!("the model server didn't answer: {failure}")),
    };
    let mut response = Response::builder().status(answer.status().as_u16());
    if let Some(kind) = answer.headers().get(reqwest::header::CONTENT_TYPE).and_then(|kind| kind.to_str().ok()) {
        response = response.header(header::CONTENT_TYPE, kind);
    }
    // The place is kept until the last byte is sent, or the agent hangs up.
    let stream = answer.bytes_stream().map(move |chunk| {
        let _held = &place;
        chunk.map_err(std::io::Error::other)
    });
    response.body(Body::from_stream(stream)).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

impl Gateway {
    /// Waits for `model` to be loaded and this request's turn.
    async fn place(self: &Arc<Self>, model: ModelId) -> Result<(Place, Ready), String> {
        let (sender, receiver) = oneshot::channel();
        let ticket = {
            let mut running = self.running.lock().await;
            let (ticket, steps) = running.schedule.want(model);
            running.waiters.insert(ticket, sender);
            self.apply(&mut running, steps);
            ticket
        };
        let place = Place { gateway: self.clone(), ticket };
        let ready = receiver.await.map_err(|_| "the model server stopped".to_owned())??;
        Ok((place, ready))
    }

    fn apply(self: &Arc<Self>, running: &mut Running, steps: Vec<Step>) {
        for step in steps {
            match step {
                Step::Run(ticket) => {
                    let ready = running.server.as_ref().map(|server| Ready { url: server.url(), key: server.key.clone() });
                    if let Some(waiter) = running.waiters.remove(&ticket) {
                        let _ = waiter.send(ready.ok_or_else(|| "the model server stopped".to_owned()));
                    }
                }
                Step::Fail(ticket) => {
                    if let Some(waiter) = running.waiters.remove(&ticket) {
                        let _ = waiter.send(Err(running.failure.clone().unwrap_or_else(|| "the model didn't load".to_owned())));
                    }
                }
                Step::Unload => {
                    if let Some(server) = running.server.take() {
                        tokio::spawn(server.stop());
                    }
                }
                Step::Load(model) => {
                    let old = running.server.take();
                    let gateway = self.clone();
                    tokio::spawn(async move {
                        if let Some(old) = old {
                            old.stop().await;
                        }
                        let started = gateway.load(&model).await;
                        let mut running = gateway.running.lock().await;
                        let loaded = match started {
                            Ok(server) => {
                                running.server = Some(server);
                                running.failure = None;
                                true
                            }
                            Err(why) => {
                                tracing::warn!("couldn't load {model}: {why}");
                                running.failure = Some(why);
                                false
                            }
                        };
                        let steps = running.schedule.load_finished(&model, loaded);
                        gateway.apply(&mut running, steps);
                    });
                }
            }
        }
    }

    async fn load(&self, model: &ModelId) -> Result<Server, String> {
        let program =
            self.program.as_deref().ok_or("there's no llama-server here; install llama.cpp, or set SILT_LLAMA_SERVER")?;
        let installed = self.models.get(model).await.ok_or_else(|| format!("there's no model {model} here"))?;
        let launch = Launch {
            weights: self.models.weights(&installed),
            context: installed.context,
            flags: installed.flags.clone(),
            port: silt_llm::free_port().map_err(|error| error.to_string())?,
            key: uuid::Uuid::new_v4().simple().to_string(),
        };
        Server::start(program, &launch, LOADING).await.map_err(|error| error.to_string())
    }
}

/// Stops an idle model, and notices a server that stopped by itself.
async fn watch(gateway: Arc<Gateway>) {
    loop {
        tokio::time::sleep(TICK).await;
        let mut running = gateway.running.lock().await;
        if running.server.as_mut().is_some_and(Server::has_exited) {
            running.server = None;
            let steps = running.schedule.server_exited();
            gateway.apply(&mut running, steps);
        }
        let steps = running.schedule.tick(Instant::now(), IDLE);
        gateway.apply(&mut running, steps);
    }
}
