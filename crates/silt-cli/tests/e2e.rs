//! Drives the real `silt` binary with a scripted stand-in for Claude Code,
//! `silt-fake-claude`, that writes its transcript where Claude would, fires
//! the hooks and edits a file inside a tool call: its `short` script, or
//! `tools` for a session with tools of every kind.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// The stand-in for Claude Code, built beside `silt`.
fn fake_claude() -> PathBuf {
    let silt = Path::new(env!("CARGO_BIN_EXE_silt"));
    let fake = silt.with_file_name(format!("silt-fake-claude{}", std::env::consts::EXE_SUFFIX));
    assert!(fake.is_file(), "{} isn't built; run `cargo build -p silt-fake-agent`", fake.display());
    fake
}

struct Env {
    _dirs: Vec<tempfile::TempDir>,
    silt_home: PathBuf,
    claude_dir: PathBuf,
    repo: PathBuf,
    fake: PathBuf,
    /// The stand-in's `FAKE_SCRIPT`.
    script: &'static str,
    /// For a test's own stand-in.
    #[cfg_attr(not(unix), expect(dead_code, reason = "only the cloud test, which needs a Unix terminal, writes one"))]
    bin: PathBuf,
}

impl Env {
    fn new() -> Self {
        Self::with_claude("short")
    }

