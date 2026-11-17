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
    let personal_session = fs::read_dir(laptop.claude_dir.join("projects"))
        .expect("projects")
        .flatten()
        .flat_map(|project| fs::read_dir(project.path()).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "jsonl"))
        .and_then(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
        .expect("the personal session");

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

    // A plugin on in Work: its runs' calls are kept with their thread (ADR 0093).
    let plugin = root.path().join("fleet.json");
    let definition = serde_json::json!({
        "name": "fleet",
        "kind": {"type": "command", "argv": ["echo", "ran on {target}"]},
        "environments": {"lab": {"target": "car-*.fleet.test", "changes": "ask"}}
    });
    fs::write(&plugin, definition.to_string()).expect("writes the plugin");
    assert!(laptop.silt(&["plugin", "add", &plugin.to_string_lossy()]).status.success());
    assert!(laptop.silt(&["plugin", "enable", "fleet", "--workspace", "work"]).status.success());
    let ran = in_work(&["claude"]);
    let said = String::from_utf8_lossy(&ran.stderr).into_owned();
    assert!(ran.status.success(), "{said}");
    let settings = fs::read_to_string(laptop.agents.join("logs/claude-settings")).expect("Claude got settings");
    let settings: serde_json::Value = serde_json::from_str(&settings).expect("json");
    assert_eq!(settings["permissions"]["disableBypassPermissionsMode"], "disable", "{settings}");
    assert!(!said.contains("sandboxed: "), "hiding other workspaces asks only for the agent's own sandbox: {said}");
    assert_eq!(settings["sandbox"]["enabled"], true, "{settings}");
    assert_eq!(settings["sandbox"]["failIfUnavailable"], true, "{settings}");
    let denied = settings["sandbox"]["filesystem"]["denyRead"].to_string();
    let silt_home = laptop.silt_home.to_string_lossy().replace('\\', "/");
    assert!(denied.contains(&silt_home), "Silt's data is out of the sandbox's reach: {denied}");
    assert!(settings["hooks"]["Stop"].is_array(), "the hooks are still there: {settings}");

    laptop.silt(&["workspace", "policy", "Work", "--silt-sandbox", "on"]);
    // The stand-in's log folder, given to agents as a person gives one a
    // variable: Silt's sandbox passes the shell's others by an allowlist.
    fs::write(laptop.silt_home.join("agent-env"), "FAKE_LOG_DIR\n").expect("writes agent-env");
    let ran = in_work(&["claude"]);
    let said = String::from_utf8_lossy(&ran.stderr).into_owned();
    if ran.status.success() {
        // Silt's own sandbox is the wall (ADR 0093); the agent's own isn't forced on inside it.
        assert!(said.contains("sandboxed: no network"), "{said}");
        let settings = fs::read_to_string(laptop.agents.join("logs/claude-settings")).expect("Claude got settings");
        let settings: serde_json::Value = serde_json::from_str(&settings).expect("json");
        assert!(settings["sandbox"].is_null(), "{settings}");
    } else {
        assert!(said.contains("Work runs its agents in Silt's sandbox, which this machine can't make (your rules): "), "{said}");
    }
    laptop.silt(&["workspace", "policy", "Work", "--silt-sandbox", "off"]);

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

    // Nor can its run's channel send hooks for the personal thread's session (ADR 0093).
    #[cfg(unix)]
    {
        assert!(laptop.silt(&["daemon", "start"]).status.success());
        let mut hook = Command::new(env!("CARGO_BIN_EXE_silt"))
            .args(["hook", "UserPromptSubmit"])
            .env("SILT_HOME", &laptop.silt_home)
            .env("SILT_RUN_SOCKET", silt_daemon::paths::agents_socket_in(&laptop.silt_home))
            .env("SILT_RUN_TOKEN", &token)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("silt hook runs");
        let input =
            serde_json::json!({"session_id": personal_session, "hook_event_name": "UserPromptSubmit", "prompt": "from Work"});
        writeln!(hook.stdin.take().expect("stdin"), "{input}").expect("write");
        let said = String::from_utf8_lossy(&hook.wait_with_output().expect("hook exits").stderr).into_owned();
        assert!(said.contains("a run's hooks speak for its own thread's sessions only"), "{said}");
    }

    // A terminal run's token can't show the person anything, even through a server offering the tools (ADR 0068).
    let page = serde_json::json!({"title": "Page", "kind": "html", "content": "<p>hi</p>"});
    let shown = mcp_answer_with(&laptop, &["mcp", "--thread", work_id, "--token", &token, "--ui"], "silt_artifact", page.clone());
    assert!(shown.contains("only an agent the web UI runs"), "{shown}");
    let forged = mcp_answer_with(&laptop, &["mcp", "--thread", work_id, "--token", "not-a-token", "--ui"], "silt_artifact", page);
    assert!(forged.contains("this run's token isn't known here"), "{forged}");

    // The plugin on in Work waits for the person on a change, and its run hears how they answered (ADR 0093).
    let arguments = serde_json::json!({"environment": "lab", "target": "car-1.fleet.test", "args": ["now"]});
    let said = std::thread::scope(|scope| {
        let call = scope.spawn(|| mcp_answer(&laptop, work_id, &token, "fleet_run", arguments.clone()));
        let mut waiting = None;
        for _ in 0..200 {
            let listed = String::from_utf8_lossy(&laptop.silt(&["approvals"]).stdout).into_owned();
            if let Some(line) = listed.lines().find(|line| line.contains("fleet fleet_run")) {
                waiting = line.split_whitespace().next().map(str::to_owned);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let id = waiting.expect("the call waits for the person");
        assert!(laptop.silt(&["approve", &id, "--reject"]).status.success());
        call.join().expect("the call ends")
    });
    assert!(said.contains("Refused: the person rejected it"), "{said}");
    // Kept with the run's own thread.
    let run_thread = args.iter().skip_while(|arg| *arg != "--thread").nth(1).cloned().expect("the run's thread");
    let access = String::from_utf8_lossy(&laptop.silt(&["access", "show", &run_thread]).stdout).into_owned();
    assert!(access.contains("fleet fleet_run in lab  refused"), "{access}");
}
