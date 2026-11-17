//! Two agents in one clone each record their own work (ADR 0089), and know about each other (ADR 0090).

mod common;

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Output, Stdio};

use common::{Machine, Server, eventually, origin};

struct World {
    laptop: Machine,
    _server: Server,
    root: tempfile::TempDir,
}

fn world(daemon: bool) -> World {
    let root = common::temp_root("silt-peers");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    if daemon {
        laptop.silt(&["daemon", "start"]);
    } else {
        laptop.silt(&["daemon", "stop"]);
    }
    World { laptop, _server: server, root }
}

fn in_tool(machine: &Machine, args: &[&str], marker: &Path, envs: &[(&str, &str)]) -> Child {
    let marker_env = marker.to_string_lossy().into_owned();
    let mut envs = envs.to_vec();
    envs.push(("FAKE_IN_TOOL", &marker_env));
    let child = machine.command(args, &envs).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().expect("starts");
    assert!(eventually(|| marker.exists()), "the agent reaches its tool");
    child
}

fn let_go(marker: &Path) {
    fs::write(PathBuf::from(format!("{}.go", marker.display())), "").expect("let the agent go");
}

fn finished(child: Child) -> Output {
    let output = child.wait_with_output().expect("waits");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("could not"), "everything was recorded: {stderr}");
    output
}

fn thread_named(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let line = stderr.lines().find(|line| line.contains("thread ")).unwrap_or_else(|| panic!("names its thread: {stderr}"));
    line.split("thread ").nth(1).and_then(|rest| rest.split([' ', ',']).next()).expect("thread id").to_owned()
}

/// Agent A is held in its tool, B starts in the same clone and is held in
/// its own, then A and B each finish their edit in turn.
fn two_agents_in_one_clone_are_both_recorded(daemon: bool) {
    let world = world(daemon);
    let laptop = &world.laptop;
    let envs: &[(&str, &str)] = if daemon { &[] } else { &[("SILT_DAEMONLESS_CAPTURE", "1")] };
    let (marker_a, marker_b) = (world.root.path().join("in-tool-a"), world.root.path().join("in-tool-b"));
    let a = in_tool(laptop, &["claude"], &marker_a, envs);
    let b = in_tool(laptop, &["claude"], &marker_b, envs);
    if daemon {
        // While both are in a tool call, an edit nobody's hook names could
        // be either's.
        fs::write(laptop.repo.join("NOTES.md"), "by hand or by an agent\n").expect("writes");
        let both_took_it = || {
            let log = laptop.stdout(&["log"]);
            let threads: Vec<&str> = log.lines().filter_map(|line| line.split_whitespace().next()).collect();
            threads.len() == 2 && threads.iter().all(|thread| laptop.stdout(&["show", thread]).contains("NOTES.md"))
        };
        assert!(eventually(both_took_it), "both threads take the edit in");
    }
    let_go(&marker_a);
    let a = finished(a);
    let_go(&marker_b);
    let b = finished(b);
    let (thread_a, thread_b) = (thread_named(&a.stderr), thread_named(&b.stderr));
    assert_ne!(thread_a, thread_b);
    let (show_a, show_b) = (laptop.stdout(&["show", &thread_a]), laptop.stdout(&["show", &thread_b]));
    for show in [&show_a, &show_b] {
        assert!(show.contains("Done: farewell added."), "{show}");
        assert!(show.contains("    ✎ src/main.rs +17 -0"), "each agent's own edit is under its tool call: {show}");
    }
    assert!(!show_a.contains("by another agent"), "B's edit came after A was done: {show_a}");
    let others = show_b.lines().find(|line| line.contains("by another agent")).unwrap_or_else(|| panic!("{show_b}"));
    assert!(others.starts_with("  ✎ by another agent (claude-code ") && others.ends_with("): src/main.rs +17 -0"), "{others}");
    if daemon {
        for show in [&show_a, &show_b] {
            let unclear = show.lines().find(|line| line.contains("by one of these agents")).unwrap_or_else(|| panic!("{show}"));
            assert!(unclear.ends_with("NOTES.md +23 -0"), "{unclear}");
        }
    }
    let farewells = |diff: &str| diff.lines().filter(|line| *line == "+fn farewell() {}").count();
    let diff_b = laptop.stdout(&["diff", &thread_b]);
    assert_eq!(farewells(&diff_b), 2, "B's tree has both edits: {diff_b}");
    let without_others = laptop.stdout(&["diff", &thread_b, "--hide-others"]);
    assert_eq!(farewells(&without_others), 1, "A's edit is left out: {without_others}");
    assert_eq!(without_others.contains("NOTES.md"), daemon, "an unclear edit may be B's own, so it stays");
    let main = fs::read_to_string(laptop.repo.join("src/main.rs")).expect("reads");
    assert_eq!(main.matches("fn farewell() {}").count(), 2, "{main}");
}

