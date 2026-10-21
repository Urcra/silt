//! Drives the real `silt` binary with a scripted stand-in for Claude Code
//! that writes its transcript where Claude would, fires the hooks and edits
//! a file inside a tool call.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

const FAKE_CLAUDE: &str = r#"#!/bin/sh
set -eu
session=""
settings=""
while [ $# -gt 0 ]; do
    case "$1" in
        --session-id) session="$2"; shift 2 ;;
        --settings) settings="$2"; shift 2 ;;
        --mcp-config) printf '%s' "$2" > "$CLAUDE_CONFIG_DIR/mcp-config.json"; shift 2 ;;
        *) shift ;;
    esac
done
slug=$(pwd | sed 's/[^A-Za-z0-9]/-/g')
dir="$CLAUDE_CONFIG_DIR/projects/$slug"
mkdir -p "$dir"
transcript="$dir/$session.jsonl"
# Like Claude Code, runs the command the settings give for the event, and
# without settings no hook at all.
hook() {
    command=$(printf '%s' "$settings" | sed -n "s/.*\"$1\":\[[^]]*\"command\":\"\([^\"]*\)\".*/\1/p")
    [ -n "$settings" ] || return 0
    [ -n "$command" ] || { echo "no $1 hook in the settings" >&2; exit 8; }
    printf '{"session_id":"%s","hook_event_name":"%s","tool_name":"Edit","tool_use_id":"toolu_1"}' "$session" "$1" | sh -c "$command"
}
printf '%s\n' '{"type":"ai-title","aiTitle":"Add a greeting","sessionId":"x"}' >> "$transcript"
printf '%s\n' '{"type":"user","message":{"role":"user","content":"Please add a farewell line to main"}}' >> "$transcript"
hook UserPromptSubmit
sleep "${FAKE_PAUSE:-0}"
printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Adding it now."},{"type":"tool_use","id":"toolu_1","name":"Edit","input":{"file_path":"src/main.rs"}}]}}' >> "$transcript"
hook PreToolUse
printf 'fn farewell() {}\n' >> src/main.rs
hook PostToolUse
printf '%s\n' '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"edited","is_error":false}]}}' >> "$transcript"
printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Done: farewell added."}]}}' >> "$transcript"
printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Decision: farewell stays in main.rs instead of its own module, to keep the binary single-file."}]}}' >> "$transcript"
printf '%s\n' '{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}' >> "$transcript"
printf '%s\n' '{"type":"user","isCompactSummary":true,"message":{"role":"user","content":"Summary: farewell was added to main."}}' >> "$transcript"
hook Stop
"#;

/// A Claude Code session of two turns with tools of every kind Silt treats
/// apart, and an edit by hand between the turns.
const FAKE_CLAUDE_TOOLS: &str = r#"#!/bin/sh
set -eu
session=""
settings=""
while [ $# -gt 0 ]; do
    case "$1" in
        --session-id) session="$2"; shift 2 ;;
        --settings) settings="$2"; shift 2 ;;
        *) shift ;;
    esac
done
dir="$CLAUDE_CONFIG_DIR/projects/$(pwd | sed 's/[^A-Za-z0-9]/-/g')"
mkdir -p "$dir"
transcript="$dir/$session.jsonl"
hook() {
    command=$(printf '%s' "$settings" | sed -n "s/.*\"$1\":\[[^]]*\"command\":\"\([^\"]*\)\".*/\1/p")
    printf '{"session_id":"%s","hook_event_name":"%s","tool_name":"%s","tool_use_id":"%s","tool_input":%s}' \
        "$session" "$1" "${2:-}" "${3:-}" "${4:-null}" | sh -c "$command"
}
say() { printf '%s\n' "$1" >> "$transcript"; }
prompt() { say "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"$1\"}}"; hook UserPromptSubmit; }
reply() { say "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"$1\"}]}}"; }
start() {
    say "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"id\":\"$1\",\"name\":\"$2\",\"input\":$3}]}}"
    hook PreToolUse "$2" "$1" "$3"
}
finish() {
    hook PostToolUse "$2" "$1" "$3"
    say "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"$1\",\"content\":\"ok\",\"is_error\":false}]}}"
}

