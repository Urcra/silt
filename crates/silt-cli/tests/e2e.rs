//! Drives the real `silt` binary with a scripted stand-in for Claude Code
//! that writes its transcript where Claude would, fires the hooks and edits
//! a file inside a tool call.

use std::fs;
use std::io::Write;
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

#[test]
fn the_web_ui_reads_a_diff_a_page_of_files_at_a_time() {
    let env = Env::with_claude(FAKE_CLAUDE_TOOLS);
    env.silt(&["claude"]);
    let thread = env.thread();
    assert!(
        eventually(|| env.stdout(&["diff", &thread]).matches("diff --git").count() == 3),
        "{}",
        env.stdout(&["diff", &thread])
    );
    let url = env.stdout(&["open", "--no-browser"]);
    let (base, token) = url.trim().split_once("/#token=").expect("a URL with a token");
    let diff = |query: &str| -> String {
        let output = Command::new("curl")
            .args(["-s", "-H"])
            .arg(format!("Authorization: Bearer {token}"))
            .arg(format!("{base}/api/threads/{thread}/diff{query}"))
            .output()
            .expect("curl runs");
        serde_json::from_slice(&output.stdout).expect("a diff")
    };
    let (first, rest, whole) = (diff("?offset=0&limit=2"), diff("?offset=2&limit=2"), diff(""));
    assert_eq!(first.matches("diff --git").count(), 2);
    assert_eq!(rest.matches("diff --git").count(), 1);
    assert_eq!(first + &rest, whole, "the pages make up the whole diff, in path order");
}