#[test]
fn two_agents_in_one_clone_are_both_recorded_with_a_daemon() {
    two_agents_in_one_clone_are_both_recorded(true);
}

#[test]
fn two_agents_in_one_clone_are_both_recorded_without_a_daemon() {
    two_agents_in_one_clone_are_both_recorded(false);
}

fn refs(owned: &[(String, String)]) -> Vec<(&str, &str)> {
    owned.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect()
}

/// What a hook the fake agent ran said back, from `FAKE_HOOK_LOG`.
fn hook_said(log: &Path, event: &str) -> String {
    let suffix = format!("-{event}");
    let found = fs::read_dir(log)
        .expect("hook log")
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().ends_with(&suffix))
        .unwrap_or_else(|| panic!("no {event} hook ran"));
    let said = fs::read_to_string(found.path()).expect("reads");
    if said.trim().is_empty() {
        return String::new();
    }
    let said: serde_json::Value = serde_json::from_str(&said).unwrap_or_else(|_| panic!("{event} said JSON: {said}"));
    said["hookSpecificOutput"]["additionalContext"].as_str().unwrap_or_default().to_owned()
}

/// The short thread id that follows `before` in `text`.
fn id_after(text: &str, before: &str) -> String {
    let rest = text.split(before).nth(1).unwrap_or_else(|| panic!("{before:?} in {text}"));
    rest.chars().take_while(char::is_ascii_hexdigit).collect()
}

/// Calls one of Silt's MCP tools as `thread`'s agent does.
fn tool(machine: &Machine, thread: &str, envs: &[(&str, &str)], name: &str, arguments: serde_json::Value) -> String {
    let mut mcp = machine
        .command(&["mcp", "--thread", thread], envs)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("silt mcp runs");
    {
        let mut stdin = mcp.stdin.take().expect("stdin");
        let call = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
        writeln!(stdin, "{call}").expect("write");
    }
    let output = String::from_utf8_lossy(&mcp.wait_with_output().expect("mcp exits").stdout).into_owned();
    let reply: serde_json::Value = serde_json::from_str(output.lines().next().unwrap_or_default()).expect("a JSON-RPC reply");
    reply["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{reply}")).to_owned()
}