prompt "Add a farewell and a helper"
start toolu_1 Edit '{"file_path":"src/main.rs"}'
printf 'fn farewell() {}\n' >> src/main.rs
finish toolu_1 Edit '{"file_path":"src/main.rs"}'
start toolu_2 Read '{"file_path":"src/main.rs"}'
finish toolu_2 Read '{"file_path":"src/main.rs"}'
start toolu_3 Bash '{"command":"make helper"}'
printf 'pub fn help() {}\n' > src/helper.rs
finish toolu_3 Bash '{"command":"make helper"}'
reply "Added both."
hook Stop
printf '// by hand\n' >> src/helper.rs
prompt "Now a note"
start toolu_4 Write '{"file_path":"'"$(pwd)"'/NOTES.md"}'
printf '# Notes\n' > NOTES.md
finish toolu_4 Write '{"file_path":"'"$(pwd)"'/NOTES.md"}'
reply "Noted."
hook Stop
"#;

struct Env {
    _dirs: Vec<tempfile::TempDir>,
    silt_home: PathBuf,
    claude_dir: PathBuf,
    repo: PathBuf,
    fake: PathBuf,
}

impl Env {
    fn new() -> Self {
        Self::with_claude(FAKE_CLAUDE)
    }

    fn with_claude(script: &str) -> Self {
        let silt_home = tempfile::Builder::new().prefix("silt").tempdir_in("/tmp").expect("temp dir");
        let claude_dir = tempfile::tempdir().expect("temp dir");
        let repo = tempfile::tempdir().expect("temp dir");
        let bin = tempfile::tempdir().expect("temp dir");
        git(repo.path(), &["init", "-q", "-b", "main"]);
        fs::create_dir(repo.path().join("src")).expect("mkdir");
        fs::write(repo.path().join("src/main.rs"), "fn main() {}\n").expect("write");
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "init"]);
        let fake = bin.path().join("claude");
        fs::write(&fake, script).expect("write fake claude");
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod");
        Self {
            silt_home: silt_home.path().to_owned(),
            claude_dir: claude_dir.path().to_owned(),
            repo: repo.path().canonicalize().expect("canonical"),
            fake,
            _dirs: vec![silt_home, claude_dir, repo, bin],
        }
    }

    fn silt(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(output.status.success(), "silt {args:?} failed: {}", String::from_utf8_lossy(&output.stderr));
        output
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with(args, &[])
    }

    fn run_with(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_silt"))
            .args(args)
            .current_dir(&self.repo)
            .env("SILT_HOME", &self.silt_home)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("SILT_CLAUDE_BIN", &self.fake)
            .envs(envs.iter().copied())
            .output()
            .expect("silt runs")
    }

    fn stdout(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&self.silt(args).stdout).into_owned()
    }

    fn thread(&self) -> String {
        let log = self.stdout(&["log"]);
        log.split_whitespace().next().expect("a thread is listed").to_owned()
    }

    fn transcript(&self) -> PathBuf {
        let projects = self.claude_dir.join("projects");
        let project = fs::read_dir(&projects).expect("projects").flatten().next().expect("project").path();
        fs::read_dir(project)
            .expect("project dir")
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|extension| extension == "jsonl"))
            .expect("transcript")
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_silt")).args(["daemon", "stop"]).env("SILT_HOME", &self.silt_home).output();
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(args)
        .status()
        .expect("git");
    assert!(status.success());
}

