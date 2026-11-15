//! A stand-in for llama.cpp's `llama-server`: it lists one device, "loads"
//! the file `-m` names, and answers OpenAI's chat API and Anthropic's
//! Messages API with `Heard: <the last prompt>`, behind `--api-key`.
//! - `FAKE_MODEL_REPLY` replies that instead;
//! - `FAKE_MODEL_LOAD_MS` takes that long to load, answering 503 meanwhile;
//! - `FAKE_MODEL_FAILS` fails to load;
//! - `FAKE_MODEL_LOG` names a file each request is appended to, as a JSON
//!   line with the model's file name, the path and the body;
//! - `FAKE_DEVICES` replaces what `--list-devices` prints;
//! - `FAKE_MODEL_TOOLS_FILE` names a file that, while it holds a JSON list of
//!   `{"name", "arguments"}`, makes a chat completion offered tools call
//!   those after a prompt, then answer `Done: <the last result>` once their
//!   results come back.

use std::convert::Infallible;
use std::env;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

const DEVICES: &str = "Available devices:\n  FAKE0: Fake GPU (24576 MiB, 24000 MiB free)\n";

struct Served {
    key: String,
    model: String,
    context: u64,
    ready: AtomicBool,
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|arg| arg == name).and_then(|at| args.get(at + 1)).cloned()
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version") {
        println!("version: 0 (fake)");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--list-devices") {
        print!("{}", env::var("FAKE_DEVICES").unwrap_or_else(|_| DEVICES.to_owned()));
        return ExitCode::SUCCESS;
    }
    let (Some(model), Some(port)) = (flag(&args, "-m"), flag(&args, "--port")) else {
        eprintln!("silt-fake-llama: -m and --port are needed");
        return ExitCode::from(2);
    };
    if env::var_os("FAKE_MODEL_FAILS").is_some() || !Path::new(&model).is_file() {
        eprintln!("llama_model_load: error loading model: failed to open {model}");
        return ExitCode::FAILURE;
    }
    let served = Arc::new(Served {
        key: flag(&args, "--api-key").unwrap_or_default(),
        model: Path::new(&model).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
        context: flag(&args, "-c").and_then(|context| context.parse().ok()).unwrap_or(4096),
        ready: AtomicBool::new(false),
    });
    let load = env::var("FAKE_MODEL_LOAD_MS").ok().and_then(|ms| ms.parse().ok()).unwrap_or(0);
    let loading = served.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(load)).await;
        loading.ready.store(true, Ordering::SeqCst);
    });
    let app = Router::new()
        .route("/health", get(health))
        .route("/props", get(props))
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat))
        .route("/v1/messages", post(messages))
        .with_state(served);
    let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port.parse::<u16>().unwrap_or(0))).await else {
        eprintln!("couldn't bind port {port}");
        return ExitCode::FAILURE;
    };
    eprintln!("main: server is listening on http://127.0.0.1:{port}");
    let _ = axum::serve(listener, app).await;
    ExitCode::SUCCESS
}

fn allowed(served: &Served, headers: &HeaderMap) -> bool {
    let bearer =
        headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer "));
    let api_key = headers.get("x-api-key").and_then(|value| value.to_str().ok());
    served.key.is_empty() || bearer.or(api_key) == Some(served.key.as_str())
}

async fn health(State(served): State<Arc<Served>>) -> Response {
    if served.ready.load(Ordering::SeqCst) {
        axum::Json(json!({"status": "ok"})).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, axum::Json(json!({"error": {"message": "Loading model"}}))).into_response()
    }
}