#[test]
fn a_hook_that_finds_no_daemon_starts_one_for_the_hooks_after_it() {
    let env = Env::new();
    env.silt(&["claude"]);
    env.silt(&["daemon", "stop"]);
    let session = env.transcript().file_stem().map(|stem| stem.to_string_lossy().into_owned()).expect("session id");
    let hook = |envs: &[(&str, &str)]| {
        let mut hook = Command::new(env!("CARGO_BIN_EXE_silt"))
            .args(["hook", "UserPromptSubmit"])
            .current_dir(&env.repo)
            .env("SILT_HOME", &env.silt_home)
            .env("CLAUDE_CONFIG_DIR", &env.claude_dir)
            .envs(envs.iter().copied())
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("hook runs");
        std::io::Write::write_all(
            &mut hook.stdin.take().expect("stdin"),
            format!(r#"{{"session_id":"{session}","hook_event_name":"UserPromptSubmit"}}"#).as_bytes(),
        )
        .expect("hook input");
        assert!(hook.wait().expect("hook ends").success());
    };
    let running = || env.stdout(&["daemon", "status"]).starts_with("running");

    hook(&[("SILT_DAEMONLESS_CAPTURE", "1")]);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!running(), "capture in process, as asked for, starts no daemon");
    hook(&[]);
    assert!(eventually(running), "the hook started the daemon");
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

/// The web UI's API, as a browser on this machine calls it.
struct Api {
    base: String,
    token: String,
}

impl Api {
    fn open(env: &Env) -> Self {
        let url = env.stdout(&["open", "--no-browser"]);
        let (base, token) = url.trim().split_once("/#token=").expect("a URL with a token");
        Self { base: base.to_owned(), token: token.to_owned() }
    }

    fn call(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> (bool, serde_json::Value) {
        let mut command = Command::new("curl");
        command
            .args(["-s", "-w", "\n%{http_code}", "-X", method, "-H", "Content-Type: application/json", "-H"])
            .arg(format!("Authorization: Bearer {}", self.token));
        if let Some(body) = body {
            command.arg("--data").arg(body.to_string());
        }
        let output = command.arg(format!("{}{path}", self.base)).output().expect("curl runs");
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let (json, status) = text.rsplit_once('\n').expect("a status line");
        (status == "200", serde_json::from_str(json).unwrap_or(serde_json::Value::Null))
    }

    fn ok(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> serde_json::Value {
        let (ok, answer) = self.call(method, path, body);
        assert!(ok, "{method} {path}: {answer}");
        answer
    }
}

#[test]
fn the_web_ui_forks_offers_and_trashes_without_the_cli() {
    let env = Env::new();
    env.silt(&["claude"]);
    let api = Api::open(&env);
    let threads = api.ok("GET", "/api/threads", None);
    let thread = threads[0]["id"].as_str().expect("a thread").to_owned();

    let forked = api.ok("POST", &format!("/api/threads/{thread}/fork"), Some(serde_json::json!({})));
    let (fork, checkout) =
        (forked["thread"].as_str().expect("the fork").to_owned(), forked["checkout"].as_str().expect("its checkout"));
    let main = Path::new(checkout).join("src/main.rs");
    fs::write(&main, fs::read_to_string(&main).expect("read") + "fn forked() {}\n").expect("edit the fork");
    assert!(eventually(|| env.stdout(&["diff", &fork]).contains("+fn forked() {}")));
    let mr = api.ok("POST", &format!("/api/threads/{fork}/mrs"), Some(serde_json::json!({ "title": "Add forked" })));
    assert_eq!(mr["title"], "Add forked");
    assert!(api.ok("GET", "/api/mrs", None).to_string().contains("Add forked"));

    let in_flight = api.ok("GET", &format!("/api/threads/{thread}/in-flight?path=src/main.rs"), None);
    assert!(in_flight.to_string().contains(&fork), "the fork changes main.rs too: {in_flight}");
    let history = api.ok("GET", &format!("/api/threads/{thread}/files/history?path=src/main.rs"), None);
    assert!(history.as_array().is_some_and(|changes| !changes.is_empty()), "{history}");
    let hits = api.ok("GET", &format!("/api/threads/{thread}/search?q=farewell"), None);
    assert!(hits.as_array().is_some_and(|hits| !hits.is_empty()), "{hits}");

    api.ok("POST", &format!("/api/threads/{fork}/trash"), None);
    assert!(api.ok("GET", "/api/trash", None).to_string().contains(&fork));
    api.ok("POST", &format!("/api/threads/{fork}/restore"), None);
    assert!(!api.ok("GET", "/api/trash", None).to_string().contains(&fork));
    let (ok, answer) = api.call("POST", "/api/threads/nosuchthread/fork", Some(serde_json::json!({})));
    assert!(!ok && answer["error"].is_string(), "{answer}");
}

/// The next event on the socket that `wanted` accepts, within the wait.
async fn event_where(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    wanted: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    use futures_util::StreamExt;
    let found = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(Ok(message)) = socket.next().await {
            let Ok(text) = message.into_text() else { continue };
            let event: serde_json::Value = serde_json::from_str(&text).expect("events are JSON");
            if wanted(&event) {
                return event;
            }
        }
        panic!("the socket closed");
    });
    found.await.expect("the event came")
}

#[test]
fn browsers_hear_of_new_threads_and_of_changes_to_the_ones_they_watch() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let env = Env::new();
    let api = Api::open(&env);
    let url = format!("{}/api/events?token={}", api.base.replacen("http", "ws", 1), api.token);
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.expect("connects");
        socket.send(Message::text(r#"{"subscribe":["threads"]}"#)).await.expect("subscribes");
        let (silt, repo, home, claude_dir, fake) =
            (env!("CARGO_BIN_EXE_silt"), env.repo.clone(), env.silt_home.clone(), env.claude_dir.clone(), env.fake.clone());
        let recorded = tokio::task::spawn_blocking(move || {
            Command::new(silt)
                .arg("claude")
                .current_dir(repo)
                .env("SILT_HOME", home)
                .env("CLAUDE_CONFIG_DIR", claude_dir)
                .env("SILT_CLAUDE_BIN", fake)
                .output()
                .expect("silt runs")
        });
        event_where(&mut socket, |event| event["event"] == "threads").await;
        assert!(recorded.await.expect("joins").status.success());

        let thread = api.ok("GET", "/api/threads", None)[0]["id"].as_str().expect("a thread").to_owned();
        let watch = serde_json::json!({ "subscribe": [format!("thread:{thread}")] }).to_string();
        socket.send(Message::text(watch)).await.expect("subscribes");
        let comment = serde_json::json!({ "on": { "on": "entry", "index": 0 }, "text": "nice" });
        api.ok("POST", &format!("/api/threads/{thread}/comments"), Some(comment));
        let changed = event_where(&mut socket, |event| event["event"] == "thread").await;
        assert_eq!(changed["thread"], thread.as_str());
        assert!(changed["deltas"].as_u64().is_some_and(|deltas| deltas > 0), "{changed}");
    });
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

/// What a cloud session's `SessionStart` hook does, where Claude Code starts
/// without Silt: run again on resume, it records into the same thread, and
/// the thread names its cloud session.
#[test]
fn track_from_hook_records_the_hooks_session_once() {
    let env = Env::new();
    let session = "11111111-2222-4333-8444-777777777777";
    let ran = Command::new(&env.fake)
        .args(["--session-id", session])
        .current_dir(&env.repo)
        .env("CLAUDE_CONFIG_DIR", &env.claude_dir)
        .env("SILT_HOME", &env.silt_home)
        .output()
        .expect("fake claude runs");
    assert!(ran.status.success());

    let input = serde_json::json!({
        "session_id": session,
        "cwd": env.repo,
        "hook_event_name": "SessionStart",
        "source": "startup",
    });
    for _ in 0..2 {
        let mut hook = Command::new(env!("CARGO_BIN_EXE_silt"))
            .args(["track", "--from-hook"])
            .current_dir("/")
            .env("SILT_HOME", &env.silt_home)
            .env("CLAUDE_CONFIG_DIR", &env.claude_dir)
            .env("CLAUDE_CODE_REMOTE_SESSION_ID", "cse_01CloudSessionOfTheHook")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("silt runs");
        hook.stdin.take().expect("stdin").write_all(input.to_string().as_bytes()).expect("write the hook input");
        let output = hook.wait_with_output().expect("silt exits");
        assert!(output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("recording this session in thread"), "{stderr}");
    }

    assert_eq!(env.stdout(&["log"]).lines().count(), 1, "{}", env.stdout(&["log"]));
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["show", &thread]).contains("> Please add a farewell line to main")));
    let threads = Api::open(&env).ok("GET", "/api/threads", None);
    assert_eq!(threads[0]["cloud"], "session_01CloudSessionOfTheHook", "{threads}");
}