/// Whether `check` holds within ten seconds. A check that panics, as a
/// command that fails while the daemon catches up does, counts as not yet.
fn eventually(mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut check)).unwrap_or(false) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn records_a_session_with_attributed_edits_and_survives_a_restart() {
    let env = Env::new();
    env.silt(&["claude"]);

    let thread = env.thread();
    let log = env.stdout(&["log"]);
    assert!(
        log.contains("Add a greeting"),
        "{log}\n{}\n{}",
        env.stdout(&["show", &thread]),
        fs::read_to_string(env.silt_home.join("daemon.log")).unwrap_or_default()
    );
    assert!(
        eventually(|| env.stdout(&["show", &thread]).contains("src/main.rs +17 -0")),
        "the tool call's edit is attributed to it:\n{}",
        env.stdout(&["show", &thread])
    );
    let show = env.stdout(&["show", &thread]);
    assert!(show.contains("> Please add a farewell line to main"), "{show}");
    assert!(show.contains("⚙ Edit"), "{show}");
    assert!(show.contains("Done: farewell added."), "{show}");
    let compacted = show.find("context compacted: Summary: farewell was added").expect("compaction is shown");
    assert!(show.find("> Please add a farewell line").expect("prompt") < compacted, "history before compaction stays");
    assert!(env.stdout(&["diff", &thread]).contains("+fn farewell() {}"));
    assert!(env.stdout(&["diff", &thread, "--turn", "1"]).contains("+fn farewell() {}"));
    assert!(env.stdout(&["search", "farewell"]).contains(&thread));

    env.silt(&["daemon", "stop"]);
    fs::write(env.repo.join("src/main.rs"), "fn main() {}\nfn farewell() {}\nfn later() {}\n").expect("edit while down");
    fs::write(
        env.transcript(),
        fs::read_to_string(env.transcript()).expect("read")
            + "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"one more thing while silt was down\"}}\n",
    )
    .expect("append while down");

    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn later() {}")));
    assert!(eventually(|| env.stdout(&["show", &thread]).contains("one more thing while silt was down")));
    let show = env.stdout(&["show", &thread]);
    assert!(show.contains("outside tool calls: src/main.rs +14 -0"), "{show}");
}

#[test]
fn hooks_capturing_in_their_own_process_record_what_the_daemon_would() {
    let env = Env::new();
    let ran = env.run_with(&["claude"], &[("SILT_DAEMONLESS_CAPTURE", "1")]);
    assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
    assert!(!String::from_utf8_lossy(&ran.stderr).contains("could not record"), "{}", String::from_utf8_lossy(&ran.stderr));

    let thread = env.thread();
    assert!(
        eventually(|| env.stdout(&["show", &thread]).contains("src/main.rs +17 -0")),
        "the tool call's edit is attributed to it:\n{}",
        env.stdout(&["show", &thread])
    );
    let show = env.stdout(&["show", &thread]);
    assert!(show.contains("> Please add a farewell line to main") && show.contains("Done: farewell added."), "{show}");
    assert!(!show.contains("outside tool calls"), "{show}");
    assert!(env.stdout(&["diff", &thread, "--turn", "1"]).contains("+fn farewell() {}"));
}

/// What a recorded session projects to, as `silt show` and `silt diff` print
/// it, with what differs between two machines (paths, thread ids) left out.
fn projection(env: &Env, envs: &[(&str, &str)]) -> String {
    let ran = env.run_with(&["claude"], envs);
    assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
    let thread = env.thread();
    let mut printed = String::new();
    for args in [&["show"][..], &["diff"], &["diff", "--turn", "1"], &["diff", "--turn", "2"]] {
        let mut args = args.to_vec();
        args.insert(1, &thread);
        printed += &format!("$ silt {}\n{}\n", args.join(" "), env.stdout(&args));
    }
    printed.replace(&env.repo.to_string_lossy().into_owned(), "<repo>").replace(&thread, "<thread>")
}

#[test]
fn a_session_recorded_without_the_daemon_projects_as_one_recorded_by_it() {
    let with_daemon = Env::with_claude(FAKE_CLAUDE_TOOLS);
    with_daemon.silt(&["daemon", "start"]);
    let recorded_by_daemon = projection(&with_daemon, &[]);
    let without = Env::with_claude(FAKE_CLAUDE_TOOLS);
    let recorded_alone = projection(&without, &[("SILT_DAEMONLESS_CAPTURE", "1")]);

    assert_eq!(recorded_alone, recorded_by_daemon);
    for (tool, file) in [("toolu_1", "src/main.rs"), ("toolu_3", "src/helper.rs"), ("toolu_4", "NOTES.md")] {
        assert!(recorded_alone.contains(&format!("✎ {file}")), "{tool}'s edit of {file} is credited to it:\n{recorded_alone}");
    }
    assert!(recorded_alone.contains("outside tool calls: src/helper.rs +1"), "the edit by hand:\n{recorded_alone}");
}

