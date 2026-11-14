//! Silt's own agent, `local`, in a chat on a local model (ADR 0066), with
//! `silt-fake-llama` in place of llama.cpp's server.

mod common;

use std::fs;
use std::path::Path;

use common::{Machine, Server, eventually, origin};
use serde_json::{Value, json};

/// A GGUF header, all a model needs to be listed and "loaded".
fn gguf(name: &str) -> Vec<u8> {
    let mut bytes = b"GGUF".to_vec();
    bytes.extend(3u32.to_le_bytes());
    bytes.extend(0u64.to_le_bytes());
    bytes.extend(2u64.to_le_bytes());
    for (key, value) in [("general.architecture", "llama"), ("general.name", name)] {
        bytes.extend((key.len() as u64).to_le_bytes());
        bytes.extend(key.as_bytes());
        bytes.extend(8u32.to_le_bytes());
        bytes.extend((value.len() as u64).to_le_bytes());
        bytes.extend(value.as_bytes());
    }
    bytes
}

fn said(machine: &Machine, thread: &str, text: &str) -> bool {
    eventually(|| {
        let history = machine.web("GET", &format!("/api/threads/{thread}/history"), None);
        let said = history["entries"].as_array().is_some_and(|entries| entries.iter().any(|entry| entry["text"] == text));
        said && machine.web("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["state"] == "idle"
    })
}

/// What the fake server was asked to complete, in order.
fn asked(log: &Path) -> Vec<Value> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|request| request["path"] == "/v1/chat/completions")
        .collect()
}

#[test]
fn a_chat_with_the_local_agent_answers_on_the_local_model_and_picks_up_again() {
    let root = common::temp_root("silt-local");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let model = root.path().join("Tiny.gguf");
    fs::write(&model, gguf("Tiny")).expect("write");
    laptop.silt(&["model", "pull", &model.to_string_lossy()]);
    let log = laptop.agents.join("logs/model-requests.jsonl");

    let agents = laptop.web("GET", "/api/agents", None);
    let local = agents["agents"].as_array().and_then(|agents| agents.iter().find(|agent| agent["id"] == "local")).cloned();
    let local = local.expect("Silt's own agent is offered");
    assert_eq!(local["settings"][0]["category"], "model", "{local}");
    assert_eq!(local["settings"][0]["choices"][0]["id"], "tiny", "{local}");

    let started =
        laptop.web("POST", "/api/agents/start", Some(json!({"agent": "local", "chat": true, "prompt": "What failed?"})));
    let thread = started.as_str().expect("the new chat").to_owned();
    assert!(said(&laptop, &thread, "Heard: What failed?"), "the local model answered");
    let first = asked(&log);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["model"], "Tiny.gguf", "the fake server saw the model it loaded");
    assert_eq!(first[0]["body"]["model"], "tiny");

    laptop.web("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(json!({"text": "And then?"})));
    assert!(said(&laptop, &thread, "Heard: And then?"));
    let conversation = asked(&log).last().expect("a second request")["body"]["messages"].clone();
    let texts: Vec<&str> =
        conversation.as_array().expect("messages").iter().filter_map(|message| message["content"].as_str()).collect();
    assert_eq!(texts[1..], ["What failed?", "Heard: What failed?", "And then?"], "the conversation goes along");

    laptop.web("POST", &format!("/api/threads/{thread}/agent/stop"), None);
    assert!(eventually(|| laptop.web("GET", &format!("/api/threads/{thread}/live"), None)["agent"].is_null()
        || laptop.web("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["state"] == "stopped"));
    laptop.web("POST", "/api/agents/start", Some(json!({"thread": thread, "agent": "local", "prompt": "Still there?"})));
    assert!(
        said(&laptop, &thread, "Heard: Still there?"),
        "{}\n{}",
        laptop.web("GET", &format!("/api/threads/{thread}/history"), None),
        laptop.daemon_log()
    );
    let resumed = asked(&log).last().expect("a third request")["body"]["messages"].clone();
    assert_eq!(resumed.as_array().map(Vec::len), Some(1 + 5), "its session loaded again, all of it: {resumed}");
}

#[test]
fn the_local_agent_without_a_model_says_how_to_get_one() {
    let root = common::temp_root("silt-local-none");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let started = laptop.web("POST", "/api/agents/start", Some(json!({"agent": "local", "chat": true, "prompt": "Hello?"})));
    let thread = started.as_str().expect("the new chat").to_owned();
    assert!(eventually(|| {
        laptop
            .web("GET", &format!("/api/threads/{thread}/history"), None)
            .to_string()
            .contains("No local model is downloaded yet")
    }));
}

#[test]
fn claude_code_runs_on_a_local_model_through_the_gateway() {
    let root = common::temp_root("silt-local-claude");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let model = root.path().join("Tiny.gguf");
    fs::write(&model, gguf("Tiny")).expect("write");
    laptop.silt(&["model", "pull", &model.to_string_lossy()]);

    laptop.silt(&["claude", "--model", "local:tiny"]);
    let run: Value =
        serde_json::from_slice(&fs::read(laptop.agents.join("logs/claude-run")).expect("the run's log")).expect("JSON");
    let args: Vec<&str> = run["args"].as_array().expect("args").iter().filter_map(Value::as_str).collect();
    assert!(
        !args.iter().any(|arg| arg.contains("local:tiny") || *arg == "--model"),
        "the flag goes in the environment: {args:?}"
    );
    let env = &run["env"];
    assert_eq!(env["ANTHROPIC_MODEL"], "local:tiny");
    assert_eq!(env["ANTHROPIC_SMALL_FAST_MODEL"], "local:tiny");
    assert_eq!(env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"], "1");
    let gateway = env["ANTHROPIC_BASE_URL"].as_str().expect("the gateway");
    assert!(gateway.starts_with("http://127.0.0.1:"), "{gateway}");
    let token = env["ANTHROPIC_AUTH_TOKEN"].as_str().expect("the run's token");

    let models = |token: &str| {
        let output = std::process::Command::new("curl")
            .args(["-s", "-w", "\n%{http_code}", "-H", &format!("x-api-key: {token}"), &format!("{gateway}/v1/models")])
            .output()
            .expect("curl runs");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let listed = models(token);
    assert!(listed.ends_with("200") && listed.contains("\"tiny\""), "{listed}");
    assert!(models("not-a-run").ends_with("401"));

    let refused = laptop.run(&["claude", "--model", "local:nope"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("there's no local model nope here"));
}
