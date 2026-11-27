//! Memory agents share through Silt's MCP server (ADR 0137).

mod common;

use std::fs;
use std::io::Write;
use std::process::Stdio;

use common::{Machine, Server, origin, thread_of};
use serde_json::{Value, json};

/// Sends `calls` to `silt mcp` for the machine's thread, as an agent other
/// than Claude Code would, and gives its replies.
fn mcp(machine: &Machine, thread: &str, calls: &[Value]) -> Vec<Value> {
    let mut command = machine.command(&["mcp", "--thread", thread, "--checkout"], &[]);
    let mut server = command
        .arg(&machine.repo)
        .env_remove("CLAUDECODE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("silt mcp runs");
    {
        let mut stdin = server.stdin.take().expect("stdin");
        for (id, call) in calls.iter().enumerate() {
            let mut call = call.clone();
            call["jsonrpc"] = json!("2.0");
            call["id"] = json!(id);
            writeln!(stdin, "{call}").expect("write");
        }
    }
    let output = server.wait_with_output().expect("mcp exits");
    String::from_utf8_lossy(&output.stdout).lines().map(|line| serde_json::from_str(line).expect("json")).collect()
}

fn text(reply: &Value) -> String {
    reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_owned()
}

fn names(reply: &Value) -> Vec<String> {
    reply["result"]["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn agents_read_and_add_to_the_memory_they_share() {
    let root = tempfile::tempdir().expect("temp dir");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let machine = Machine::new(root.path(), "laptop", &origin, &server);
    machine.silt(&["claude"]);
    let thread = thread_of(&machine);
    let slug: String =
        machine.repo.to_string_lossy().chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' }).collect();
    let project = machine.claude_dir.join("projects").join(slug).join("memory");
    let everywhere = machine.silt_home.join("memory/everywhere");

    let write = |name: &str, kind: &str, body: &str| json!({"method": "tools/call", "params": {"name": "silt_memory_write", "arguments": {"name": name, "kind": kind, "body": body, "description": "one line"}}});
    let replies = mcp(
        &machine,
        &thread,
        &[
            json!({"method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
            json!({"method": "tools/list"}),
            write("The gate runs on the pull request", "project", "CI runs it whole."),
            write("Writes British English", "user", "Colour, not color."),
            json!({"method": "tools/call", "params": {"name": "silt_memory_search", "arguments": {"query": "gate"}}}),
            json!({"method": "tools/call", "params": {"name": "silt_memory_read", "arguments": {"file": "user-writes-british-english.md", "everywhere": true}}}),
            json!({"method": "tools/call", "params": {"name": "silt_memory_read", "arguments": {"file": "../secrets.md"}}}),
        ],
    );
    let instructions = replies[0]["result"]["instructions"].as_str().unwrap_or_default();
    assert!(instructions.contains("# Memory") && instructions.contains("Nothing is remembered yet"), "{instructions}");
    assert!(names(&replies[1]).iter().any(|name| name == "silt_memory_write"), "{:?}", names(&replies[1]));
    assert_eq!(text(&replies[2]), "Saved as project-the-gate-runs-on-the-pull-request.md.");
    assert!(project.join("project-the-gate-runs-on-the-pull-request.md").exists(), "in the project's shared memory");
    assert_eq!(text(&replies[3]), "Saved as user-writes-british-english.md, in the memory everywhere.");
    assert!(everywhere.join("user-writes-british-english.md").exists(), "what's about the person holds everywhere");
    assert!(text(&replies[4]).contains("[project] The gate runs on the pull request"), "{}", text(&replies[4]));
    assert!(text(&replies[5]).contains("Colour, not color."), "{}", text(&replies[5]));
    assert!(replies[6].to_string().contains("isn't a memory's file"), "{}", replies[6]);

    // The next agent is told what's remembered.
    let replies = mcp(&machine, &thread, &[json!({"method": "initialize", "params": {"protocolVersion": "2025-06-18"}})]);
    let instructions = replies[0]["result"]["instructions"].as_str().unwrap_or_default();
    assert!(instructions.contains("## This project") && instructions.contains("## Everywhere"), "{instructions}");
    assert!(instructions.contains("(user-writes-british-english.md) — one line"), "{instructions}");

    // Off, each agent keeps its own memory, and Silt gives none.
    fs::write(machine.silt_home.join("memory.json"), r#"{"share":false}"#).expect("write");
    let replies = mcp(
        &machine,
        &thread,
        &[json!({"method": "initialize", "params": {"protocolVersion": "2025-06-18"}}), json!({"method": "tools/list"})],
    );
    assert!(!replies[0]["result"]["instructions"].as_str().unwrap_or_default().contains("# Memory"));
    assert!(!names(&replies[1]).iter().any(|name| name.starts_with("silt_memory")), "{:?}", names(&replies[1]));
}