#[test]
fn the_agent_starts_without_waiting_for_the_daemon_which_takes_over_once_it_answers() {
    let env = Env::new();
    let ran = env.run_with(&["claude"], &[("FAKE_PAUSE", "5")]);
    assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
    assert!(env.stdout(&["daemon", "status"]).starts_with("running"), "the wrapper started the daemon");
    let log = fs::read_to_string(env.silt_home.join("daemon.log")).expect("daemon log");
    assert!(log.contains("took over recording"), "the daemon took the session over mid-turn: {log}");

    let thread = env.thread();
    let show = env.stdout(&["show", &thread]);
    assert!(show.contains("src/main.rs +17 -0") && !show.contains("outside tool calls"), "{show}");
    assert!(show.contains("Done: farewell added."), "{show}");
}

#[test]
fn isolated_sessions_work_in_a_managed_checkout() {
    let env = Env::new();
    env.silt(&["claude", "--isolated"]);
    let thread = env.thread();
    let checkout = env.repo.join(".silt/checkouts").join(
        fs::read_dir(env.repo.join(".silt/checkouts")).expect("checkouts").flatten().next().expect("one checkout").file_name(),
    );
    assert!(fs::read_to_string(checkout.join("src/main.rs")).expect("read").contains("fn farewell"));
    assert_eq!(fs::read_to_string(env.repo.join("src/main.rs")).expect("read"), "fn main() {}\n");
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
}

#[test]
fn track_attaches_to_a_session_started_without_silt() {
    let env = Env::new();
    let mut fake = Command::new(&env.fake);
    fake.args(["--session-id", "11111111-2222-4333-8444-555555555555"])
        .current_dir(&env.repo)
        .env("CLAUDE_CONFIG_DIR", &env.claude_dir)
        .env("SILT_HOME", &env.silt_home);
    // Another test forking while this script was being written can hold it
    // open for a moment.
    let output = (0..50)
        .find_map(|_| match fake.output() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(20));
                None
            }
            other => Some(other),
        })
        .expect("the script was released")
        .expect("fake claude runs");
    assert!(output.status.success());

    assert!(env.stdout(&["track"]).contains("tracking session"));
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["show", &thread]).contains("> Please add a farewell line to main")));
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
}

#[test]
fn a_renamed_file_keeps_its_history() {
    let env = Env::new();
    env.silt(&["claude"]);
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
    fs::rename(env.repo.join("src/main.rs"), env.repo.join("src/app.rs")).expect("rename");
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("rename to src/app.rs")));
    let diff = env.stdout(&["diff", &thread]);
    assert!(diff.contains("rename from src/main.rs"), "{diff}");
    assert!(diff.contains("+++ b/src/app.rs"), "{diff}");
    assert!(!diff.contains("/dev/null"), "a rename, not a delete and a create: {diff}");
}