async fn props(State(served): State<Arc<Served>>, headers: HeaderMap) -> Response {
    if !allowed(&served, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    axum::Json(json!({"default_generation_settings": {"n_ctx": served.context}, "model_path": served.model})).into_response()
}

async fn models(State(served): State<Arc<Served>>, headers: HeaderMap) -> Response {
    if !allowed(&served, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    axum::Json(json!({"object": "list", "data": [{"id": served.model, "object": "model"}]})).into_response()
}

/// The body of a request, logged, if it's allowed.
async fn read(served: &Served, request: Request) -> Result<Value, StatusCode> {
    if !allowed(served, request.headers()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let path = request.uri().path().to_owned();
    let bytes = axum::body::to_bytes(request.into_body(), 1 << 24).await.map_err(|_| StatusCode::BAD_REQUEST)?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    if let Some(log) = env::var_os("FAKE_MODEL_LOG")
        && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log)
    {
        let _ = writeln!(file, "{}", json!({"model": served.model, "path": path, "body": body}));
    }
    Ok(body)
}

/// What the last user message says, as plain text.
fn last_prompt(body: &Value) -> String {
    let message =
        body["messages"].as_array().and_then(|messages| messages.iter().rev().find(|message| message["role"] == "user"));
    match message.map(|message| &message["content"]) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

fn reply_to(body: &Value) -> String {
    let last = body["messages"].as_array().and_then(|messages| messages.last());
    if let Some(result) = last.filter(|message| message["role"] == "tool") {
        return format!("Done: {}", result["content"].as_str().unwrap_or_default());
    }
    env::var("FAKE_MODEL_REPLY").unwrap_or_else(|_| format!("Heard: {}", last_prompt(body)))
}

/// The calls the script makes, when the request offers tools and its last
/// message is a prompt.
fn scripted_calls(body: &Value) -> Vec<Value> {
    let last_is_prompt =
        body["messages"].as_array().and_then(|messages| messages.last()).is_some_and(|message| message["role"] == "user");
    if !body["tools"].is_array() || !last_is_prompt {
        return Vec::new();
    }
    let script = env::var_os("FAKE_MODEL_TOOLS_FILE").and_then(|file| std::fs::read(file).ok());
    let calls: Vec<Value> = script.and_then(|script| serde_json::from_slice(&script).ok()).unwrap_or_default();
    calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            json!({"index": index, "id": format!("call_{index}"), "type": "function",
            "function": {"name": call["name"], "arguments": call["arguments"].to_string()}})
        })
        .collect()
}

fn words(text: &str) -> Vec<String> {
    let mut words: Vec<String> = text.split_inclusive(' ').map(str::to_owned).collect();
    if words.is_empty() {
        words.push(String::new());
    }
    words
}

fn sse(events: Vec<String>) -> Response {
    let stream = futures_util::stream::iter(events.into_iter().map(Ok::<_, Infallible>));
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from_stream(stream))
        .expect("a response builds")
}

async fn chat(State(served): State<Arc<Served>>, request: Request) -> Response {
    let body = match read(&served, request).await {
        Ok(body) => body,
        Err(status) => return status.into_response(),
    };
    let calls = scripted_calls(&body);
    let reply = if calls.is_empty() { reply_to(&body) } else { String::new() };
    let usage = json!({"prompt_tokens": 10, "completion_tokens": words(&reply).len(), "total_tokens": 10 + words(&reply).len()});
    if body["stream"] != true {
        return axum::Json(json!({
            "id": "chatcmpl-fake",
            "object": "chat.completion",
            "model": served.model,
            "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": reply}}],
            "usage": usage,
            "timings": {"prompt_n": 10, "prompt_per_second": 1000.0, "predicted_per_second": 100.0},
        }))
        .into_response();
    }
    let chunk = |delta: Value, finish: Value| {
        let chunk = json!({"id": "chatcmpl-fake", "object": "chat.completion.chunk", "model": served.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]});
        format!("data: {chunk}\n\n")
    };
    let mut events = vec![chunk(json!({"role": "assistant", "content": ""}), Value::Null)];
    if !reply.is_empty() {
        events.extend(words(&reply).into_iter().map(|word| chunk(json!({"content": word}), Value::Null)));
    }
    events.extend(calls.iter().map(|call| chunk(json!({"tool_calls": [call]}), Value::Null)));
    let finish = if calls.is_empty() { "stop" } else { "tool_calls" };
    let mut last: Value =
        serde_json::from_str(chunk(json!({}), json!(finish)).trim_start_matches("data: ").trim()).expect("json");
    last["usage"] = usage;
    events.push(format!("data: {last}\n\n"));
    events.push("data: [DONE]\n\n".to_owned());
    sse(events)
}

async fn messages(State(served): State<Arc<Served>>, request: Request) -> Response {
    let body = match read(&served, request).await {
        Ok(body) => body,
        Err(status) => return status.into_response(),
    };
    let reply = reply_to(&body);
    let usage = json!({"input_tokens": 10, "output_tokens": words(&reply).len()});
    if body["stream"] != true {
        return axum::Json(json!({
            "id": "msg_fake", "type": "message", "role": "assistant", "model": served.model,
            "content": [{"type": "text", "text": reply}], "stop_reason": "end_turn", "usage": usage,
        }))
        .into_response();
    }
    let event = |kind: &str, data: Value| format!("event: {kind}\ndata: {data}\n\n");
    let mut events = vec![
        event(
            "message_start",
            json!({"type": "message_start", "message": {"id": "msg_fake", "type": "message", "role": "assistant",
            "model": served.model, "content": [], "stop_reason": null, "usage": {"input_tokens": 10, "output_tokens": 0}}}),
        ),
        event(
            "content_block_start",
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        ),
    ];
    events.extend(words(&reply).into_iter().map(|word| {
        event(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": word}}),
        )
    }));
    events.push(event("content_block_stop", json!({"type": "content_block_stop", "index": 0})));
    events.push(event("message_delta", json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": usage})));
    events.push(event("message_stop", json!({"type": "message_stop"})));
    sse(events)
}
