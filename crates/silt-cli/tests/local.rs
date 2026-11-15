//! Silt's own agent, `local`, in a chat on a local model (ADR 0072), with
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
fn requests(log: &Path) -> Vec<Value> {
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
    let first = requests(&log);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["model"], "Tiny.gguf", "the fake server saw the model it loaded");
    assert_eq!(first[0]["body"]["model"], "tiny");

    laptop.web("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(json!({"text": "And then?"})));
    assert!(said(&laptop, &thread, "Heard: And then?"));
    let conversation = requests(&log).last().expect("a second request")["body"]["messages"].clone();
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
    let resumed = requests(&log).last().expect("a third request")["body"]["messages"].clone();
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
    assert_eq!(env["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "32768", "it compacts for the local model's context");
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

fn said_like(machine: &Machine, thread: &str, start: &str) -> bool {
    eventually(|| {
        let history = machine.web("GET", &format!("/api/threads/{thread}/history"), None);
        let said = history["entries"].as_array().is_some_and(|entries| {
            entries.iter().any(|entry| entry["text"].as_str().is_some_and(|text| text.starts_with(start)))
        });
        said && machine.web("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["state"] == "idle"
    })
}

#[test]
fn the_local_agent_works_with_its_tools_and_asks_before_it_writes() {
    let root = common::temp_root("silt-local-tools");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let model = root.path().join("Tiny.gguf");
    fs::write(&model, gguf("Tiny")).expect("write");
    laptop.silt(&["model", "pull", &model.to_string_lossy()]);
    let script = json!([{"name": "write", "arguments": {"path": "notes.md", "content": "disk full at 03:12"}}]);
    fs::write(laptop.agents.join("logs/model-tools.json"), script.to_string()).expect("the script");

    let start = json!({"agent": "local", "chat": true, "prompt": "Note what failed", "preset": "ask_for_everything"});
    let thread = laptop.web("POST", "/api/agents/start", Some(start)).as_str().expect("the new chat").to_owned();
    let mut asked = Value::Null;
    assert!(eventually(|| {
        let live = laptop.web("GET", &format!("/api/threads/{thread}/live"), None);
        asked = live["agent"]["permissions"][0].clone();
        !asked.is_null()
    }));
    assert_eq!(asked["title"], "Write notes.md", "{asked}");
    let answer = json!({"id": asked["id"], "option": "allow"});
    laptop.web("POST", &format!("/api/threads/{thread}/agent/answer"), Some(answer));
    assert!(said_like(&laptop, &thread, "Done: Wrote"), "{}", laptop.web("GET", &format!("/api/threads/{thread}/history"), None));

    let folder = fs::read_dir(laptop.silt_home.join("chats")).expect("chats").next().expect("one chat").expect("entry").path();
    assert_eq!(fs::read_to_string(folder.join("notes.md")).expect("the note"), "disk full at 03:12");
    let history = laptop.web("GET", &format!("/api/threads/{thread}/history"), None).to_string();
    assert!(history.contains("Write notes.md"), "the tool call is in the history: {history}");
    let sent = requests(&laptop.agents.join("logs/model-requests.jsonl"));
    let tools = &sent[0]["body"]["tools"];
    assert!(tools.as_array().is_some_and(|tools| tools.iter().any(|tool| tool["function"]["name"] == "grep")), "{tools}");
}

#[test]
fn in_chat_mode_the_local_agent_is_offered_no_tools() {
    let root = common::temp_root("silt-local-chat");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let model = root.path().join("Tiny.gguf");
    fs::write(&model, gguf("Tiny")).expect("write");
    laptop.silt(&["model", "pull", &model.to_string_lossy()]);
    let script = json!([{"name": "run", "arguments": {"command": "touch should-not-exist"}}]);
    fs::write(laptop.agents.join("logs/model-tools.json"), script.to_string()).expect("the script");
    let start = json!({"agent": "local", "chat": true, "prompt": "Just talk", "mode": "chat"});
    let thread = laptop.web("POST", "/api/agents/start", Some(start)).as_str().expect("the new chat").to_owned();
    assert!(said(&laptop, &thread, "Heard: Just talk"));
    let sent = requests(&laptop.agents.join("logs/model-requests.jsonl"));
    assert!(sent.iter().all(|request| request["body"]["tools"].is_null()));
}

#[test]
fn a_local_only_workspace_runs_only_local_models_and_keeps_its_threads_here() {
    let root = common::temp_root("silt-local-only");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let model = root.path().join("Tiny.gguf");
    fs::write(&model, gguf("Tiny")).expect("write");
    laptop.silt(&["model", "pull", &model.to_string_lossy()]);
    let repo = laptop.repo.to_string_lossy().into_owned();
    laptop.silt(&["workspace", "new", "Logs", "--local-only", "--folder", &repo]);

    let codex = laptop.run(&["codex"]);
    assert!(!codex.status.success());
    assert!(String::from_utf8_lossy(&codex.stderr).contains("Logs is local only: Codex doesn't run on a local model"));
    let online = laptop.run(&["claude"]);
    assert!(!online.status.success());
    assert!(String::from_utf8_lossy(&online.stderr).contains("choose a local model for Claude Code"));

    laptop.silt(&["claude", "--model", "local:tiny"]);
    let threads = laptop.web("GET", "/api/threads", None);
    let recorded = threads.as_array().and_then(|threads| threads.first()).cloned().expect("the run's thread");
    assert_eq!(recorded["local_only"], true, "it stays on this machine: {recorded}");

    let bound = laptop.run(&["workspace", "policy", "Default", "--local-only", "on"]);
    assert!(!bound.status.success());
    assert!(String::from_utf8_lossy(&bound.stderr).contains("a Local only workspace has none"));
}

/// What `silt mcp` answers one call with, for a run's token.
fn mcp_answer(machine: &Machine, thread: &str, token: &str, tool: &str, arguments: Value) -> String {
    use std::io::Write as _;
    let mut mcp = std::process::Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(["mcp", "--thread", thread, "--token", token])
        .env("SILT_HOME", &machine.silt_home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("silt mcp runs");
    {
        let mut stdin = mcp.stdin.take().expect("stdin");
        let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": arguments}});
        writeln!(stdin, "{call}").expect("write");
    }
    String::from_utf8_lossy(&mcp.wait_with_output().expect("mcp exits").stdout).into_owned()
}

/// Claude Code's run in `dir` on the local model, and the thread and token
/// it got.
fn claude_in(machine: &Machine, dir: &Path) -> (String, String) {
    let output = machine.command(&["claude", "--model", "local:tiny"], &[]).current_dir(dir).output().expect("silt runs");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let run: Value =
        serde_json::from_slice(&fs::read(machine.agents.join("logs/claude-run")).expect("the run's log")).expect("JSON");
    let args: Vec<&str> = run["args"].as_array().expect("args").iter().filter_map(Value::as_str).collect();
    let config: Value =
        serde_json::from_str(args.iter().skip_while(|arg| **arg != "--mcp-config").nth(1).expect("Silt's MCP server"))
            .expect("JSON");
    let mcp: Vec<String> = config["mcpServers"]["silt"]["args"]
        .as_array()
        .expect("args")
        .iter()
        .filter_map(|arg| arg.as_str().map(str::to_owned))
        .collect();
    let after = |flag: &str| mcp.iter().skip_while(|arg| *arg != flag).nth(1).cloned().expect("the flag's value");
    (after("--thread"), after("--token"))
}

#[test]
fn in_a_local_only_workspace_an_agents_history_tools_read_its_own_project_only() {
    let root = common::temp_root("silt-local-history");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let model = root.path().join("Tiny.gguf");
    fs::write(&model, gguf("Tiny")).expect("write");
    laptop.silt(&["model", "pull", &model.to_string_lossy()]);
    // Two projects under one folder: a clone, and a repository of its own.
    let logs = root.path().join("laptop/logs");
    let service = logs.join("service");
    common::git(root.path(), &["clone", "-q", &origin.to_string_lossy(), &service.to_string_lossy()]);
    let notes = logs.join("notes");
    fs::create_dir_all(&notes).expect("mkdir");
    common::git(&notes, &["init", "-q"]);
    fs::write(notes.join("README.md"), "notes\n").expect("write");
    common::git(&notes, &["add", "."]);
    common::git(&notes, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "first"]);
    let logs = common::canonical(&logs);
    laptop.silt(&["workspace", "new", "Logs", "--local-only", "--folder", &logs.to_string_lossy()]);

    let (notes_thread, _) = claude_in(&laptop, &common::canonical(&notes));
    let (service_thread, token) = claude_in(&laptop, &common::canonical(&service));
    let own = mcp_answer(&laptop, &service_thread, &token, "silt_history_read", json!({}));
    assert!(!own.contains("no thread matches"), "it reads its own project's thread: {own}");
    let other = mcp_answer(&laptop, &service_thread, &token, "silt_history_read", json!({"thread": notes_thread}));
    assert!(other.contains("no thread matches"), "another project in the workspace is out of reach: {other}");

    laptop.silt(&["workspace", "policy", "Logs", "--wide-history", "on"]);
    let (service_thread, token) = claude_in(&laptop, &common::canonical(&service));
    let wide = mcp_answer(&laptop, &service_thread, &token, "silt_history_read", json!({"thread": notes_thread}));
    assert!(!wide.contains("no thread matches"), "widened to the workspace: {wide}");
}

/// Whether `needle` is anywhere in the store's files, the WAL included.
fn in_store(machine: &Machine, needle: &str) -> bool {
    ["silt.sqlite", "silt.sqlite-wal"].iter().any(|name| {
        fs::read(machine.silt_home.join(name))
            .is_ok_and(|bytes| bytes.windows(needle.len()).any(|window| window == needle.as_bytes()))
    })
}

#[test]
fn erasing_a_local_only_thread_removes_it_its_forks_and_the_agents_copies() {
    let root = common::temp_root("silt-local-erase");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let model = root.path().join("Tiny.gguf");
    fs::write(&model, gguf("Tiny")).expect("write");
    laptop.silt(&["model", "pull", &model.to_string_lossy()]);
    let repo = laptop.repo.to_string_lossy().into_owned();
    laptop.silt(&["workspace", "new", "Logs", "--local-only", "--folder", &repo]);

    laptop.silt(&["claude", "--model", "local:tiny"]);
    let thread = laptop.web("GET", "/api/threads", None)[0]["id"].as_str().expect("the thread").to_owned();
    // The fork's agent says something only it says, which the search index keeps as one word.
    let said = [("FAKE_SAY", "the zebrafjord42 customer's export failed")];
    let forked = laptop.command(&["fork", &thread, "--", "--model", "local:tiny"], &said).output().expect("silt runs");
    assert!(forked.status.success(), "{}", String::from_utf8_lossy(&forked.stderr));
    let fork = String::from_utf8_lossy(&forked.stdout).into_owned();
    let transcripts: Vec<std::path::PathBuf> = walk(&laptop.claude_dir.join("projects"));
    assert!(!transcripts.is_empty(), "Claude Code kept its transcript");
    assert!(in_store(&laptop, "zebrafjord42"), "the store holds what was said");

    let asked = laptop.stdout(&["thread", "erase", &thread]);
    assert!(asked.contains("erasing removes threads"), "{asked}");
    assert!(asked.contains("--yes"), "{asked}");
    assert_eq!(laptop.web("GET", "/api/threads", None).as_array().map(Vec::len), Some(2), "nothing went yet: {fork}");

    let erased = laptop.stdout(&["thread", "erase", &thread, "--yes"]);
    assert!(erased.starts_with("erased"), "{erased}");
    assert_eq!(laptop.web("GET", "/api/threads", None).as_array().map(Vec::len), Some(0), "the thread and its fork are gone");
    assert!(transcripts.iter().all(|path| !path.exists()), "Claude Code's own copy is gone");
    assert!(!in_store(&laptop, "zebrafjord42"), "nothing of it stays in the store's files");
    assert!(!laptop.stdout(&["search", "zebrafjord42"]).contains("zebrafjord42"), "search finds nothing of it");

    let elsewhere = laptop.run(&["thread", "erase", "nope"]);
    assert!(!elsewhere.status.success());
}

#[test]
fn only_a_local_only_thread_can_be_erased() {
    let root = common::temp_root("silt-local-erase-refused");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    laptop.silt(&["claude"]);
    let thread = laptop.web("GET", "/api/threads", None)[0]["id"].as_str().expect("the thread").to_owned();
    let refused = laptop.run(&["thread", "erase", &thread, "--yes"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("only threads of a Local only workspace can be erased"));
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "jsonl") {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn an_erasure_a_stop_interrupted_is_finished_at_the_next_start() {
    let root = common::temp_root("silt-local-erase-resumed");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let model = root.path().join("Tiny.gguf");
    fs::write(&model, gguf("Tiny")).expect("write");
    laptop.silt(&["model", "pull", &model.to_string_lossy()]);
    let repo = laptop.repo.to_string_lossy().into_owned();
    laptop.silt(&["workspace", "new", "Logs", "--local-only", "--folder", &repo]);
    laptop.silt(&["claude", "--model", "local:tiny"]);
    let thread = laptop.web("GET", "/api/threads", None)[0]["id"].as_str().expect("the thread").to_owned();

    laptop.silt(&["daemon", "stop"]);
    let markers = laptop.silt_home.join("erasing");
    fs::create_dir_all(&markers).expect("mkdir");
    let marker = markers.join(format!("{thread}.json"));
    fs::write(&marker, json!({"threads": [thread], "files": [], "checkouts": [], "blobs": []}).to_string()).expect("the marker");
    laptop.silt(&["model", "list"]);
    assert!(eventually(|| !marker.exists()), "the erasure was finished");
    assert_eq!(laptop.web("GET", "/api/threads", None).as_array().map(Vec::len), Some(0));
}