/// A session longer than one read is taken in whole, each line once.
#[test]
fn track_takes_in_a_long_session_whole() {
    let env = Env::new();
    let session = "11111111-2222-4333-8444-666666666666";
    let project =
        env.claude_dir.join("projects").join(env.repo.to_string_lossy().replace(|ch: char| !ch.is_ascii_alphanumeric(), "-"));
    fs::create_dir_all(&project).expect("mkdir");
    let padding = "x".repeat(1_000);
    let lines: String = (0..3_000)
        .map(|index| {
            let text = format!("prompt-{index}-end {padding}");
            format!("{}\n", serde_json::json!({ "type": "user", "message": { "role": "user", "content": text } }))
        })
        .collect();
    assert!(lines.len() > 2 << 20, "longer than one read");
    fs::write(project.join(format!("{session}.jsonl")), lines).expect("write the session");

    assert!(env.stdout(&["track"]).contains("tracking session"));
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["show", &thread]).contains("prompt-2999-end")));
    let show = env.stdout(&["show", &thread]);
    for index in [0, 1_499, 2_999] {
        assert_eq!(show.matches(&format!("prompt-{index}-end")).count(), 1, "prompt {index} is recorded once");
    }
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

/// A busy watcher may report a rename's two halves apart; a file gone and
/// its bytes back elsewhere moments later is still a move.
#[test]
fn a_file_gone_and_back_elsewhere_moments_later_is_a_rename() {
    let env = Env::new();
    env.silt(&["claude"]);
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
    let bytes = fs::read(env.repo.join("src/main.rs")).expect("reads");
    fs::remove_file(env.repo.join("src/main.rs")).expect("removes");
    std::thread::sleep(std::time::Duration::from_millis(300));
    fs::write(env.repo.join("src/app.rs"), bytes).expect("writes");
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("rename to src/app.rs")));
    let diff = env.stdout(&["diff", &thread]);
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
fn a_commit_is_found_with_the_thread_and_turn_that_made_it() {
    let env = Env::new();
    env.silt(&["claude"]);
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
    git(&env.repo, &["commit", "-qam", "add a farewell"]);
    let commit = String::from_utf8_lossy(
        &Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&env.repo).output().expect("git runs").stdout,
    )
    .trim()
    .to_owned();
    let full = env.stdout(&["history", &thread]);
    let full_id = full.split_whitespace().nth(1).expect("the thread's full id").trim_end_matches(':').to_owned();
    assert!(eventually(|| env.stdout(&["log", "--commit", &commit[..10]]).contains(&format!("{full_id} turn 1"))));
    assert!(env.stdout(&["log", "--commit", "abcdef12"]).contains("no thread here made"));
    let refused = env.run(&["log", "--commit", "xy"]);
    assert!(String::from_utf8_lossy(&refused.stderr).contains("four hex digits"));
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

    // Against a branch as git has it now, rather than a version of the thread.
    let against_main = env.stdout(&["diff", &thread, "--base", "main"]);
    assert!(against_main.contains("+fn later() {}") && !against_main.contains("+fn farewell"), "{against_main}");
    let against_first = env.stdout(&["diff", &thread, "--base", &format!("branch:{first}")]);
    assert!(against_first.contains("+fn later() {}") && against_first.contains("+fn farewell() {}"), "{against_first}");
    let nowhere = env.run(&["diff", &thread, "--base", "branch:nowhere"]);
    assert!(String::from_utf8_lossy(&nowhere.stderr).contains("names no commit"));
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