    fn with_claude(script: &'static str) -> Self {
        // Under `/tmp` on Unix, which keeps the daemon's socket path short.
        let mut builder = tempfile::Builder::new();
        builder.prefix("silt");
        let silt_home = if cfg!(unix) { builder.tempdir_in("/tmp") } else { builder.tempdir() }.expect("temp dir");
        let claude_dir = tempfile::tempdir().expect("temp dir");
        let repo = tempfile::tempdir().expect("temp dir");
        let bin = tempfile::tempdir().expect("temp dir");
        git(repo.path(), &["init", "-q", "-b", "main"]);
        fs::create_dir(repo.path().join("src")).expect("mkdir");
        fs::write(repo.path().join("src/main.rs"), "fn main() {}\n").expect("write");
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "init"]);
        let repo_path = silt_os::canonicalize(repo.path()).expect("canonical");
        Self {
            silt_home: silt_home.path().to_owned(),
            claude_dir: claude_dir.path().to_owned(),
            repo: repo_path,
            fake: fake_claude(),
            script,
            bin: bin.path().to_owned(),
            _dirs: vec![silt_home, claude_dir, repo, bin],
        }
    }

    /// The stand-in as a test runs it directly.
    fn claude(&self) -> Command {
        let mut command = Command::new(&self.fake);
        command.env("FAKE_SCRIPT", self.script);
        command
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
            .env("FAKE_SCRIPT", self.script)
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

    /// The managed checkout `silt claude --isolated` made, every edit in
    /// which is its thread's, an agent working there or not.
    fn isolated_checkout(&self) -> PathBuf {
        let checkouts = self.repo.join(".silt/checkouts");
        checkouts.join(fs::read_dir(&checkouts).expect("checkouts").flatten().next().expect("one checkout").file_name())
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
    fs::write(
        env.transcript(),
        fs::read_to_string(env.transcript()).expect("read")
            + "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"one more thing while silt was down\"}}\n",
    )
    .expect("append while down");
    assert!(eventually(|| env.stdout(&["show", &thread]).contains("one more thing while silt was down")));
}

#[test]
fn the_clone_is_the_person_s_again_once_its_agent_finishes_until_the_next_turn() {
    let env = Env::new();
    env.silt(&["claude"]);
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["show", &thread]).contains("src/main.rs +17 -0")));
    let before = env.stdout(&["diff", &thread]);

    fs::write(env.repo.join("src/main.rs"), "fn main() {}\nfn farewell() {}\nfn later() {}\n").expect("edit by hand");
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(env.stdout(&["diff", &thread]), before, "an edit after the session isn't the thread's");
    env.silt(&["daemon", "stop"]);
    env.silt(&["log"]);
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(env.stdout(&["diff", &thread]), before, "nor once a restarted daemon rescans the clone");

    let session = env.transcript().file_stem().expect("named").to_string_lossy().into_owned();
    let mut hook = Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(["hook", "UserPromptSubmit"])
        .current_dir(&env.repo)
        .env("SILT_HOME", &env.silt_home)
        .env("CLAUDE_CONFIG_DIR", &env.claude_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("silt runs");
    let input = format!(r#"{{"session_id":"{session}","hook_event_name":"UserPromptSubmit","prompt":"one more"}}"#);
    hook.stdin.take().expect("piped").write_all(input.as_bytes()).expect("hook input");
    assert!(hook.wait().expect("hook ran").success());
    assert!(
        eventually(|| env.stdout(&["diff", &thread]).contains("+fn later() {}")),
        "the next turn takes in what changed meanwhile:\n{}",
        env.stdout(&["show", &thread])
    );
    let show = env.stdout(&["show", &thread]);
    assert!(show.contains("outside tool calls: src/main.rs +14 -0"), "the person's, not the agent's: {show}");
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
    let env = Env::with_claude("tools");
    env.silt(&["claude"]);
    let thread = env.thread();
    assert!(
        eventually(|| env.stdout(&["diff", &thread]).matches("diff --git").count() == 3),
        "{}",
        env.stdout(&["diff", &thread])
    );
    let url = env.stdout(&["open", "--no-browser"]);
    let (base, token) = web_ui(&url);
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
    let repo = env.repo.to_string_lossy().into_owned();
    // Paths are also shown as JSON, where Windows' backslashes are doubled.
    printed.replace(&repo.replace('\\', "\\\\"), "<repo>").replace(&repo, "<repo>").replace(&thread, "<thread>")
}

#[test]
fn a_session_recorded_without_the_daemon_projects_as_one_recorded_by_it() {
    let with_daemon = Env::with_claude("tools");
    with_daemon.silt(&["daemon", "start"]);
    let recorded_by_daemon = projection(&with_daemon, &[]);
    let without = Env::with_claude("tools");
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
        let (base, token) = web_ui(&url);
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
        (status == "200", serde_json::from_str(json).unwrap_or_else(|_| serde_json::Value::String(text.clone())))
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

/// Starts `agent` of the scripted ones on `prompt`: `v1`, which speaks ACP
/// v1 as Copilot does, or `v2`, which speaks v2 as Claude's adapter does.
fn start_fake_acp(env: &Env, api: &Api, agent: &str, prompt: &str) -> String {
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([
        {"id": "v1", "name": "Fake v1", "command": fake},
        {"id": "v2", "name": "Fake v2", "command": fake, "env": {"FAKE_ACP_V2": "1"}}
    ]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started =
        api.ok("POST", "/api/agents/start", Some(serde_json::json!({"clone": env.repo, "agent": agent, "prompt": prompt})));
    started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("the thread").to_owned())
}

/// The thread's history as kind and text, once its agent is idle and the
/// history says `last`.
fn settled_history(api: &Api, thread: &str, last: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let settled = eventually(|| {
        let history = api.ok("GET", &format!("/api/threads/{thread}/history"), None);
        entries = history["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| {
                (entry["kind"].as_str().unwrap_or_default().to_owned(), entry["text"].as_str().unwrap_or_default().to_owned())
            })
            .collect();
        let idle = api.ok("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["state"] == "idle";
        idle && entries.iter().any(|(_, text)| text == last)
    });
    assert!(settled, "the agent never settled on {last:?}: {entries:?}");
    entries
}

fn said(entries: &[(String, String)]) -> Vec<(&str, &str)> {
    entries
        .iter()
        .filter(|(kind, _)| ["user", "wake", "assistant"].contains(&kind.as_str()))
        .map(|(kind, text)| (kind.as_str(), text.as_str()))
        .collect()
}

/// Claude goes on by itself once a background task it started ends: an
/// agent speaking v2 says when that work starts and stops, so it is a turn
/// of its own, shown working, and a prompt sent meanwhile waits for it.
#[test]
fn work_an_agent_speaking_v2_takes_up_by_itself_is_its_own_turn() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v2", "add acp Later");
    let entries = settled_history(&api, &thread, "Went on by itself.");
    assert_eq!(
        said(&entries),
        [
            ("user", "add acp Later"),
            ("assistant", "Done: add acp Later"),
            ("wake", silt_agents::acp::WOKE),
            ("assistant", "Went on by itself.")
        ]
    );
    let kinds: Vec<&str> = entries.iter().map(|(kind, _)| kind.as_str()).collect();
    let reply = kinds.iter().position(|kind| *kind == "assistant").expect("a reply");
    assert_eq!(kinds[reply + 1], "usage", "a turn's cost comes after what it said: {kinds:?}");
    let turns = api.ok("GET", &format!("/api/threads/{thread}/turns"), None);
    assert_eq!(turns.as_array().map(Vec::len), Some(2), "{turns}");
    assert_eq!(entries[turns[1]["entry"].as_u64().expect("its entry") as usize].0, "wake", "{turns}");
}

/// An agent speaking v1, as Copilot does, can't say it went on by itself:
/// what it says once its turn ended opens a turn of its own, closed by the
/// next prompt, which comes after it.
#[test]
fn what_an_agent_speaking_v1_says_after_its_turn_is_its_own_turn_before_the_next_prompt() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v1", "add acp Later");
    settled_history(&api, &thread, "Went on by itself.");
    api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": "and then"})));
    let entries = settled_history(&api, &thread, "Done: and then");
    assert_eq!(
        said(&entries),
        [
            ("user", "add acp Later"),
            ("assistant", "Done: add acp Later"),
            ("wake", silt_agents::acp::WOKE),
            ("assistant", "Went on by itself."),
            ("user", "and then"),
            ("assistant", "Done: and then")
        ]
    );
    let turns = api.ok("GET", &format!("/api/threads/{thread}/turns"), None);
    assert_eq!(turns.as_array().map(Vec::len), Some(3), "{turns}");
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
                .env("FAKE_SCRIPT", "short")
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
    let checkout = env.isolated_checkout();
    assert!(fs::read_to_string(checkout.join("src/main.rs")).expect("read").contains("fn farewell"));
    assert_eq!(fs::read_to_string(env.repo.join("src/main.rs")).expect("read"), "fn main() {}\n");
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
}

