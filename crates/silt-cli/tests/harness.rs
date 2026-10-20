//! `silt harness` over ACP against a stand-in for Anthropic's API: it reads,
//! has a stale edit refused after a teammate's change, reads again, edits,
//! and runs a command in the sandbox once allowed.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::routing::post;
use serde_json::{Value, json};
use silt_acp::{Command, Event, Start, Update, start};
use tokio::sync::mpsc;

struct Model {
    checkout: PathBuf,
    /// A directory outside the checkout the sandboxed command tries to write.
    outside: PathBuf,
    requests: Mutex<Vec<Value>>,
}

fn tool(id: &str, name: &str, input: Value) -> Value {
    json!({"content": [{"type": "tool_use", "id": id, "name": name, "input": input}], "stop_reason": "tool_use"})
}

/// Answers each call of the turn in order, like a model would.
async fn messages(State(model): State<Arc<Model>>, Json(request): Json<Value>) -> Json<Value> {
    let call = {
        let mut requests = model.requests.lock().expect("lock");
        requests.push(request);
        requests.len()
    };
    let edit = json!({"path": "src/main.rs", "old": "fn main() {}", "new": "fn main() {\n    println!(\"hi\");\n}"});
    Json(match call {
        1 => tool("t1", "read_file", json!({"path": "src/main.rs"})),
        2 => {
            std::fs::write(model.checkout.join("src/main.rs"), "// a teammate's line\nfn main() {}\n").expect("teammate edits");
            tool("t2", "edit_file", edit)
        }
        3 => tool("t3", "read_file", json!({"path": "src/main.rs"})),
        4 => tool("t4", "edit_file", edit),
        5 => tool(
            "t5",
            "run",
            json!({"command": format!("cat src/main.rs; touch {}/escaped", model.outside.display())}),
        ),
        _ => json!({"content": [{"type": "text", "text": "All done."}], "stop_reason": "end_turn"}),
    })
}

fn sandbox_available() -> bool {
    std::process::Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(["sandbox", "--", "true"])
        .status()
        .is_ok_and(|status| status.success())
}