/// A starts and is held in its tool; B starts in the same clone, hears of
/// A, and messages it; A hears of B after its tool call. Once A ends, B no
/// longer lists it; an agent in its own checkout is told apart.
fn agents_hear_of_each_other_and_message_each_other(daemon: bool) {
    let world = world(daemon);
    let laptop = &world.laptop;
    let base: Vec<(&str, &str)> = if daemon { Vec::new() } else { vec![("SILT_DAEMONLESS_CAPTURE", "1")] };
    let (logs_a, logs_b, logs_c) =
        (world.root.path().join("hooks-a"), world.root.path().join("hooks-b"), world.root.path().join("hooks-c"));
    for logs in [&logs_a, &logs_b, &logs_c] {
        fs::create_dir_all(logs).expect("hook log dir");
    }
    let logged = |logs: &Path| -> Vec<(String, String)> {
        let log = ("FAKE_HOOK_LOG".to_owned(), logs.to_string_lossy().into_owned());
        base.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).chain([log]).collect()
    };
    let (envs_a, envs_b, envs_c) = (logged(&logs_a), logged(&logs_b), logged(&logs_c));
    let (marker_a, marker_b, marker_c) =
        (world.root.path().join("in-tool-a"), world.root.path().join("in-tool-b"), world.root.path().join("in-tool-c"));

    let a = in_tool(laptop, &["claude"], &marker_a, &refs(&envs_a));
    assert!(
        !logs_a.read_dir().expect("log").flatten().any(|entry| {
            entry.file_name().to_string_lossy().ends_with("-SessionStart") && entry.metadata().is_ok_and(|meta| meta.len() > 0)
        }),
        "alone in the project, A hears nothing"
    );
    let b = in_tool(laptop, &["claude"], &marker_b, &refs(&envs_b));
    let greeting = hook_said(&logs_b, "SessionStart");
    assert!(greeting.contains("other agents are working in this project now"), "{greeting}");
    assert!(greeting.contains("(claude, in your working tree): working now"), "{greeting}");
    assert!(greeting.contains("Asked to: \"Please add a farewell line to main\""), "{greeting}");
    let (thread_a, thread_b) = (id_after(&greeting, "\n- thread "), id_after(&greeting, "Your thread is "));
    assert_ne!(thread_a, thread_b);

    let listed = tool(laptop, &thread_b, &base, "silt_agents", serde_json::json!({}));
    assert!(listed.contains(&format!("thread {thread_a} (claude, in your working tree)")), "{listed}");
    let sent = tool(
        laptop,
        &thread_b,
        &base,
        "silt_message",
        serde_json::json!({"to": thread_a, "text": "I'm adding a farewell to main.rs too"}),
    );
    assert!(sent.starts_with(&format!("Sent to thread {thread_a}")) && sent.contains("after its next tool call"), "{sent}");
    let refused = tool(laptop, &thread_b, &base, "silt_message", serde_json::json!({"to": thread_b, "text": "me"}));
    assert!(refused.contains("your own thread"), "{refused}");

    let_go(&marker_a);
    let a = finished(a);
    assert!(String::from_utf8_lossy(&a.stderr).contains(&thread_a), "A's own thread");
    let heard = hook_said(&logs_a, "PostToolUse");
    assert!(
        heard.contains(&format!(
            "another agent started working in this project: thread {thread_b} (claude, in your working tree)"
        )),
        "{heard}"
    );
    assert!(heard.contains(&format!("a message from the agent in thread {thread_b}")), "{heard}");
    assert!(heard.contains("not your user") && heard.contains("I'm adding a farewell to main.rs too"), "{heard}");
    assert!(hook_said(&logs_a, "Stop").is_empty(), "handed over once");

    let alone = tool(laptop, &thread_b, &base, "silt_agents", serde_json::json!({}));
    assert_eq!(alone, "No other agent is working in this project now.", "A ended");

    let c = in_tool(laptop, &["claude", "--isolated"], &marker_c, &refs(&envs_c));
    let_go(&marker_b);
    finished(b);
    let arrived = hook_said(&logs_b, "PostToolUse");
    assert!(arrived.contains("(claude, in its own checkout `.silt/checkouts/"), "{arrived}");
    let_go(&marker_c);
    finished(c);

    if daemon {
        let in_flight = |found: &String| found.contains(&thread_b) && !found.contains(&thread_a);
        assert!(
            eventually(|| in_flight(&tool(
                laptop,
                &thread_a,
                &base,
                "silt_in_flight",
                serde_json::json!({"path": "src/main.rs"})
            ))),
            "A's in-flight lists B, not A"
        );
    }
}

#[test]
fn agents_hear_of_each_other_and_message_each_other_with_a_daemon() {
    agents_hear_of_each_other_and_message_each_other(true);
}

#[test]
fn agents_hear_of_each_other_and_message_each_other_without_a_daemon() {
    agents_hear_of_each_other_and_message_each_other(false);
}