#[test]
fn track_attaches_to_a_session_started_without_silt() {
    let env = Env::new();
    let mut fake = env.claude();
    fake.args(["--session-id", "11111111-2222-4333-8444-555555555555"])
        .current_dir(&env.repo)
        .env("CLAUDE_CONFIG_DIR", &env.claude_dir)
        .env("SILT_HOME", &env.silt_home);
    let output = output_once_released(&mut fake);
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
    let ran = output_once_released(
        env.claude()
            .args(["--session-id", session])
            .current_dir(&env.repo)
            .env("CLAUDE_CONFIG_DIR", &env.claude_dir)
            .env("SILT_HOME", &env.silt_home),
    );
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

/// A `claude` that also plays `--cloud` and `-p --cloud`: it names a cloud
/// session after a moment and then stays, as the real one does, in the
/// terminal it was started in, and takes a follow-up on stdin. Anything
/// else is the stand-in's `short` session.
#[cfg(unix)]
fn fake_cloud_claude(env: &Env) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let script = format!(
        r#"#!/bin/sh
set -eu
case "${{1:-}}" in
    --cloud=*)
        {{ printf '%s\n' "$1"; [ -t 0 ] && echo tty; }} > "$CLAUDE_CONFIG_DIR/cloud-started"
        printf '\033[2J\033[1;1HStarting a cloud session\r\n'
        sleep 0.2
        printf 'Branch topic is not on GitHub; starting from main\r\n'
        printf 'View: \033[4mhttps://claude.ai/code/session_01FakeCloud\033[0m?from=cli\r\n'
        sleep 30
        exit 0 ;;
    -p)
        printf '%s\n' "$*" > "$CLAUDE_CONFIG_DIR/cloud-sent"
        cat >> "$CLAUDE_CONFIG_DIR/cloud-sent"
        echo '{{"ok":true,"session_id":"session_01FakeCloud","url":"https://claude.ai/code/session_01FakeCloud"}}'
        exit 0 ;;