/// Talks to the MCP server Silt configured for the session, the way
/// Claude Code would.
struct McpClient {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl McpClient {
    fn start(env: &Env) -> Self {
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(env.claude_dir.join("mcp-config.json")).expect("mcp config")).expect("json");
        let server = &config["mcpServers"]["silt"];
        let args: Vec<String> =
            server["args"].as_array().expect("args").iter().map(|arg| arg.as_str().expect("string").to_owned()).collect();
        let mut child = Command::new(server["command"].as_str().expect("command"))
            .args(args)
            .current_dir(&env.repo)
            .env("SILT_HOME", &env.silt_home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("mcp server starts");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = std::io::BufReader::new(child.stdout.take().expect("stdout"));
        let mut client = Self { child, stdin, stdout, next_id: 0 };
        client.request("initialize", serde_json::json!({"protocolVersion": "2025-06-18", "capabilities": {}}));
        client
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        use std::io::{BufRead, Write};
        self.next_id += 1;
        let message = serde_json::json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params});
        writeln!(self.stdin, "{message}").expect("send");
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("reply");
        serde_json::from_str(&line).expect("json reply")
    }

    fn tool(&mut self, name: &str, arguments: serde_json::Value) -> String {
        let reply = self.request("tools/call", serde_json::json!({"name": name, "arguments": arguments}));
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        reply["result"]["content"][0]["text"].as_str().expect("text").to_owned()
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn an_agent_reads_what_was_decided_before_compaction() {
    let env = Env::new();
    env.silt(&["claude"]);
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["show", &thread]).contains("context compacted")));

    let mut mcp = McpClient::start(&env);
    let found = mcp.tool("silt_history_search", serde_json::json!({"query": "farewell module"}));
    let index: usize = found
        .lines()
        .find(|line| line.contains("[module]"))
        .and_then(|line| line.strip_prefix('['))
        .and_then(|line| line.split(']').next())
        .and_then(|index| index.parse().ok())
        .unwrap_or_else(|| panic!("the decision is found: {found}"));
    let page = mcp.tool("silt_history_read", serde_json::json!({"offset": index, "limit": 3}));
    assert!(page.contains("farewell stays in main.rs instead of its own module"), "{page}");
    let whole = mcp.tool("silt_history_read", serde_json::json!({}));
    let decision = whole.find("Decision:").expect("decision listed");
    let compaction = whole.find("compaction").expect("compaction listed");
    assert!(decision < compaction, "the decision came before the compaction: {whole}");

    let history = mcp.tool("silt_file_history", serde_json::json!({"path": "src/main.rs"}));
    assert!(history.contains("tool call toolu_1") && history.contains("Please add a farewell line"), "{history}");

    let mut hook = Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(["hook", "SessionStart"])
        .env("SILT_HOME", &env.silt_home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("hook runs");
    let session = env.transcript().file_stem().map(|stem| stem.to_string_lossy().into_owned()).expect("session id");
    std::io::Write::write_all(
        &mut hook.stdin.take().expect("stdin"),
        format!(r#"{{"session_id":"{session}","hook_event_name":"SessionStart","source":"compact"}}"#).as_bytes(),
    )
    .expect("hook input");
    let output = String::from_utf8_lossy(&hook.wait_with_output().expect("hook ends").stdout).into_owned();
    assert!(output.contains("silt_history_search") && output.contains("additionalContext"), "{output}");
}

#[test]
fn diffs_start_from_the_base_asked_for() {
    let env = Env::new();
    let first = String::from_utf8_lossy(
        &Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&env.repo).output().expect("git runs").stdout,
    )
    .trim()
    .to_owned();
    env.silt(&["claude"]);
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
    git(&env.repo, &["commit", "-qam", "farewell"]);
    assert!(eventually(|| !env.stdout(&["diff", &thread, "--base", "last-commit"]).contains("farewell")));
    let main = env.repo.join("src/main.rs");
    fs::write(&main, fs::read_to_string(&main).expect("read") + "fn later() {}\n").expect("write");
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn later() {}")));

    let since_commit = env.stdout(&["diff", &thread, "--base", "last-commit"]);
    assert!(since_commit.contains("+fn later() {}") && !since_commit.contains("+fn farewell"), "{since_commit}");
    let since_first = env.stdout(&["diff", &thread, "--base", &format!("commit:{}", &first[..8])]);
    assert!(since_first.contains("+fn later() {}") && since_first.contains("+fn farewell() {}"), "{since_first}");
    let last_turn = env.stdout(&["diff", &thread, "--base", "last-turn"]);
    assert!(last_turn.contains("+fn farewell() {}"), "{last_turn}");
    let refused = env.run(&["diff", &thread, "--base", "commit:0000000"]);
    assert!(String::from_utf8_lossy(&refused.stderr).contains("never was at a commit"));
}

#[test]
fn uncommitted_changes_are_where_a_thread_starts_not_its_edits() {
    let env = Env::new();
    fs::write(env.repo.join("notes.md"), "draft\n").expect("write");
    env.silt(&["claude"]);
    let thread = env.thread();
    let show = env.stdout(&["show", &thread]);
    assert!(!show.contains("notes.md"), "{show}");
    let diff = env.stdout(&["diff", &thread]);
    assert!(diff.contains("+fn farewell() {}") && !diff.contains("draft"), "{diff}");
}
