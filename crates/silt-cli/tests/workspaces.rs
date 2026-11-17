//! Workspaces (ADR 0065): a work workspace's rules hold the agents started
//! in it, its threads keep to its own server, and its agents' history tools
//! don't read another workspace's threads.

mod common;

use std::fs;
use std::io::Write as _;
use std::process::{Command, Output, Stdio};

use common::{Machine, Server, canonical, git, origin};

fn threads(machine: &Machine) -> Vec<serde_json::Value> {
    machine.web("GET", "/api/threads", None).as_array().cloned().unwrap_or_default()
}

fn thread_in(machine: &Machine, workspace: &str) -> serde_json::Value {
    threads(machine)
        .into_iter()
        .find(|thread| thread["workspace"] == workspace)
        .unwrap_or_else(|| panic!("a thread in {workspace}: {:?}", threads(machine)))
}

/// What `silt mcp` answers a history tool called with `arguments`.
fn mcp_answer(machine: &Machine, thread: &str, token: &str, tool: &str, arguments: serde_json::Value) -> String {
    mcp_answer_with(machine, &["mcp", "--thread", thread, "--token", token], tool, arguments)
}

/// [`mcp_answer`] from `silt` run with `args`.
fn mcp_answer_with(machine: &Machine, args: &[&str], tool: &str, arguments: serde_json::Value) -> String {
    let mut mcp = Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(args)
        .env("SILT_HOME", &machine.silt_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("silt mcp runs");
    {
        let mut stdin = mcp.stdin.take().expect("stdin");
        let call = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": arguments}});
        writeln!(stdin, "{call}").expect("write");
    }
    String::from_utf8_lossy(&mcp.wait_with_output().expect("mcp exits").stdout).into_owned()
}

fn failed_with(output: &Output, said: &str) -> bool {
    !output.status.success() && String::from_utf8_lossy(&output.stderr).contains(said)
}

#[test]
fn a_work_workspace_holds_its_agents_to_its_rules_and_keeps_its_history_apart() {
    let root = common::temp_root("silt-workspaces");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let work = root.path().join("laptop/work");
    git(root.path(), &["clone", "-q", &origin.to_string_lossy(), &work.to_string_lossy()]);
    let work = canonical(&work);
    let work_text = work.to_string_lossy().into_owned();

    laptop.silt(&["workspace", "new", "Work", "--local", "--folder", &work_text]);
    laptop.silt(&[
        "workspace",
        "policy",
        "Work",
        "--agents",
        "claude,copilot",
        "--max-preset",
        "ask_before_commands",
        "--hide-other-workspaces",
        "on",
        // Where the stand-in agent keeps its logs, which Silt's sandbox would otherwise hide;
        // and no network, which it doesn't need and some test machines can't give a sandbox.
        "--network",
        "none",
        "--paths",
        &format!("{}:rw", laptop.agents.to_string_lossy()),
    ]);
    let listed = String::from_utf8_lossy(&laptop.silt(&["workspace", "list"]).stdout).into_owned();
    assert!(
        listed.contains("Work  [work]")
            && listed.contains("claude, copilot only · at most Ask before commands · other workspaces' files hidden"),
        "{listed}"
    );

    // A personal thread, in the default workspace, as before.
    laptop.silt(&["claude"]);

    let in_work = |args: &[&str]| laptop.command(args, &[]).current_dir(&work).output().expect("silt runs");
    let which = String::from_utf8_lossy(&in_work(&["workspace", "which"]).stdout).into_owned();
    assert!(which.starts_with("Work (by rule folder ") && which.contains("this machine only"), "{which}");
    assert!(
        failed_with(&in_work(&["codex"]), "Codex isn't allowed in Work: your rules allow claude, copilot"),
        "an agent the rules don't name is refused"
    );
    assert!(
        failed_with(
            &in_work(&["claude", "--dangerously-skip-permissions"]),
            "--dangerously-skip-permissions stops Claude Code asking before it acts, which Work doesn't allow"
        ),
        "skipping permissions is refused under a cap"
    );

    let ran = in_work(&["claude"]);
    let said = String::from_utf8_lossy(&ran.stderr).into_owned();
    assert!(ran.status.success(), "{said}");
    let settings = fs::read_to_string(laptop.agents.join("logs/claude-settings")).expect("Claude got settings");
    let settings: serde_json::Value = serde_json::from_str(&settings).expect("json");
    assert_eq!(settings["permissions"]["disableBypassPermissionsMode"], "disable", "{settings}");
    if said.contains("sandboxed: ") {
        // Silt's own sandbox is the wall (ADR 0093); the agent's own isn't forced on inside it.
        assert!(said.contains("sandboxed: no network"), "{said}");
        assert!(settings["sandbox"].is_null(), "{settings}");
    } else {
        assert_eq!(settings["sandbox"]["enabled"], true, "{settings}");
        assert_eq!(settings["sandbox"]["failIfUnavailable"], true, "{settings}");
        let denied = settings["sandbox"]["filesystem"]["denyRead"].to_string();
        let silt_home = laptop.silt_home.to_string_lossy().replace('\\', "/");
        assert!(denied.contains(&silt_home), "Silt's data is out of the sandbox's reach: {denied}");
    }
    assert!(settings["hooks"]["Stop"].is_array(), "the hooks are still there: {settings}");

    let work_thread = thread_in(&laptop, "work");
    let personal = thread_in(&laptop, "default");
    assert_eq!(work_thread["local_only"], true, "Work keeps its threads on this machine: {work_thread}");
    assert!(personal["server"].is_string(), "the default workspace syncs as before: {personal}");

    // The work agent's history tools, with its run's token, don't reach the personal thread.
    let config = fs::read_to_string(laptop.claude_dir.join("mcp-config.json")).expect("Claude got Silt's MCP server");
    let config: serde_json::Value = serde_json::from_str(&config).expect("json");
    let args: Vec<String> = config["mcpServers"]["silt"]["args"]
        .as_array()
        .expect("args")
        .iter()
        .filter_map(|arg| arg.as_str().map(str::to_owned))
        .collect();
    let token = args.iter().skip_while(|arg| *arg != "--token").nth(1).cloned().expect("a run token");
    let work_id = work_thread["id"].as_str().expect("id");
    let personal_id = personal["id"].as_str().expect("id");
    let own = mcp_answer(&laptop, work_id, &token, "silt_history_read", serde_json::json!({}));
    assert!(!own.contains("no thread matches"), "it reads its own thread: {own}");
    let other = mcp_answer(&laptop, work_id, &token, "silt_history_read", serde_json::json!({"thread": personal_id}));
    assert!(other.contains("no thread matches"), "another workspace's thread is out of reach: {other}");
    let forged = mcp_answer(&laptop, work_id, "not-a-token", "silt_history_read", serde_json::json!({}));
    assert!(forged.contains("this run's token isn't known here"), "{forged}");

    // A terminal run's token can't show the person anything, even through a server offering the tools (ADR 0068).
    let page = serde_json::json!({"title": "Page", "kind": "html", "content": "<p>hi</p>"});
    let shown = mcp_answer_with(&laptop, &["mcp", "--thread", work_id, "--token", &token, "--ui"], "silt_artifact", page.clone());
    assert!(shown.contains("only an agent the web UI runs"), "{shown}");
    let forged = mcp_answer_with(&laptop, &["mcp", "--thread", work_id, "--token", "not-a-token", "--ui"], "silt_artifact", page);
    assert!(forged.contains("this run's token isn't known here"), "{forged}");
}