esac
FAKE_SCRIPT=short exec '{}' "$@"
"#,
        env.fake.display()
    );
    let fake = env.bin.join("claude");
    fs::write(&fake, script).expect("write fake claude");
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod");
    fake
}

/// The web UI starts a cloud session through the person's own `claude`,
/// lists it until the thread recording it arrives, and sends that thread's
/// session a follow-up.
// The cloud session runs in a terminal of its own, which is Unix's here.
#[cfg(unix)]
#[test]
fn the_web_ui_starts_and_steers_a_cloud_session_through_claude() {
    let mut env = Env::new();
    env.fake = fake_cloud_claude(&env);
    git(&env.repo, &["remote", "add", "origin", "https://github.com/example/app"]);
    let api = Api::open(&env);
    let started = std::time::Instant::now();
    let launch = api.ok("POST", "/api/cloud", Some(serde_json::json!({ "clone": env.repo, "task": "-p fix the tests" })));
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "stopped once the session was named");
    assert_eq!(launch["id"], "session_01FakeCloud", "{launch}");
    assert_eq!(launch["url"], "https://claude.ai/code/session_01FakeCloud");
    assert_eq!(launch["warnings"][0], "Branch topic is not on GitHub; starting from main", "{launch}");
    let ran = fs::read_to_string(env.claude_dir.join("cloud-started")).expect("claude --cloud ran");
    assert_eq!(ran, "--cloud=-p fix the tests\ntty\n", "the task as one argument, in a terminal");
    let elsewhere = tempfile::tempdir().expect("temp dir");
    let (ok, refused) = api.call("POST", "/api/cloud", Some(serde_json::json!({ "clone": elsewhere.path(), "task": "x" })));
    assert!(!ok, "only a known clone: {refused}");

    let launches = api.ok("GET", "/api/cloud", None);
    assert_eq!(launches[0]["thread"], serde_json::Value::Null, "{launches}");

    let session = "11111111-2222-4333-8444-888888888888";
    let ran = Command::new(&env.fake)
        .args(["--session-id", session])
        .current_dir(&env.repo)
        .env("CLAUDE_CONFIG_DIR", &env.claude_dir)
        .env("SILT_HOME", &env.silt_home)
        .output()
        .expect("fake claude runs");
    assert!(ran.status.success());
    let mut hook = Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(["track", "--from-hook"])
        .current_dir(&env.repo)
        .env("SILT_HOME", &env.silt_home)
        .env("CLAUDE_CONFIG_DIR", &env.claude_dir)
        .env("CLAUDE_CODE_REMOTE_SESSION_ID", "cse_01FakeCloud")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("silt runs");
    let input = serde_json::json!({ "session_id": session, "cwd": env.repo, "hook_event_name": "SessionStart" });
    hook.stdin.take().expect("stdin").write_all(input.to_string().as_bytes()).expect("write the hook input");
    assert!(hook.wait().expect("silt exits").success());
    let thread = env.thread();
    let launches = api.ok("GET", "/api/cloud", None);
    assert!(launches[0]["thread"].as_str().is_some_and(|recording| recording.ends_with(&thread)), "{launches}");

    api.ok("POST", &format!("/api/threads/{thread}/cloud/send"), Some(serde_json::json!({ "text": "--and the docs" })));
    let sent = fs::read_to_string(env.claude_dir.join("cloud-sent")).expect("claude -p --cloud ran");
    assert_eq!(sent, "-p --output-format json --cloud session_01FakeCloud\n--and the docs", "the message on stdin");
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
    env.silt(&["claude", "--isolated"]);
    let thread = env.thread();
    let checkout = env.isolated_checkout();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
    fs::rename(checkout.join("src/main.rs"), checkout.join("src/app.rs")).expect("rename");
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
    env.silt(&["claude", "--isolated"]);
    let thread = env.thread();
    let checkout = env.isolated_checkout();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
    let bytes = fs::read(checkout.join("src/main.rs")).expect("reads");
    fs::remove_file(checkout.join("src/main.rs")).expect("removes");
    std::thread::sleep(std::time::Duration::from_millis(300));
    fs::write(checkout.join("src/app.rs"), bytes).expect("writes");
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
fn commits_get_silts_note_and_blame_reads_the_log_or_the_notes() {
    let env = Env::with_claude("tools");
    git(&env.repo, &["config", "silt.notes", "local"]);
    env.silt(&["claude"]);
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+# Notes")));
    git(&env.repo, &["add", "."]);
    git(&env.repo, &["commit", "-qm", "farewell, helper and notes"]);
    let note = || {
        let output =
            Command::new("git").args(["notes", "--ref=silt", "show", "HEAD"]).current_dir(&env.repo).output().expect("git");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    assert!(eventually(|| note().starts_with("silt-note 1\n")), "no note on the commit");
    let note = note();
    assert!(note.contains("turn 1 Add a farewell and a helper\n") && note.contains("turn 2 Now a note\n"), "{note}");
    assert!(note.contains("file src/main.rs\n2 turn 1 agent claude-code\n"), "{note}");
    assert!(note.contains("file NOTES.md\n1 turn 2 agent claude-code\n"), "{note}");
    assert!(note.contains(" person\n"), "the hand edit is the person's: {note}");

    let from_log = env.stdout(&["blame", "src/main.rs"]);
    let lines: Vec<&str> = from_log.lines().collect();
    assert_eq!(lines.len(), 2, "{from_log}");
    assert!(!lines[0].contains(" t1 ") && lines[0].ends_with("| fn main() {}"), "git's line: {from_log}");
    assert!(lines[1].contains(" t1 ") && lines[1].contains("claude-code") && lines[1].contains("Add a farewell"), "{from_log}");

    let from_notes = env.stdout(&["blame", "--notes", "src/main.rs"]);
    assert_eq!(from_notes, from_log, "the notes say what the log says");
}

#[test]
fn diffs_start_from_the_base_asked_for() {
    let env = Env::new();
    let first = String::from_utf8_lossy(
        &Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&env.repo).output().expect("git runs").stdout,
    )
    .trim()
    .to_owned();
    env.silt(&["claude", "--isolated"]);
    let thread = env.thread();
    let checkout = env.isolated_checkout();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
    git(&checkout, &["commit", "-qam", "farewell"]);
    assert!(eventually(|| !env.stdout(&["diff", &thread, "--base", "last-commit"]).contains("farewell")));
    let main = checkout.join("src/main.rs");
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
    let committed = String::from_utf8_lossy(
        &Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&checkout).output().expect("git runs").stdout,
    )
    .trim()
    .to_owned();
    let against_head = env.stdout(&["diff", &thread, "--base", &format!("branch:{committed}")]);
    assert!(against_head.contains("+fn later() {}") && !against_head.contains("+fn farewell"), "{against_head}");
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

/// Runs a script a test just wrote. Another test forking while it was being
/// written can hold it open for a moment, which fails the exec.
fn output_once_released(command: &mut Command) -> std::process::Output {
    (0..50)
        .find_map(|_| match command.output() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(20));
                None
            }
            other => Some(other),
        })
        .expect("the script was released")
        .expect("the script runs")
}

/// The daemon's address and token from `silt open`'s URL, which names the
/// project it ran in when that's a clone.
fn web_ui(url: &str) -> (&str, &str) {
    let (page, token) = url.trim().split_once("#token=").expect("a URL with a token");
    (page.split_once("/?").map_or(page.trim_end_matches('/'), |(base, _project)| base), token)
}