fn last_tool_result(request: &Value) -> Value {
    request["messages"].as_array().and_then(|messages| messages.last()).expect("a message")["content"][0].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_harness_edits_on_top_of_a_teammate_and_runs_sandboxed() {
    let checkout = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(checkout.path().join("src")).expect("dir");
    std::fs::write(checkout.path().join("src/main.rs"), "fn main() {}\n").expect("file");
    let outside = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let model = Arc::new(Model {
        checkout: checkout.path().to_owned(),
        outside: outside.path().to_owned(),
        requests: Mutex::default(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let address = listener.local_addr().expect("address");
    let app = axum::Router::new().route("/v1/messages", post(messages)).with_state(model.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });

    let home = tempfile::tempdir().expect("tempdir");
    let mut env = vec![
        ("ANTHROPIC_API_KEY".to_owned(), "test-key".to_owned()),
        ("ANTHROPIC_BASE_URL".to_owned(), format!("http://{address}")),
        ("SILT_HOME".to_owned(), home.path().to_string_lossy().into_owned()),
    ];
    let sandboxed = sandbox_available();
    if !sandboxed {
        assert!(std::env::var_os("SILT_REQUIRE_SANDBOX").is_none(), "the sandbox is required but unavailable");
        eprintln!("namespaces are off here; running the command unsandboxed");
        env.push(("SILT_HARNESS_UNSANDBOXED".to_owned(), "1".to_owned()));
    }
    let (events, mut received) = mpsc::unbounded_channel();
    let agent = start(
        Start {
            program: env!("CARGO_BIN_EXE_silt").to_owned(),
            args: vec!["harness".to_owned()],
            env,
            cwd: checkout.path().to_owned(),
            mcp: None,
            load: None,
        },
        events,
    );
    let next = async |received: &mut mpsc::UnboundedReceiver<Event>| {
        tokio::time::timeout(Duration::from_secs(30), received.recv()).await.expect("in time").expect("an event")
    };
    assert!(matches!(next(&mut received).await, Event::Ready { loaded: false, .. }));
    agent.send(Command::Prompt("print hi from main".into()));

    let mut said = String::new();
    let mut outputs = Vec::new();
    loop {
        match next(&mut received).await {
            Event::Permission(permission) => {
                assert!(permission.title.contains("cat src/main.rs"), "{permission:?}");
                agent.send(Command::Answer {
                    id: permission.id,
                    option: Some("allow".into()),
                });
            }
            Event::Update(Update::AgentText(text)) => said.push_str(&text),
            Event::Update(Update::ToolUpdate { output: Some(output), .. }) => outputs.push(output),
            Event::TurnEnded { reason } => {
                assert_eq!(reason, "end_turn");
                break;
            }
            Event::Failed(error) => panic!("{error}"),
            _ => {}
        }
    }
    assert_eq!(said, "All done.");
    let requests = model.requests.lock().expect("lock").clone();
    let refusal = last_tool_result(&requests[2]);
    assert_eq!(refusal["is_error"], true);
    assert!(refusal["content"].as_str().expect("text").contains("changed since you last read it"));
    assert_eq!(requests[0]["model"], "claude-opus-5-5");

    let edited = "// a teammate's line\nfn main() {\n    println!(\"hi\");\n}\n";
    assert_eq!(std::fs::read_to_string(checkout.path().join("src/main.rs")).expect("reads"), edited);
    let ran = outputs.last().expect("the command's output");
    assert!(ran.contains("println!(\"hi\")"), "{ran}");
    if sandboxed {
        assert!(!outside.path().join("escaped").exists(), "the command wrote outside the checkout");
    }
    agent.send(Command::Stop);
}

/// Copies the redacted token it was shown into another file, and says so.
async fn copying(State(requests): State<Arc<Mutex<Vec<Value>>>>, Json(request): Json<Value>) -> Json<Value> {
    let call = {
        let mut requests = requests.lock().expect("lock");
        requests.push(request.clone());
        requests.len()
    };
    let shown = last_tool_result(&request)["content"].as_str().unwrap_or_default().to_owned();
    let hidden = shown.split_whitespace().find(|word| word.starts_with("[REDACTED:")).unwrap_or_default().to_owned();
    Json(match call {
        1 => tool("t1", "read_file", json!({"path": "config.txt"})),
        2 => tool("t2", "write_file", json!({"path": "copy.txt", "content": format!("copied {hidden}\n")})),
        _ => {
            let earlier = requests.lock().expect("lock")[1].clone();
            let hidden = last_tool_result(&earlier)["content"].as_str().unwrap_or_default().split_whitespace().last().unwrap_or_default().to_owned();
            json!({"content": [{"type": "text", "text": format!("Copied {hidden}.")}], "stop_reason": "end_turn"})
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_never_reach_the_model_but_its_edits_keep_them() {
    let token = "ghp_abcdefghijklmnopqrstuvwxyz0123";
    let checkout = tempfile::tempdir().expect("tempdir");
    std::fs::write(checkout.path().join("config.txt"), format!("token {token}\n")).expect("file");
    let requests: Arc<Mutex<Vec<Value>>> = Arc::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let address = listener.local_addr().expect("address");
    let app = axum::Router::new().route("/v1/messages", post(copying)).with_state(requests.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });

    let home = tempfile::tempdir().expect("tempdir");
    let (events, mut received) = mpsc::unbounded_channel();
    let agent = start(
        Start {
            program: env!("CARGO_BIN_EXE_silt").to_owned(),
            args: vec!["harness".to_owned()],
            env: vec![
                ("ANTHROPIC_API_KEY".to_owned(), "test-key".to_owned()),
                ("ANTHROPIC_BASE_URL".to_owned(), format!("http://{address}")),
                ("SILT_HOME".to_owned(), home.path().to_string_lossy().into_owned()),
            ],
            cwd: checkout.path().to_owned(),
            mcp: None,
            load: None,
        },
        events,
    );
    let mut said = String::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(30), received.recv()).await.expect("in time").expect("an event") {
            Event::Ready { .. } => {
                agent.send(Command::Prompt(format!("copy the token, it is {token}")));
            }
            Event::Update(Update::AgentText(text)) => said.push_str(&text),
            Event::TurnEnded { .. } => break,
            Event::Failed(error) => panic!("{error}"),
            _ => {}
        }
    }
    let sent = serde_json::to_string(&*requests.lock().expect("lock")).expect("json");
    assert!(!sent.contains(token), "the model saw the token: {sent}");
    assert!(sent.contains(&silt_redact::placeholder(token)));
    assert_eq!(
        std::fs::read_to_string(checkout.path().join("copy.txt")).expect("written"),
        format!("copied {token}\n")
    );
    assert_eq!(said, format!("Copied {token}."));
    agent.send(Command::Stop);
}
