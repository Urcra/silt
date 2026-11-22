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
        // Under a short temporary directory on Unix, which keeps the daemon's
        // socket path short: `TMPDIR` when it is, else `/tmp`.
        let mut builder = tempfile::Builder::new();
        builder.prefix("silt");
        let temp = std::env::temp_dir();
        let short = if temp.as_os_str().len() <= 40 { temp } else { "/tmp".into() };
        let silt_home = if cfg!(unix) { builder.tempdir_in(short) } else { builder.tempdir() }.expect("temp dir");
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

/// Longer than a daemon started in the background keeps trying for the
/// data dir's lock, a second.
const DAEMON_LOCK_WAIT: Duration = Duration::from_millis(1500);

impl Drop for Env {
    /// A session's wrapper and hooks each start a daemon when none runs, so
    /// several may start: those that lost the race for the lock take it once
    /// the winner stops. Stopped until none has held it for as long as they
    /// try, else one would run on in the data dir once it's removed,
    /// recreating it.
    fn drop(&mut self) {
        let daemon = |command: &str| {
            Command::new(env!("CARGO_BIN_EXE_silt")).args(["daemon", command]).env("SILT_HOME", &self.silt_home).output()
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut quiet_since = Instant::now();
        let _ = daemon("stop");
        while quiet_since.elapsed() < DAEMON_LOCK_WAIT && Instant::now() < deadline {
            let status = daemon("status").map(|output| String::from_utf8_lossy(&output.stdout).into_owned()).unwrap_or_default();
            if status.trim() != "not running" {
                let _ = daemon("stop");
                quiet_since = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
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

    next_prompt(&env, "one more");
    assert!(
        eventually(|| env.stdout(&["diff", &thread]).contains("+fn later() {}")),
        "the next turn takes in what changed meanwhile:\n{}",
        env.stdout(&["show", &thread])
    );
    let show = env.stdout(&["show", &thread]);
    assert!(show.contains("outside tool calls: src/main.rs +14 -0"), "the person's, not the agent's: {show}");
}

/// Submits the next prompt of the session, as Claude Code's hook does, which
/// takes in what changed in the clone since the last turn.
fn next_prompt(env: &Env, prompt: &str) {
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
    let input = serde_json::json!({"session_id": session, "hook_event_name": "UserPromptSubmit", "prompt": prompt});
    hook.stdin.take().expect("piped").write_all(input.to_string().as_bytes()).expect("hook input");
    assert!(hook.wait().expect("hook ran").success());
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
    assert_eq!(first.clone() + &rest, whole, "the pages make up the whole diff, in path order");

    let output = Command::new("curl")
        .args(["-s", "-H"])
        .arg(format!("Authorization: Bearer {token}"))
        .arg(format!("{base}/api/threads/{thread}/diff/stats"))
        .output()
        .expect("curl runs");
    let stats: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).expect("the diff's files");
    assert_eq!(stats.len(), 3, "{stats:?}");
    let mut by_path = String::new();
    for stat in &stats {
        let path = stat["path"].as_str().expect("a path");
        let one = diff(&format!("?path={path}"));
        assert_eq!(one.matches("diff --git").count(), 1, "{one}");
        assert_eq!(stat["size"].as_u64(), Some(one.len() as u64), "its size is its part of the diff");
        let added = one.lines().filter(|line| line.starts_with('+') && !line.starts_with("+++")).count();
        assert_eq!(stat["added"].as_u64(), Some(added as u64), "{one}");
        by_path += &one;
    }
    assert_eq!(by_path, whole, "file by file, the stats' order is the diff's");
}

#[test]
fn a_subagent_nests_under_its_call_and_a_background_command_says_how_it_ended() {
    let env = Env::with_claude("beside");
    env.silt(&["claude"]);
    let thread = env.thread();
    let url = env.stdout(&["open", "--no-browser"]);
    let (base, token) = web_ui(&url);
    let get = |path: &str| -> serde_json::Value {
        let output = Command::new("curl")
            .args(["-s", "-H"])
            .arg(format!("Authorization: Bearer {token}"))
            .arg(format!("{base}/api/threads/{thread}/{path}"))
            .output()
            .expect("curl runs");
        serde_json::from_slice(&output.stdout).expect("json")
    };
    let entries = || get("history")["entries"].as_array().cloned().unwrap_or_default();
    assert!(eventually(|| entries().iter().any(|entry| entry["kind"] == "assistant"
        && entry["side"].is_null()
        && entry["text"].as_str().is_some_and(|text| text.contains("failed to start")))));
    let entries = entries();
    let side: Vec<&str> =
        entries.iter().filter(|entry| entry["side"] == "agent-a1").filter_map(|entry| entry["text"].as_str()).collect();
    assert_eq!(side, ["Investigate the lexer", "Read lexer.rs: nothing wrong with it."], "the subagent's own entries, marked");
    let news: Vec<&serde_json::Value> =
        entries.iter().filter_map(|entry| entry["task"].as_object().map(|_| &entry["task"])).collect();
    assert!(
        news.iter().any(|task| task["call"] == "toolu_a" && task["side"] == "agent-a1" && task["status"] == "completed"),
        "{news:?}"
    );
    assert!(
        news.iter().any(|task| task["call"] == "toolu_b" && task["kind"] == "background" && task["status"] == "running"),
        "{news:?}"
    );
    assert!(news.iter().any(|task| task["id"] == "bg1" && task["status"] == "failed"), "{news:?}");
    assert_eq!(get("turns").as_array().map(Vec::len), Some(1), "the subagent's prompt starts no turn of the thread's");
    let show = env.stdout(&["show", &thread]);
    assert!(!show.contains("nothing wrong with it"), "the thread's own conversation leaves the subagent's out:\n{show}");
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
fn what_git_rewrote_in_a_tool_call_can_be_left_out_of_the_diff() {
    git_left_out("git", true);
}

#[test]
fn what_git_rewrote_between_turns_can_be_left_out_of_the_diff() {
    git_left_out("git-by-hand", false);
}

#[test]
fn what_a_stash_apply_wrote_can_be_left_out_of_the_diff() {
    let env = Env::with_claude("git");
    fs::write(env.repo.join("lib.rs"), "fn lib() {}\n").expect("write");
    git(&env.repo, &["add", "."]);
    git(&env.repo, &["commit", "-q", "-m", "lib"]);
    fs::write(env.repo.join("lib.rs"), "fn stashed() {}\nfn lib() {}\n").expect("write");
    git(&env.repo, &["stash", "-q"]);
    let ran = env.run_with(&["claude"], &[("FAKE_GIT", "stash apply -q")]);
    assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
    let thread = env.thread();
    assert!(
        eventually(|| env.stdout(&["diff", &thread]).contains("+fn stashed() {}")),
        "the diff has what the stash brought back:\n{}",
        env.stdout(&["diff", &thread])
    );
    let mine = env.stdout(&["diff", &thread, "--hide-git"]);
    assert!(mine.contains("+fn mine() {}") && mine.contains("+fn after() {}") && !mine.contains("fn stashed"), "{mine}");
}

/// The stand-in's `script` edits `src/main.rs`, merges `upstream` (a new
/// `pulled.rs` and a line atop `lib.rs`) and edits `lib.rs`: `--hide-git`
/// shows its edits alone, numbered without git's line, with the daemon and
/// without.
fn git_left_out(script: &'static str, in_one_turn: bool) {
    for daemonless in [false, true] {
        let env = Env::with_claude(script);
        fs::write(env.repo.join("lib.rs"), "fn lib() {}\n").expect("write");
        git(&env.repo, &["add", "."]);
        git(&env.repo, &["commit", "-q", "-m", "lib"]);
        git(&env.repo, &["checkout", "-q", "-b", "upstream"]);
        fs::write(env.repo.join("pulled.rs"), "fn theirs() {}\n").expect("write");
        fs::write(env.repo.join("lib.rs"), "fn upstream() {}\nfn lib() {}\n").expect("write");
        git(&env.repo, &["add", "."]);
        git(&env.repo, &["commit", "-q", "-m", "upstream"]);
        git(&env.repo, &["checkout", "-q", "main"]);
        let capture: &[(&str, &str)] = if daemonless { &[("SILT_DAEMONLESS_CAPTURE", "1")] } else { &[] };
        let ran = env.run_with(&["claude"], capture);
        assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
        let thread = env.thread();
        assert!(
            eventually(|| {
                let diff = env.stdout(&["diff", &thread]);
                diff.contains("+fn mine() {}") && diff.contains("+fn theirs() {}") && diff.contains("+fn after() {}")
            }),
            "the diff has both the agent's edits and git's:\n{}",
            env.stdout(&["diff", &thread])
        );
        let mine = env.stdout(&["diff", &thread, "--hide-git"]);
        assert!(mine.contains("+fn mine() {}") && !mine.contains("pulled.rs"), "daemonless: {daemonless}\n{mine}");
        assert!(mine.contains("+fn after() {}") && !mine.contains("fn upstream"), "daemonless: {daemonless}\n{mine}");
        if in_one_turn {
            let turn = env.stdout(&["diff", &thread, "--turn", "1", "--hide-git"]);
            assert!(turn.contains("+fn mine() {}") && !turn.contains("pulled.rs"), "daemonless: {daemonless}\n{turn}");
        }

        // `fn after` is line 2 without git's line above it, and line 3 on disk.
        env.silt(&["comment", &thread, "--file", "lib.rs", "--lines", "2", "--hide-git", "why", "after?"]);
        let on_disk = env.stdout(&["comments", &thread]);
        assert!(on_disk.contains("lib.rs:3"), "daemonless: {daemonless}\n{on_disk}");
        let without_git = env.stdout(&["comments", &thread, "--hide-git"]);
        assert!(without_git.contains("lib.rs:2"), "daemonless: {daemonless}\n{without_git}");

        // A pending comment's anchor, taken without git's line, is placed as each diff numbers it.
        let api = Api::open(&env);
        let lines = serde_json::json!({"on": "lines", "path": "lib.rs", "start": 2, "end": 2, "hide_git": true});
        let (ok, pending) = api.call("POST", &format!("/api/threads/{thread}/comments/anchor"), Some(lines));
        assert!(ok, "{pending}");
        let place = |hide_git: bool| {
            let body = serde_json::json!({"base": null, "anchors": [pending["anchor"]], "hide_git": hide_git});
            let (ok, placed) = api.call("POST", &format!("/api/threads/{thread}/comments/place"), Some(body));
            assert!(ok, "{placed}");
            placed[0]["start"].clone()
        };
        assert_eq!(place(true), 2, "daemonless: {daemonless}");
        assert_eq!(place(false), 3, "daemonless: {daemonless}");

        // The expanded review numbers the turn that wrote a line, and the lines new since Seen, as its diff does.
        let turn_runs = |git: &str| {
            let (ok, files) = api.call("GET", &format!("/api/threads/{thread}/diff/turns?git={git}"), None);
            assert!(ok, "{files}");
            let lib = files.as_array().expect("files").iter().find(|file| file["path"] == "lib.rs").cloned();
            lib.map(|lib| lib["runs"].clone()).unwrap_or_default()
        };
        let starts = |runs: serde_json::Value| -> Vec<u64> {
            runs.as_array().map(|runs| runs.iter().filter_map(|run| run["start"].as_u64()).collect()).unwrap_or_default()
        };
        assert_eq!(starts(turn_runs("hide")), vec![2], "daemonless: {daemonless}");
        let shown = starts(turn_runs("show"));
        assert!(shown.contains(&3) && !shown.contains(&2), "git's line above moves it to 3: {shown:?}, daemonless: {daemonless}");
        let seen = |git: &str| {
            let (ok, seen) = api.call("GET", &format!("/api/threads/{thread}/seen?git={git}"), None);
            assert!(ok, "{seen}");
            seen
        };
        let head = seen("show")["head"].as_str().expect("head").to_owned();
        let mark = serde_json::json!({"path": "lib.rs", "seen": true, "at": head});
        let (ok, marked) = api.call("PUT", &format!("/api/threads/{thread}/seen"), Some(mark));
        assert!(ok, "{marked}");
        assert_eq!(seen("hide")["files"][0]["seen"], "seen", "the box takes no tick away; daemonless: {daemonless}");
        let lib = env.repo.join("lib.rs");
        fs::write(&lib, fs::read_to_string(&lib).expect("read") + "fn later() {}\n").expect("write");
        next_prompt(&env, "one more");
        assert!(eventually(|| seen("show")["files"][0]["seen"] == "changed"), "daemonless: {daemonless}");
        assert_eq!(seen("show")["files"][0]["new_lines"], serde_json::json!([{"start": 4, "end": 4}]));
        assert_eq!(seen("hide")["files"][0]["new_lines"], serde_json::json!([{"start": 3, "end": 3}]));
    }
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
    start_fake_acp_in(env, api, &serde_json::json!({"clone": env.repo, "agent": agent, "prompt": prompt}))
}

/// [`start_fake_acp`] with the start request given whole.
fn start_fake_acp_in(env: &Env, api: &Api, start: &serde_json::Value) -> String {
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([
        {"id": "v1", "name": "Fake v1", "command": fake},
        {"id": "v2", "name": "Fake v2", "command": fake, "env": {"FAKE_ACP_V2": "1"}}
    ]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started = api.ok("POST", "/api/agents/start", Some(start.clone()));
    started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("the thread").to_owned())
}

/// An agent whose own sign-in expired waits on a card, not an error
/// (ADR 0056): its sign-in runs on its machine, the card shows the link and
/// takes the code from its page, and the agent goes on with what it was
/// asked, the prompt recorded once. Over v1 the agent refuses its session,
/// as Copilot does; over v2 it refuses the prompt, as Claude's adapter does.
/// An agent that answers v2 in v1 and lists its sign-in only to a v1 client
/// asking for one, as Antigravity's does, is asked again in v1.
#[test]
fn an_agent_whose_sign_in_expired_is_signed_in_from_the_web_ui_and_goes_on() {
    let env = Env::new();
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let markers = tempfile::tempdir().expect("temp dir");
    let (v1, v2, asked) = (markers.path().join("v1"), markers.path().join("v2"), markers.path().join("asked"));
    let agents = serde_json::json!([
        {"id": "v1", "name": "Fake v1", "command": fake, "env": {"FAKE_ACP_SIGN_IN": v1}},
        {"id": "v2", "name": "Fake v2", "command": fake, "env": {"FAKE_ACP_SIGN_IN": v2, "FAKE_ACP_V2": "1"}},
        {"id": "asked", "name": "Fake asked", "command": fake, "env": {"FAKE_ACP_SIGN_IN": asked, "FAKE_ACP_SIGN_IN_IF_ASKED": "1"}}
    ]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    for (agent, marker) in [("v1", &v1), ("v2", &v2), ("asked", &asked)] {
        let started =
            api.ok("POST", "/api/agents/start", Some(serde_json::json!({"clone": env.repo, "agent": agent, "prompt": "hello"})));
        let thread =
            started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("the thread").to_owned());
        let live = || api.ok("GET", &format!("/api/threads/{thread}/live"), None)["agent"].clone();
        let mut seen = serde_json::Value::Null;
        let asked = eventually(|| {
            seen = live();
            seen["state"] == "awaiting_permission" && seen["sign_in"]["methods"][0]["id"] == "fake-login"
        });
        assert!(asked, "{agent}: the agent asks for a sign-in: {seen}");
        assert_eq!(seen["error"], serde_json::Value::Null, "{agent}: needing a sign-in is no error");

        let sign_in = |code: &str| {
            api.ok("POST", &format!("/api/threads/{thread}/agent/sign-in"), Some(serde_json::json!({"method": "fake-login"})));
            let mut seen = serde_json::Value::Null;
            let shown = eventually(|| {
                seen = live();
                seen["sign_in"]["running"]["asks_code"] == true
            });
            assert!(shown, "{agent}: the sign-in asks for its code: {seen}");
            assert_eq!(seen["sign_in"]["running"]["link"], "https://example.invalid/oauth?state=fake", "{agent}");
            api.ok("POST", &format!("/api/threads/{thread}/agent/sign-in/code"), Some(serde_json::json!({"code": code})));
        };
        sign_in("bad");
        let failed = eventually(|| {
            seen = live();
            seen["sign_in"]["failed"].as_str().is_some_and(|failed| failed.contains("wasn't accepted"))
        });
        assert!(failed, "{agent}: a wrong code fails, and the card stays: {seen}");
        assert!(!marker.exists());

        sign_in("good");
        let entries = settled_history(&api, &thread, "Done: hello");
        assert!(marker.exists(), "{agent}: the sign-in ran with the agent's own environment");
        assert_eq!(live()["sign_in"], serde_json::Value::Null, "{agent}");
        let prompts = entries.iter().filter(|(kind, text)| kind == "user" && text == "hello").count();
        assert_eq!(prompts, 1, "{agent}: the prompt is recorded once: {entries:?}");
    }
}

/// A project started from nothing (ADR 0053): a folder made for it with an
/// empty repository, named by its folder and kept on this machine, whose
/// threads move to its remote's project once it has one.
#[test]
fn a_new_project_starts_in_a_folder_made_for_it_and_follows_it_to_a_remote() {
    let env = Env::new();
    let api = Api::open(&env);
    let (_place, folder) = new_project(&api);
    let folder_project = silt_git::normalize_remote(&folder.to_string_lossy());
    assert!(folder.join(".git").is_dir(), "the folder has a repository");
    let (made_again, answer) = api.call("POST", "/api/projects/new", Some(serde_json::json!({"path": folder})));
    assert!(!made_again && answer["error"].as_str().is_some_and(|error| error.contains("already there")), "{answer}");
    let projects = api.ok("GET", "/api/projects", None);
    let listed = projects.as_array().expect("projects").iter().find(|listed| listed["repo"] == folder_project.as_str()).cloned();
    let listed = listed.unwrap_or_else(|| panic!("the new project is listed: {projects}"));
    assert_eq!((listed["name"].as_str(), listed["home"].as_str()), (Some("quest-viewer"), Some("local")));

    let first = start_here(&env, &api, &folder, "hello");
    assert_eq!(repo_of(&api, &first).as_deref(), Some(folder_project.as_str()));

    git(&folder, &["remote", "add", "origin", "git@github.com:acme/quest-viewer.git"]);
    let second = start_here(&env, &api, &folder, "again");
    for thread in [&first, &second] {
        assert_eq!(repo_of(&api, thread).as_deref(), Some("github.com/acme/quest-viewer"), "thread {thread}");
    }
}

/// [`a_new_project_starts_in_a_folder_made_for_it_and_follows_it_to_a_remote`]
/// with the remote added while the daemon was stopped.
#[test]
fn a_new_projects_threads_follow_a_remote_added_while_the_daemon_was_stopped() {
    let env = Env::new();
    let api = Api::open(&env);
    let (_place, folder) = new_project(&api);
    let thread = start_here(&env, &api, &folder, "hello");
    env.silt(&["daemon", "stop"]);
    git(&folder, &["remote", "add", "origin", "git@github.com:acme/quest-viewer.git"]);
    let api = Api::open(&env);
    assert_eq!(repo_of(&api, &thread).as_deref(), Some("github.com/acme/quest-viewer"));
}

/// Makes the project `quest-viewer` from the web UI, in a folder of its
/// own, which goes when the first value is dropped.
fn new_project(api: &Api) -> (tempfile::TempDir, PathBuf) {
    let place = tempfile::tempdir().expect("temp dir");
    let folder = silt_os::canonicalize(place.path()).expect("canonical").join("quest-viewer");
    let project = api.ok("POST", "/api/projects/new", Some(serde_json::json!({"path": folder})));
    assert_eq!(project, silt_git::normalize_remote(&folder.to_string_lossy()).as_str());
    (place, folder)
}

/// Starts the scripted agent working in `clone` itself, as the web UI's
/// Local does, on `prompt`, once it has replied.
fn start_here(env: &Env, api: &Api, clone: &Path, prompt: &str) -> String {
    let thread = start_fake_acp_in(env, api, &serde_json::json!({"clone": clone, "agent": "v2", "prompt": prompt, "here": true}));
    settled_history(api, &thread, &format!("Done: {prompt}"));
    thread
}

/// The project the thread listing names for `thread`.
fn repo_of(api: &Api, thread: &str) -> Option<String> {
    let threads = api.ok("GET", "/api/threads", None);
    let listed = threads.as_array().expect("threads").iter().find(|listed| listed["id"] == thread).cloned();
    listed.map(|listed| listed["repo"].as_str().unwrap_or_default().to_owned())
}

/// The thread's history as kind and text, once its agent is idle and the
/// history says `last`.
fn settled_history(api: &Api, thread: &str, last: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let settled = eventually(|| {
        let history = api.ok("GET", &format!("/api/threads/{thread}/history?whole=true"), None);
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
/// what it says once its turn ended opens a turn of its own, which works
/// until it goes quiet, and a prompt sent meanwhile waits for it.
#[test]
fn what_an_agent_speaking_v1_says_after_its_turn_is_its_own_turn_before_the_next_prompt() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v1", "add acp Later");
    let woke = eventually(|| {
        let history = api.ok("GET", &format!("/api/threads/{thread}/history"), None);
        history["entries"].as_array().is_some_and(|entries| entries.iter().any(|entry| entry["kind"] == "wake"))
    });
    assert!(woke, "the agent went on by itself");
    let sent = api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": "and then"})));
    assert_eq!(sent, "queued", "it waits for the agent's own turn");
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

/// The web UI shows a long reply whole; only an agent reading history
/// through its tools gets entries cut short.
#[test]
fn a_long_reply_reaches_the_web_ui_whole() {
    let env = Env::new();
    let api = Api::open(&env);
    let prompt = "word ".repeat(1000);
    let thread = start_fake_acp(&env, &api, "v2", prompt.trim());
    let reply = format!("Done: {}", prompt.trim());
    let entries = settled_history(&api, &thread, &reply);
    assert!(said(&entries).contains(&("assistant", reply.as_str())));
}

/// The thread's history entries as the API gives them.
fn history_entries(api: &Api, thread: &str) -> Vec<serde_json::Value> {
    api.ok("GET", &format!("/api/threads/{thread}/history?whole=true"), None)["entries"].as_array().cloned().unwrap_or_default()
}

/// The commands an agent offers reach the web UI: over v1 once its session
/// opens, as Copilot sends them, and over v2 with the session and as they
/// change, as Claude's adapter does. New thread offers each agent's from its
/// last run here.
#[test]
fn the_commands_an_agent_offers_reach_the_web_ui_and_are_kept_for_new_threads() {
    let env = Env::new();
    let api = Api::open(&env);
    let names = |commands: &serde_json::Value| -> Vec<String> {
        commands.as_array().into_iter().flatten().filter_map(|command| command["name"].as_str().map(str::to_owned)).collect()
    };
    let offered = |thread: &str| api.ok("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["commands"].clone();

    let v1 = start_fake_acp(&env, &api, "v1", "hello");
    assert!(eventually(|| names(&offered(&v1)) == ["count", "compact"]), "{}", offered(&v1));
    let count = offered(&v1)[0].clone();
    assert_eq!((count["description"].as_str(), count["hint"].as_str()), (Some("Count what you name"), Some("<what to count>")));
    assert_eq!(offered(&v1)[1]["hint"], serde_json::Value::Null);

    let v2 = start_fake_acp(&env, &api, "v2", "hello");
    settled_history(&api, &v2, "Done: hello");
    assert_eq!(names(&offered(&v2)), ["count"], "the agent dropped compact once prompted");

    let choices = api.ok("GET", "/api/agents", None);
    let kept = |agent: &str| {
        let choice = choices["agents"].as_array().expect("agents").iter().find(|choice| choice["id"] == agent).cloned();
        names(&choice.unwrap_or_default()["commands"])
    };
    assert_eq!((kept("v1"), kept("v2")), (vec!["count".to_owned(), "compact".to_owned()], vec!["count".to_owned()]), "{choices}");
}

/// The prompt Claude's adapter predicts once a turn ends reaches the web UI
/// for the composer to offer, and the next prompt spends it.
#[test]
fn the_prompt_an_agent_suggests_reaches_the_web_ui_until_the_next_prompt() {
    let env = Env::new();
    let api = Api::open(&env);
    let suggested = |thread: &str| api.ok("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["suggestion"].clone();
    let thread = start_fake_acp(&env, &api, "v2", "add acp Suggest");
    assert!(eventually(|| suggested(&thread) == "Add a test for it"), "{}", suggested(&thread));
    api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": "and then"})));
    settled_history(&api, &thread, "Done: and then");
    assert_eq!(suggested(&thread), serde_json::Value::Null);
}

/// An agent that puts its plan to the person itself, as Claude does with
/// ExitPlanMode, is the one to say when there's a plan: a plan turn it ends,
/// here with its plan sent back and a command left running, isn't one to
/// approve. Silt remembers that the agent plans so.
#[test]
fn an_agent_that_presents_its_own_plans_is_never_taken_to_have_planned_by_its_turns_end() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp_in(
        &env,
        &api,
        &serde_json::json!({"clone": env.repo, "agent": "v2", "prompt": "Present Background", "preset": "plan"}),
    );
    let (mut presented, mut asked_by_silt) = (false, false);
    let went_on = eventually(|| {
        let live = api.ok("GET", &format!("/api/threads/{thread}/live"), None);
        for asked in live["agent"]["permissions"].as_array().into_iter().flatten() {
            presented |= asked["plan"] == "1. Count the lines.";
            let answer = serde_json::json!({"id": asked["id"], "option": "reject"});
            api.call("POST", &format!("/api/threads/{thread}/agent/answer"), Some(answer));
        }
        let state = live["agent"]["state"].as_str().unwrap_or_default().to_owned();
        asked_by_silt |= state == "awaiting_input";
        let printed = history_entries(&api, &thread).iter().any(|entry| entry["text"] == "The background command printed BG.");
        printed && state == "idle"
    });
    assert!(presented, "the agent put its plan to the person");
    assert!(!asked_by_silt, "Silt asked to approve a plan of its own reading");
    assert!(went_on, "the agent went on once the command ended, and stopped");
    let seen: serde_json::Value =
        serde_json::from_slice(&fs::read(env.silt_home.join("agents-seen.json")).expect("agents seen")).expect("json");
    assert_eq!(seen["v2"]["presents_plans"], true, "{seen}");
}

/// Over v2 a subagent runs in a session of its own and a command left in
/// the background ends after the turn: both are tasks of the call that
/// started them, the subagent's words are its own, and the turn the
/// command's end sets off names it.
#[test]
fn subagents_and_background_commands_of_an_agent_speaking_v2_are_tasks() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v2", "add acp Subagent Background");
    settled_history(&api, &thread, "The background command printed BG.");
    let entries = history_entries(&api, &thread);
    let news: Vec<(String, String, String)> = entries
        .iter()
        .filter_map(|entry| {
            let task = entry.get("task").filter(|task| !task.is_null())?;
            let field = |key: &str| task[key].as_str().unwrap_or_default().to_owned();
            Some((field("call"), field("kind"), field("status")))
        })
        .collect();
    let at = |call: &str, kind: &str, status: &str| (call.to_owned(), kind.to_owned(), status.to_owned());
    for expected in [
        at("agent-1", "subagent", "running"),
        at("agent-1", "subagent", "completed"),
        at("bg-1", "background", "running"),
        at("bg-1", "background", "completed"),
    ] {
        assert!(news.contains(&expected), "{expected:?} in {news:?}");
    }
    let own_words = entries.iter().find(|entry| entry["text"] == "Counted the lines.").expect("the subagent's words");
    assert_eq!(own_words["side"], "agent-1", "the subagent's, not the thread's");
    let call = entries.iter().find(|entry| entry["kind"] == "tool_call" && entry["tool"]["id"] == "agent-1").expect("its call");
    assert_eq!(call["tool"]["subject"], "Count lines");
    let wake = entries.iter().find(|entry| entry["kind"] == "wake").expect("the turn it set off");
    assert_eq!(wake["text"], "`sleep 1 && echo BG` finished in the background, and the agent went on by itself.");
    let turns = api.ok("GET", &format!("/api/threads/{thread}/turns"), None);
    assert_eq!(turns.as_array().map(Vec::len), Some(2), "the subagent's prompt opens no turn: {turns}");
}

/// Over v1 an agent such as Copilot names a shell it detached only in its
/// answers, and goes on by itself by reading it once it ended: the shell is
/// a task of the call that started it, and the turn it set off names it.
#[test]
fn a_shell_an_agent_speaking_v1_detached_is_a_task_that_names_the_turn_it_sets_off() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v1", "add acp Detached");
    settled_history(&api, &thread, "The detached shell printed DETACHED.");
    let raw = history_entries(&api, &thread);
    let calls: Vec<(&str, &str)> =
        raw.iter().filter_map(|entry| Some((entry["task"]["call"].as_str()?, entry["task"]["status"].as_str()?))).collect();
    assert_eq!(calls, [("shell-1", "running"), ("shell-1", "completed")], "both are news of the call that started it");
    let wake = raw.iter().find(|entry| entry["kind"] == "wake").expect("the turn it set off");
    assert_eq!(wake["text"], "`sleep 1 && echo DETACHED` ended in the background, and the agent went on by itself.");
}

/// Once the person stops the agent, a command it left running doesn't get
/// to set it going again before they prompt it.
#[test]
fn once_stopped_an_agent_isn_t_set_going_by_what_it_left_running() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v2", "add acp Background");
    settled_history(&api, &thread, "Done: add acp Background");
    api.ok("POST", &format!("/api/threads/{thread}/agent/cancel"), None);
    let woke = eventually(|| {
        let idle = api.ok("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["state"] == "idle";
        idle && history_entries(&api, &thread).iter().any(|entry| entry["kind"] == "wake")
    });
    assert!(woke, "the command's end set the agent going");
    let entries = history_entries(&api, &thread);
    let wake = entries.iter().find(|entry| entry["kind"] == "wake").expect("the turn");
    assert!(wake["text"].as_str().is_some_and(|text| text.ends_with("Silt stopped it, as you had stopped the agent.")), "{wake}");
    assert!(
        !entries.iter().any(|entry| entry["text"] == "The background command printed BG."),
        "it was stopped before it went on"
    );
}

/// The processes the fake agent's `Keep` left running for this test's repository.
fn kept(env: &Env) -> Vec<u32> {
    let repo = env.repo.file_name().expect("named").to_string_lossy().into_owned();
    silt_os::processes()
        .expect("lists processes")
        .into_iter()
        .filter(|process| process.command.iter().any(|part| part == "keep-running") && process.command.join(" ").contains(&repo))
        .map(|process| process.pid)
        .collect()
}

/// What the history says of the task the call `call` started, in order.
fn task_news(api: &Api, thread: &str, call: &str) -> Vec<String> {
    history_entries(api, thread)
        .iter()
        .filter(|entry| entry["task"]["call"] == call)
        .filter_map(|entry| entry["task"]["status"].as_str().map(str::to_owned))
        .collect()
}

/// A command the agent left running in the background is ended from the web
/// UI: its process with it, its card says it stopped, and its end doesn't
/// set the agent going.
#[test]
fn a_command_left_running_in_the_background_is_stopped_from_the_web_ui() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v2", "add acp Keep");
    settled_history(&api, &thread, "Done: add acp Keep");
    assert!(eventually(|| kept(&env).len() == 1), "the command runs");
    let live = api.ok("GET", &format!("/api/threads/{thread}/live"), None);
    assert_eq!(live["agent"]["stoppable"], serde_json::json!(["keep-1"]), "{live}");
    assert_eq!(listed_background(&api, &thread), 1, "the thread list says something still runs");
    let (refused, said) = api.call("POST", &format!("/api/threads/{thread}/agent/tasks/run-1/stop"), None);
    assert!(!refused && said["error"].as_str().is_some_and(|error| error.contains("isn't running")), "{said}");

    api.ok("POST", &format!("/api/threads/{thread}/agent/tasks/keep-1/stop"), None);
    assert!(eventually(|| kept(&env).is_empty()), "its process ended");
    let woke = eventually(|| {
        let idle = api.ok("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["state"] == "idle";
        idle && history_entries(&api, &thread).iter().any(|entry| entry["kind"] == "wake")
    });
    assert!(woke, "its end set the agent going");
    assert_eq!(task_news(&api, &thread, "keep-1"), ["running", "stopped"], "its end is no news");
    let entries = history_entries(&api, &thread);
    let wake = entries.iter().find(|entry| entry["kind"] == "wake").expect("the turn");
    assert!(
        wake["text"].as_str().is_some_and(|text| text.ends_with("Silt stopped it, as you had stopped the command.")),
        "{wake}"
    );
    assert!(!entries.iter().any(|entry| entry["text"] == "The kept command ended."), "it was stopped before it went on");
    let live = api.ok("GET", &format!("/api/threads/{thread}/live"), None);
    assert_eq!(live["agent"]["stoppable"], serde_json::json!([]), "{live}");
    assert_eq!(listed_background(&api, &thread), 0, "{live}");
}

/// A command left running that ended without Silt hearing of it is marked
/// stopped when the person stops it, rather than shown running for good.
#[test]
fn a_command_left_running_that_ended_unheard_is_marked_stopped_when_stopped() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v2", "add acp Unheard");
    settled_history(&api, &thread, "Done: add acp Unheard");
    let live = api.ok("GET", &format!("/api/threads/{thread}/live"), None);
    assert_eq!(live["agent"]["stoppable"], serde_json::json!(["unheard-1"]), "{live}");

    api.ok("POST", &format!("/api/threads/{thread}/agent/tasks/unheard-1/stop"), None);
    assert_eq!(task_news(&api, &thread, "unheard-1"), ["running", "stopped"]);
    let live = api.ok("GET", &format!("/api/threads/{thread}/live"), None);
    assert_eq!(live["agent"]["stoppable"], serde_json::json!([]), "{live}");
    assert_eq!(listed_background(&api, &thread), 0, "{live}");
}

/// A shell Copilot detached and never reads ends without Silt hearing of
/// it: Silt sees its process gone and marks it done, and leaves it running
/// until then.
#[test]
fn a_shell_an_agent_speaking_v1_never_reads_is_done_once_its_process_ended() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v1", "add acp Forgotten");
    settled_history(&api, &thread, "Done: add acp Forgotten");
    assert!(eventually(|| kept(&env).len() == 1), "the command runs");
    // Past its first look among the agent's processes.
    std::thread::sleep(Duration::from_secs(7));
    assert_eq!(task_news(&api, &thread, "shell-1"), ["running"], "it still runs");
    let live = api.ok("GET", &format!("/api/threads/{thread}/live"), None);
    assert_eq!(live["agent"]["stoppable"], serde_json::json!(["shell-1"]), "{live}");

    for pid in kept(&env) {
        silt_os::kill(pid).expect("ends");
    }
    assert!(eventually(|| task_news(&api, &thread, "shell-1") == ["running", "completed"]), "its end was seen");
    let live = api.ok("GET", &format!("/api/threads/{thread}/live"), None);
    assert_eq!(live["agent"]["stoppable"], serde_json::json!([]), "{live}");
    assert_eq!(listed_background(&api, &thread), 0, "{live}");
}

/// A subagent the agent left running past its turn keeps the thread in
/// progress until it ends, as the person stopping the agent ends it.
#[test]
fn a_subagent_left_running_keeps_its_thread_in_progress_until_it_ends() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v2", "add acp Aside");
    settled_history(&api, &thread, "Done: add acp Aside");
    assert_eq!(listed_background(&api, &thread), 1, "the subagent still runs, though the turn ended");
    api.ok("POST", &format!("/api/threads/{thread}/agent/cancel"), None);
    assert!(eventually(|| listed_background(&api, &thread) == 0), "the subagent stopped with the agent");
    assert_eq!(task_news(&api, &thread, "aside-1"), ["running", "stopped"]);
}

/// How many commands and subagents the thread list says the thread's agent
/// left running.
fn listed_background(api: &Api, thread: &str) -> u64 {
    let threads = api.ok("GET", "/api/threads", None);
    let listed = threads.as_array().expect("a list").iter().find(|listed| listed["id"] == thread).expect("listed");
    listed["background"].as_u64().expect("a count")
}

/// What the agent left running goes with it, whether the person stops the
/// agent or the daemon stops, and the history says so, also after the
/// daemon starts again.
#[test]
fn what_an_agent_left_running_stops_with_it() {
    let env = Env::new();
    let api = Api::open(&env);
    let stopped = start_fake_acp(&env, &api, "v2", "add acp Keep");
    settled_history(&api, &stopped, "Done: add acp Keep");
    assert!(eventually(|| kept(&env).len() == 1), "the command runs");
    api.ok("POST", &format!("/api/threads/{stopped}/agent/stop"), None);
    assert!(eventually(|| task_news(&api, &stopped, "keep-1") == ["running", "stopped"]), "it stopped with the agent");
    assert!(eventually(|| kept(&env).is_empty()), "its process ended");

    let left = start_fake_acp(&env, &api, "v2", "add acp Keep");
    settled_history(&api, &left, "Done: add acp Keep");
    assert!(eventually(|| kept(&env).len() == 1), "the command runs");
    env.silt(&["daemon", "stop"]);
    assert!(eventually(|| kept(&env).is_empty()), "its process ended with the daemon");
    let api = Api::open(&env);
    assert_eq!(task_news(&api, &left, "keep-1"), ["running", "stopped"]);
    assert!(api.ok("GET", &format!("/api/threads/{left}/live"), None)["agent"].is_null(), "no agent runs it");

    // The machine goes down: the daemon, its agent and the command end at once.
    let downed = start_fake_acp(&env, &api, "v2", "add acp Keep");
    settled_history(&api, &downed, "Done: add acp Keep");
    assert!(eventually(|| kept(&env).len() == 1), "the command runs");
    let processes = silt_os::processes().expect("lists processes");
    let parent = |pid: u32| processes.iter().find(|process| process.pid == pid).and_then(|process| process.parent);
    let command = kept(&env)[0];
    let agent = parent(command).expect("the agent");
    let daemon = parent(agent).expect("the daemon");
    for pid in [daemon, agent, command] {
        silt_os::kill(pid).expect("ends");
    }
    let api = Api::open(&env);
    assert_eq!(task_news(&api, &downed, "keep-1"), ["running", "stopped"], "the daemon starting again says so");
}

/// Copilot over v1 tags its subagent's calls with the subagent's own id and
/// leaves its words untagged while it waits on it: all of it is the
/// subagent's, inside the call that started it.
#[test]
fn a_subagent_of_an_agent_speaking_v1_is_kept_inside_the_call_that_started_it() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v1", "add acp Subagent");
    settled_history(&api, &thread, "Done: add acp Subagent");
    let entries = history_entries(&api, &thread);
    let side = |text: &str| {
        entries
            .iter()
            .find(|entry| entry["text"].as_str().is_some_and(|said| said.starts_with(text)))
            .map(|entry| entry["side"].clone())
    };
    assert_eq!(side("wc -l src/main.rs"), Some(serde_json::json!("task-1")), "its call");
    assert_eq!(side("It has 2 lines."), Some(serde_json::json!("task-1")), "its words");
    let news: Vec<(&str, &str)> =
        entries.iter().filter_map(|entry| Some((entry["task"]["call"].as_str()?, entry["task"]["status"].as_str()?))).collect();
    assert_eq!(news, [("task-1", "running"), ("task-1", "completed")]);
    let call = entries.iter().find(|entry| entry["kind"] == "tool_call" && entry["tool"]["id"] == "task-1").expect("its call");
    assert_eq!((call["tool"]["subject"].as_str(), call["tool"]["detail"].as_str()), (Some("Count lines"), Some("explore")));
    let described: Vec<(Option<&str>, Option<&str>)> = entries
        .iter()
        .filter(|entry| entry["kind"] == "tool_call" && entry["tool"]["id"] == "count-1")
        .map(|entry| (entry["tool"]["subject"].as_str(), entry["tool"]["detail"].as_str()))
        .collect();
    let said = (Some("wc -l src/main.rs"), Some("Count main's lines"));
    assert_eq!(described.last(), Some(&said), "the subagent's call, once updated, is still a command: {described:?}");
}

/// An agent on v1, as Copilot, ignores a cancel when no prompt is out, so
/// once the person stopped it, the agent that goes on by itself is stopped
/// as Stop agent does, and the next prompt starts it again.
#[test]
fn once_stopped_an_agent_speaking_v1_that_goes_on_by_itself_is_stopped() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_fake_acp(&env, &api, "v1", "add acp Detached");
    settled_history(&api, &thread, "Done: add acp Detached");
    api.ok("POST", &format!("/api/threads/{thread}/agent/cancel"), None);
    let stopped = eventually(|| api.ok("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["state"] == "stopped");
    assert!(stopped, "the agent was stopped once it went on");
    let entries = history_entries(&api, &thread);
    let wake = entries.iter().find(|entry| entry["kind"] == "wake").expect("the turn it set off");
    assert!(wake["text"].as_str().is_some_and(|text| text.ends_with("Silt stopped it, as you had stopped the agent.")), "{wake}");
    assert!(!entries.iter().any(|entry| entry["text"] == "The detached shell printed DETACHED."), "it said nothing more");
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
fn files_recorded_before_gitignore_named_them_leave_the_thread_but_not_the_disk() {
    let env = Env::new();
    env.silt(&["claude", "--isolated"]);
    let thread = env.thread();
    let checkout = env.isolated_checkout();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
    fs::create_dir_all(checkout.join(".venv/lib")).expect("mkdir");
    fs::write(checkout.join(".venv/lib/site.py"), "import os\n").expect("an environment made before it's ignored");
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("b/.venv/lib/site.py")));

    fs::write(checkout.join(".gitignore"), ".venv/\n").expect("ignore it");
    assert!(
        eventually(|| {
            let diff = env.stdout(&["diff", &thread]);
            diff.contains("b/.gitignore") && !diff.contains("b/.venv")
        }),
        "{}",
        env.stdout(&["diff", &thread])
    );
    assert_eq!(fs::read_to_string(checkout.join(".venv/lib/site.py")).expect("still there"), "import os\n");
    assert!(env.stdout(&["diff", &thread]).contains("+fn farewell() {}"), "the rest of the thread stays");
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

/// The artifacts of a thread's history, in order.
fn artifacts_of(api: &Api, thread: &str) -> Vec<serde_json::Value> {
    let history = api.ok("GET", &format!("/api/threads/{thread}/history"), None);
    history["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter(|entry| entry["kind"] == "artifact")
        .map(|entry| entry["artifact"].clone())
        .collect()
}

/// An artifact's content, as the web UI reads it.
fn artifact_text(api: &Api, thread: &str, artifact: &serde_json::Value) -> String {
    use base64::Engine as _;
    let blob = artifact["blob"].as_str().expect("blob");
    let content = api.ok("GET", &format!("/api/threads/{thread}/artifacts/{blob}"), None);
    let bytes = base64::engine::general_purpose::STANDARD.decode(content["data"].as_str().expect("data")).expect("base64");
    String::from_utf8(bytes).expect("utf-8")
}

/// An agent the web UI runs shows the person what it made through Silt's
/// own tools (ADR 0068): an artifact, a new version of it under the same
/// id, and an image in the conversation. Each is kept in the thread's
/// history as written (ADR 0071), and served only by the blob an artifact
/// names; a fork inherits them.
#[test]
fn an_agent_the_web_ui_runs_shows_an_image_and_versions_an_artifact() {
    let env = Env::new();
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let html = format!("<h1>Rescan benchmark</h1><p>{secret}</p>");
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake, "env": {"FAKE_ACP_ARTIFACT": html}}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started = api.ok(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": env.repo, "agent": "v1", "prompt": "Artifact please"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    let said = settled_history(&api, &thread, "Done: Artifact please");

    let first = artifacts_of(&api, &thread);
    assert_eq!(first.len(), 1, "{said:?}");
    assert_eq!(
        (&first[0]["title"], &first[0]["kind"], &first[0]["version"]),
        (&"Rescan benchmark".into(), &"html".into(), &1.into())
    );
    assert_eq!(first[0]["inline"], false);
    let text = artifact_text(&api, &thread, &first[0]);
    assert!(text.starts_with("<h1>Rescan benchmark</h1>") && text.contains(secret), "kept as written (ADR 0071): {text}");
    let (served, _) = api.call("GET", &format!("/api/threads/{thread}/artifacts/{}", "ab".repeat(32)), None);
    assert!(!served, "a blob no artifact of the thread names isn't served");

    let sent =
        api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": "Artifact again"})));
    assert!(sent == "sent" || sent == "queued", "{sent}");
    settled_history(&api, &thread, "Done: Artifact again");
    let api_prompt = |text: &str| {
        api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": text})));
        settled_history(&api, &thread, &format!("Done: {text}"));
    };
    api_prompt("Show me the pixel");
    let all = artifacts_of(&api, &thread);
    assert_eq!(all.len(), 3, "{all:?}");
    assert_eq!((&all[1]["id"], &all[1]["version"]), (&first[0]["id"], &2.into()), "a version of the same artifact");
    assert_eq!(
        (&all[2]["inline"], &all[2]["media_type"], &all[2]["caption"]),
        (&true.into(), &"image/png".into(), &"One pixel".into())
    );
    assert_ne!(all[2]["id"], first[0]["id"], "an image shown is its own");

    // A version made by editing the last in place, with a note, and a
    // description later versions keep (ADR 0073).
    api_prompt("Artifact edit");
    let edited = artifacts_of(&api, &thread);
    let latest = edited.iter().rfind(|artifact| artifact["id"] == first[0]["id"]).expect("the artifact");
    assert_eq!((&latest["version"], &latest["note"]), (&3.into(), &"Retitled".into()), "{edited:?}");
    assert_eq!(latest["description"], "How long a rescan takes");
    let text = artifact_text(&api, &thread, latest);
    assert!(text.starts_with("<h1>Rescan benchmark, edited</h1>") && text.contains(secret), "the rest is kept: {text}");

    // The page loads from a frame address with no token (ADR 0074): one
    // version, under the sandbox's policy, as the panel's frame gets it.
    let id = first[0]["id"].as_str().expect("id");
    let made = api.ok(
        "POST",
        &format!("/api/threads/{thread}/artifacts/{id}/frame"),
        Some(serde_json::json!({"version": 1, "theme": "dark", "parent": api.base.trim_end_matches('/')})),
    );
    let path = made["path"].as_str().expect("a frame path");
    let output = Command::new("curl")
        .args(["-s", "-D", "-", &format!("{}{}", api.base.trim_end_matches('/'), path)])
        .output()
        .expect("curl runs");
    let served = String::from_utf8_lossy(&output.stdout).into_owned();
    let (head, page) = served.split_once("\r\n\r\n").unwrap_or((&served, ""));
    assert!(
        head.contains(" 200 ") && head.to_ascii_lowercase().contains("content-security-policy: sandbox allow-scripts;"),
        "{head}"
    );
    assert!(page.contains("setAttribute('data-theme','dark')") && page.ends_with(&html), "{page}");
    let (made_for_image, _) = api.call(
        "POST",
        &format!("/api/threads/{thread}/artifacts/{}/frame", all[2]["id"].as_str().expect("id")),
        Some(serde_json::json!({})),
    );
    assert!(!made_for_image, "only HTML opens in a frame");

    let found = api.ok("GET", "/api/search?q=Rescan", None);
    assert!(found.to_string().contains(&thread[..12]) || found.to_string().contains(&thread), "found by its title: {found}");

    let forked = api.ok("POST", &format!("/api/threads/{thread}/fork"), Some(serde_json::json!({})));
    let fork = forked.as_str().map(str::to_owned).unwrap_or_else(|| forked["thread"].as_str().expect("fork").to_owned());
    let inherited = artifacts_of(&api, &fork);
    assert_eq!(inherited.len(), 4, "a fork inherits what was shown: {inherited:?}");
    assert!(artifact_text(&api, &fork, &inherited[0]).starts_with("<h1>Rescan benchmark</h1>"));
}

/// A page saves itself as the next version for the person, refused over a
/// version that's no longer the latest; it learns who views it; and it asks
/// this machine's Claude Code a question, with no tools (ADR 0072).
#[test]
fn a_page_saves_itself_knows_its_viewer_and_asks_an_agent() {
    let env = Env::new();
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started = api.ok(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": env.repo, "agent": "v1", "prompt": "Artifact saver"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    settled_history(&api, &thread, "Done: Artifact saver");
    let made = artifacts_of(&api, &thread);
    let id = made[0]["id"].as_str().expect("id").to_owned();

    let page = "<!doctype html><html><head></head><body><h1>Saved</h1></body></html>";
    let saved = api.ok(
        "POST",
        &format!("/api/threads/{thread}/artifacts/{id}/publish"),
        Some(serde_json::json!({"html": page, "base": 1})),
    );
    assert_eq!(saved["version"], 2, "{saved}");
    let all = artifacts_of(&api, &thread);
    let latest = all.last().expect("a version");
    assert_eq!((&latest["version"], &latest["by"]), (&2.into(), &"you".into()), "{all:?}");
    assert_eq!(latest["capabilities"], made[0]["capabilities"], "the declaration carries forward");
    assert_eq!(artifact_text(&api, &thread, latest), page);
    let (stale, said) = api.call(
        "POST",
        &format!("/api/threads/{thread}/artifacts/{id}/publish"),
        Some(serde_json::json!({"html": page, "base": 1})),
    );
    assert!(!stale && said["refusal"] == "conflict", "the code a page reads: {said}");

    // The Artifacts page lists it; a pin keeps it first and archiving hides
    // it, for this machine's person, who owns the thread (ADR 0075).
    let listed = api.ok("GET", "/api/artifacts", None);
    let row = listed.as_array().and_then(|rows| rows.iter().find(|row| row["artifact"]["id"] == id)).cloned().expect("listed");
    assert_eq!(
        (&row["artifact"]["version"], &row["pinned"], &row["archived"], &row["mine"]),
        (&2.into(), &false.into(), &false.into(), &true.into()),
        "{row}"
    );
    api.ok("PUT", &format!("/api/threads/{thread}/artifacts/{id}/pin"), Some(serde_json::json!({"pinned": true})));
    api.ok("PUT", &format!("/api/threads/{thread}/artifacts/{id}/archive"), Some(serde_json::json!({"archived": true})));
    let listed = api.ok("GET", "/api/artifacts", None);
    let row = listed.as_array().and_then(|rows| rows.iter().find(|row| row["artifact"]["id"] == id)).cloned().expect("listed");
    assert_eq!((&row["pinned"], &row["archived"]), (&true.into(), &true.into()), "{row}");
    api.ok("PUT", &format!("/api/threads/{thread}/artifacts/{id}/archive"), Some(serde_json::json!({"archived": false})));
    // Its thumbnail is drawn the first time it's asked for, where a headless
    // browser is installed, and kept in the page data for the next time.
    let (drawn, thumbnail) = api.call("GET", &format!("/api/threads/{thread}/artifacts/{id}/thumbnail"), None);
    if drawn {
        assert_eq!(thumbnail["media_type"], "image/jpeg");
        let again = api.ok("GET", &format!("/api/threads/{thread}/artifacts/{id}/thumbnail"), None);
        assert_eq!(again["data"], thumbnail["data"], "kept, not drawn again");
    } else {
        assert!(thumbnail.to_string().contains("headless"), "{thumbnail}");
    }
    let listed = api.ok("GET", "/api/artifacts", None);
    assert!(listed.as_array().is_some_and(|rows| rows.iter().any(|row| row["artifact"]["id"] == id && row["archived"] == false)));

    let viewer = api.ok("GET", &format!("/api/threads/{thread}/viewer"), None);
    assert_eq!((&viewer["id"], &viewer["level"]), (&"local".into(), &"owner".into()), "{viewer}");

    // Nothing is asked until the person allows what the version declares.
    let grants = format!("/api/threads/{thread}/artifacts/{id}/grants");
    let (refused, said) =
        api.call("POST", &format!("/api/threads/{thread}/artifacts/{id}/sample"), Some(serde_json::json!({"input": "hi"})));
    assert!(!refused && said["refusal"] == "not_granted", "{said}");
    api.ok("PUT", &grants, Some(serde_json::json!({"capability": "sample", "state": "granted"})));
    let held = api.ok("GET", &grants, None);
    assert_eq!(held["sample"], "granted", "{held}");
    let answer = api.ok(
        "POST",
        &format!("/api/threads/{thread}/artifacts/{id}/sample"),
        Some(serde_json::json!({"ask": "q1", "input": [{"role": "user", "content": "Name three rivers"}]})),
    );
    assert_eq!((&answer["text"], &answer["truncated"]), (&"Sampled: Name three rivers".into(), &false.into()), "{answer}");
    // A page that declares images shows them to the agent with its question.
    let pixel = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let image = serde_json::json!({"media_type": "image/png", "data": pixel});
    let sample = format!("/api/threads/{thread}/artifacts/{id}/sample");
    let seen = api.ok("POST", &sample, Some(serde_json::json!({"input": "What is this?", "images": [image]})));
    assert_eq!(seen["text"], "Sampled: What is this? (1 images)", "{seen}");
    let svg = serde_json::json!({"media_type": "image/svg+xml", "data": pixel});
    let (refused, said) = api.call("POST", &sample, Some(serde_json::json!({"input": "What is this?", "images": [svg]})));
    assert!(!refused && said.to_string().contains("PNG"), "only images the agent reads: {said}");
    // A tool the page gives its question runs in the page: here the test
    // answers the agent's call of it as the page would.
    let tools = serde_json::json!([{"name": "lookup", "description": "Finds a row by its name"}]);
    let answered = std::thread::scope(|scope| {
        let asking = scope
            .spawn(|| api.ok("POST", &sample, Some(serde_json::json!({"ask": "q3", "input": "Which row?", "tools": tools}))));
        let call = format!("/api/threads/{thread}/artifacts/{id}/sample/q3/tools/1");
        let ran = eventually(|| api.call("POST", &call, Some(serde_json::json!({"ok": true, "value": "row 42"}))).0);
        assert!(ran, "the agent called the page's tool");
        asking.join().expect("asked")
    });
    assert_eq!(answered["text"], "Sampled: Which row? [lookup: row 42]", "{answered}");
    // A tier picks print mode's model; a model's name must be one, never an option.
    let quick = api.ok("POST", &sample, Some(serde_json::json!({"input": "Quickly", "model": {"model_tier": "quick"}})));
    assert_eq!((&quick["text"], &quick["model_applied"]), (&"Sampled: Quickly (on haiku)".into(), &"haiku".into()), "{quick}");
    let (refused, said) = api.call("POST", &sample, Some(serde_json::json!({"input": "Hi", "model": {"model": "--yolo"}})));
    assert!(!refused && said.to_string().contains("isn't a Claude model"), "{said}");
    let (refused, _) = api.call(
        "POST",
        &format!("/api/threads/{thread}/artifacts/{}/sample", "0".repeat(12)),
        Some(serde_json::json!({"input": "hi"})),
    );
    assert!(!refused, "only an artifact that declares sample asks");
}

/// A page that stops asking ends its agent's run: here the run waits on a
/// call of the page's tool, which the page never answers (ADR 0072).
#[test]
fn a_page_that_stops_asking_ends_its_agents_run() {
    let env = Env::new();
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started = api.ok(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": env.repo, "agent": "v1", "prompt": "Artifact saver"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    settled_history(&api, &thread, "Done: Artifact saver");
    let id = artifacts_of(&api, &thread)[0]["id"].as_str().expect("id").to_owned();
    api.ok(
        "PUT",
        &format!("/api/threads/{thread}/artifacts/{id}/grants"),
        Some(serde_json::json!({"capability": "sample", "state": "granted"})),
    );
    let sample = format!("/api/threads/{thread}/artifacts/{id}/sample");
    let tools = serde_json::json!([{"name": "lookup", "description": "Finds a row by its name"}]);
    let began = std::time::Instant::now();
    let (answered, said) = std::thread::scope(|scope| {
        let asking = scope
            .spawn(|| api.call("POST", &sample, Some(serde_json::json!({"ask": "q9", "input": "Which row?", "tools": tools}))));
        // The page stops asking, again until the question it asked has ended, as a stop
        // that reaches the daemon before the question does stops nothing.
        let stop = format!("/api/threads/{thread}/artifacts/{id}/sample/q9/stop");
        while !asking.is_finished() && began.elapsed() < std::time::Duration::from_secs(60) {
            api.ok("POST", &stop, None);
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        asking.join().expect("asked")
    });
    assert!(!answered && said.to_string().contains("stopped asking"), "{said}");
    assert!(began.elapsed() < std::time::Duration::from_secs(60), "well before the run's own limit");
}

/// A page is built from files beside it, which its frame fetches from its
/// own address; a version can change one file alone, and the rest carry
/// over. People add files to the page, served beside it too; and the page
/// reads the thread's files within the globs it declares (ADR 0074, 0078).
#[test]
fn a_page_has_files_beside_it_takes_uploads_and_reads_the_threads_files() {
    use base64::Engine as _;
    let env = Env::new();
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started = api.ok(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": env.repo, "agent": "v1", "prompt": "Artifact files"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    settled_history(&api, &thread, "Done: Artifact files");
    let made = artifacts_of(&api, &thread);
    assert_eq!(made[0]["files"]["data.json"]["media_type"], "application/json", "{made:?}");
    let id = made[0]["id"].as_str().expect("id").to_owned();
    let base = api.base.trim_end_matches('/').to_owned();
    let frame = |version: u32| {
        let made =
            api.ok("POST", &format!("/api/threads/{thread}/artifacts/{id}/frame"), Some(serde_json::json!({"version": version})));
        made["path"].as_str().expect("a frame path").to_owned()
    };
    let fetch = |url: &str| {
        let output = Command::new("curl").args(["-s", "-D", "-", url]).output().expect("curl runs");
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        (head.to_ascii_lowercase(), body.to_owned())
    };
    let first = frame(1);
    let (head, _) = fetch(&format!("{base}{first}"));
    assert!(head.contains(&format!("connect-src {}{first}", base.to_ascii_lowercase())), "{head}");
    let (head, body) = fetch(&format!("{base}{first}data.json"));
    assert!(head.contains(" 200 ") && head.contains("access-control-allow-origin: *"), "{head}");
    assert_eq!(body, r#"{"n": 3}"#);
    assert!(fetch(&format!("{base}{first}missing.json")).0.contains(" 404 "));

    let sent =
        api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": "Artifact files edit"})));
    assert!(sent == "sent" || sent == "queued", "{sent}");
    settled_history(&api, &thread, "Done: Artifact files edit");
    let all = artifacts_of(&api, &thread);
    let second = all.last().expect("a version");
    assert_eq!(second["version"], 2, "{all:?}");
    assert_eq!(second["files"]["css/site.css"], made[0]["files"]["css/site.css"], "an unchanged file carries over");
    assert_eq!(second["blob"], made[0]["blob"], "the page is as it was");
    assert_eq!(fetch(&format!("{base}{}data.json", frame(2))).1, r#"{"n": 4}"#);

    let assets = format!("/api/threads/{thread}/artifacts/{id}/assets");
    let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nfake");
    let added = api.ok(
        "POST",
        &assets,
        Some(serde_json::json!({"op": "add", "name": "chart.png", "media_type": "image/png", "data": png})),
    );
    let url = added["url"].as_str().expect("an asset url");
    let (head, _) = fetch(&format!("{base}{}{url}", frame(2)));
    assert!(
        head.contains(" 200 ") && head.contains("content-type: image/png"),
        "an added file is served beside the page: {head}"
    );
    let (refused, _) = api.call(
        "POST",
        &assets,
        Some(serde_json::json!({"op": "add", "name": "x.html", "media_type": "text/html", "data": png})),
    );
    assert!(!refused, "a page's files are never pages");
    // An SVG someone adds is kept as a picture only.
    let svg = base64::engine::general_purpose::STANDARD.encode(
        br#"<svg xmlns="http://www.w3.org/2000/svg" onload="steal()"><script>steal()</script><rect width="4" height="4"/></svg>"#,
    );
    let drawing = api.ok(
        "POST",
        &assets,
        Some(serde_json::json!({"op": "add", "name": "d.svg", "media_type": "image/svg+xml", "data": svg})),
    );
    let kept =
        api.ok("GET", &format!("/api/threads/{thread}/artifacts/{id}/assets/{}", drawing["id"].as_str().expect("id")), None);
    let kept =
        String::from_utf8(base64::engine::general_purpose::STANDARD.decode(kept["data"].as_str().expect("data")).expect("b64"))
            .expect("utf-8");
    assert!(kept.contains("<rect") && !kept.contains("steal"), "{kept}");
    api.ok("POST", &assets, Some(serde_json::json!({"op": "delete", "id": drawing["id"]})));
    // The agent reads a file someone added, as text when it is text.
    let csv = base64::engine::general_purpose::STANDARD.encode(b"name,score\nada,9\n");
    let table = api.ok(
        "POST",
        &assets,
        Some(serde_json::json!({"op": "add", "name": "scores.csv", "media_type": "text/csv", "data": csv})),
    );
    let asset = table["id"].as_str().expect("an id");
    let text = format!("Read asset {asset}");
    api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": text})));
    let said = settled_history(&api, &thread, &format!("Done: {text}"));
    assert!(said.iter().any(|(_, said)| said.contains("ada,9") && said.contains("text/csv")), "the agent read it: {said:?}");
    api.ok("POST", &assets, Some(serde_json::json!({"op": "delete", "id": asset})));
    let listed = api.ok("POST", &assets, Some(serde_json::json!({"op": "list"})));
    assert_eq!(listed.as_array().map(Vec::len), Some(1), "{listed}");
    api.ok("POST", &assets, Some(serde_json::json!({"op": "delete", "id": added["id"]})));
    assert_eq!(api.ok("POST", &assets, Some(serde_json::json!({"op": "list"}))).as_array().map(Vec::len), Some(0));

    let files = format!("/api/threads/{thread}/artifacts/{id}/files");
    let (refused, said) = api.call("POST", &files, Some(serde_json::json!({"op": "list"})));
    assert!(!refused && said.to_string().contains("not_granted"), "asked first: {said}");
    let grant = serde_json::json!({"capability": "files", "state": "granted"});
    api.ok("PUT", &format!("/api/threads/{thread}/artifacts/{id}/grants"), Some(grant));
    let listed = api.ok("POST", &files, Some(serde_json::json!({"op": "list", "recursive": true})));
    let paths: Vec<&str> =
        listed["entries"].as_array().expect("entries").iter().filter_map(|entry| entry["path"].as_str()).collect();
    assert!(
        paths.contains(&"src/main.rs") && paths.iter().all(|path| path.starts_with("src/")),
        "only what it declares: {paths:?}"
    );
    let read = api.ok("POST", &files, Some(serde_json::json!({"op": "read", "path": "src/main.rs"})));
    assert!(read["text"].as_str().is_some_and(|text| text.contains("fn main")), "{read}");
    let (outside, _) = api.call("POST", &files, Some(serde_json::json!({"op": "read", "path": "README.md"})));
    assert!(!outside, "a file outside its globs is refused");
    let changed = api.ok("POST", &files, Some(serde_json::json!({"op": "history", "path": "src/main.rs"})));
    assert!(changed["path"] == "src/main.rs" && changed["changes"].is_array(), "{changed}");
    let (outside, _) = api.call("POST", &files, Some(serde_json::json!({"op": "history", "path": "README.md"})));
    assert!(!outside, "nor its history");
}

/// A deck is a type whose content is JSON Silt checks (ADR 0076): one that
/// fits its schema is kept, a patch adds a slide in place, and one that
/// doesn't fit is refused.
#[test]
fn a_deck_is_checked_against_its_schema_and_revised_by_a_patch() {
    let env = Env::new();
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started = api.ok(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": env.repo, "agent": "v1", "prompt": "Artifact deck"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    settled_history(&api, &thread, "Done: Artifact deck");
    let made = artifacts_of(&api, &thread);
    assert_eq!((&made[0]["kind"], &made[0]["media_type"]), (&"slides".into(), &"application/json".into()), "{made:?}");

    let prompt = |text: &str| {
        api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": text})));
        settled_history(&api, &thread, &format!("Done: {text}"))
    };
    prompt("Artifact deck more");
    let all = artifacts_of(&api, &thread);
    let latest = all.last().expect("a version");
    assert_eq!(latest["version"], 2, "{all:?}");
    let deck: serde_json::Value = serde_json::from_str(&artifact_text(&api, &thread, latest)).expect("json");
    assert_eq!(deck["slides"].as_array().map(Vec::len), Some(3));
    assert_eq!(deck["slides"][2]["title"], "Next");
    // The deck prints a slide a page, where there's a headless browser.
    let id = latest["id"].as_str().expect("id");
    let page = serde_json::json!({"html": "<!doctype html><style>@page{size:1280px 720px;margin:0}section{height:720px;break-after:page}</style><section>One</section><section>Two</section>"});
    let (printed, pdf) = api.call("POST", &format!("/api/threads/{thread}/artifacts/{id}/pdf"), Some(page));
    if printed {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(pdf["data"].as_str().expect("data")).expect("base64");
        let pages = bytes
            .windows(b"/Type /Page\n".len())
            .filter(|window| window.starts_with(b"/Type /Page") && !window.starts_with(b"/Type /Pages"))
            .count();
        assert!(bytes.starts_with(b"%PDF") && pages >= 2, "a page a slide: {pages}");
    } else {
        assert!(pdf.to_string().contains("headless"), "{pdf}");
    }

    let said = prompt("Artifact bad deck");
    assert_eq!(artifacts_of(&api, &thread).len(), 2, "a deck with no slides isn't kept");
    assert!(said.iter().any(|(_, text)| text.contains("at least one slide")), "the agent hears why: {said:?}");
}

/// A doc and a sheet are edited in place by the person and the agent at
/// once (ADR 0077): the doc's text keeps both people's edits; the sheet's
/// formula keeps meaning the same cells when a row goes in above them, and
/// the sheet downloads as a workbook.
#[test]
fn a_doc_and_a_sheet_are_edited_in_place_by_the_person_and_the_agent() {
    let env = Env::new();
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started = api.ok(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": env.repo, "agent": "v1", "prompt": "Artifact doc"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    settled_history(&api, &thread, "Done: Artifact doc");
    let prompt = |text: &str| {
        api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": text})));
        settled_history(&api, &thread, &format!("Done: {text}"))
    };
    let doc = artifacts_of(&api, &thread)[0]["id"].as_str().expect("id").to_owned();
    let path = format!("/api/threads/{thread}/artifacts/{doc}/doc");
    assert_eq!(api.ok("POST", &path, Some(serde_json::json!({"op": "read"})))["text"], "# Plan\n- ship\n");
    // A comment on "ship" follows it when a line goes in above.
    let on = serde_json::json!({"on": "doc", "artifact": doc, "start": 9, "end": 13});
    api.ok("POST", &format!("/api/threads/{thread}/comments"), Some(serde_json::json!({"on": on, "text": "by when?"})));
    let commented = || {
        let comments = api.ok("GET", &format!("/api/threads/{thread}/comments"), None);
        comments.as_array().and_then(|all| all.iter().find(|comment| comment["text"] == "by when?")).expect("the comment")["location"].clone()
    };
    assert_eq!((&commented()["start"], &commented()["excerpt"]), (&9.into(), &"ship".into()));
    // The person types a line while the agent edits another.
    let typed =
        api.ok("POST", &path, Some(serde_json::json!({"op": "edit", "edits": [{"pos": 7, "delete": 0, "insert": "- test\n"}]})));
    assert_eq!(typed["text"], "# Plan\n- test\n- ship\n");
    let moved = commented();
    assert_eq!(
        (&moved["at"], &moved["start"], &moved["end"], &moved["excerpt"]),
        (&"doc".into(), &16.into(), &20.into(), &"ship".into()),
        "{moved}"
    );
    // The person reads, the agent edits, then the person's edit, counted
    // against what they read, lands where they meant it.
    let seen = api.ok("POST", &path, Some(serde_json::json!({"op": "read"})));
    assert_eq!(seen["text"], "# Plan\n- test\n- ship\n");
    prompt("Doc edit");
    let kept = artifacts_of(&api, &thread);
    let after = kept.iter().rfind(|version| version["id"] == doc).expect("the doc's versions");
    assert_eq!(after["note"], "After the agent's turn", "the agent's edit is kept as a version: {after}");
    let end = "# Plan\n- test\n- ship\n".chars().count();
    let late =
        serde_json::json!({"op": "edit", "base": seen["base"], "edits": [{"pos": end, "delete": 0, "insert": "- demo\n"}]});
    let merged = api.ok("POST", &path, Some(late));
    assert_eq!(merged["text"], "# Plan\n- test\n- ship on Friday\n- demo\n", "both edits, each where it was meant");
    assert_ne!(merged["base"], seen["base"], "the next edit counts from here");
    // Search finds the doc's text as it stands, words the person typed too.
    let found = api.ok("GET", "/api/search?q=test%20frid", None);
    let hit = found.as_array().and_then(|hits| hits.iter().find(|hit| hit["artifact"] == doc.as_str())).expect("a doc hit");
    assert_eq!((&hit["kind"], &hit["thread"]), (&"doc".into(), &thread.as_str().into()), "{hit}");
    assert!(hit["snippet"].as_str().is_some_and(|snippet| snippet.contains("[test]") && snippet.contains("[Friday]")), "{hit}");

    // Save version keeps the doc as it stands, with the person's note.
    let before = artifacts_of(&api, &thread).len();
    let saved = api.ok(
        "POST",
        &format!("/api/threads/{thread}/artifacts/{doc}/versions"),
        Some(serde_json::json!({"note": "Plan agreed"})),
    );
    let versions = artifacts_of(&api, &thread);
    let kept = versions.last().expect("a version");
    assert_eq!(
        (versions.len(), &kept["version"], &saved["version"]),
        (before + 1, &kept["version"], &kept["version"]),
        "{saved}"
    );
    assert_eq!((&kept["note"], &kept["id"]), (&"Plan agreed".into(), &doc.as_str().into()), "{kept}");
    assert_eq!(artifact_text(&api, &thread, kept), "# Plan\n- test\n- ship on Friday\n- demo\n");
    // A PDF of it, printed where there's a headless browser.
    let page = serde_json::json!({"html": "<!doctype html><h1>Plan</h1><ul><li>test</li></ul>"});
    let (printed, pdf) = api.call("POST", &format!("/api/threads/{thread}/artifacts/{doc}/pdf"), Some(page));
    if printed {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(pdf["data"].as_str().expect("data")).expect("base64");
        assert!(pdf["media_type"] == "application/pdf" && bytes.starts_with(b"%PDF"), "a PDF");
    } else {
        assert!(pdf.to_string().contains("headless"), "{pdf}");
    }

    prompt("Artifact sheet");
    let sheet = artifacts_of(&api, &thread).last().expect("the sheet")["id"].as_str().expect("id").to_owned();
    let said = prompt("Sheet set");
    assert!(said.iter().any(|(_, text)| text.contains(r#"["total","8"]"#)), "the agent reads the sum: {said:?}");
    let path = format!("/api/threads/{thread}/artifacts/{sheet}/sheet");
    api.ok("POST", &path, Some(serde_json::json!({"op": "insert", "axis": "rows", "at": 0})));
    api.ok("POST", &path, Some(serde_json::json!({"op": "set", "cells": {"A1": "fruit", "B1": "100"}})));
    let view = api.ok("POST", &path, Some(serde_json::json!({"op": "read"})));
    assert_eq!(view["raw"][3][1], "=SUM(B2:B3)", "the formula moved with its cells: {view}");
    assert_eq!(view["values"][3][1], "8", "and means the same: {view}");
    let said = prompt("Sheet grow");
    assert!(
        said.iter().any(|(_, text)| text.contains(r#"["total","8.0"]"#)),
        "a row in at the top, and B shown with a decimal: {said:?}"
    );
    let grown = api.ok("POST", &path, Some(serde_json::json!({"op": "read"})));
    assert_eq!((&grown["raw"][0][0], &grown["raw"][4][1]), (&"".into(), &"=SUM(B3:B4)".into()), "{grown}");
    api.ok("POST", &path, Some(serde_json::json!({"op": "remove", "axis": "rows", "at": 0})));
    // A second sheet, which a read counts, search finds and the workbook holds.
    api.ok("POST", &path, Some(serde_json::json!({"op": "set", "sheet": 1, "cells": {"A1": "rhubarb", "A2": "=1+1"}})));
    let first = api.ok("POST", &path, Some(serde_json::json!({"op": "read"})));
    assert_eq!(first["sheets"], 2, "{first}");
    let second = api.ok("POST", &path, Some(serde_json::json!({"op": "read", "sheet": 1})));
    assert_eq!((&second["raw"][0][0], &second["values"][1][0]), (&"rhubarb".into(), &"2".into()), "{second}");
    // A formula on the first sheet reads the second, and keeps meaning the same
    // cell when a row goes in at the second's top.
    api.ok("POST", &path, Some(serde_json::json!({"op": "set", "cells": {"D1": "=Sheet2!A2*10"}})));
    let reads = api.ok("POST", &path, Some(serde_json::json!({"op": "read"})));
    assert_eq!((&reads["raw"][0][3], &reads["values"][0][3]), (&"=Sheet2!A2*10".into(), &"20".into()), "{reads}");
    api.ok("POST", &path, Some(serde_json::json!({"op": "insert", "sheet": 1, "axis": "rows", "at": 0})));
    let followed = api.ok("POST", &path, Some(serde_json::json!({"op": "read"})));
    assert_eq!((&followed["raw"][0][3], &followed["values"][0][3]), (&"=Sheet2!A3*10".into(), &"20".into()), "{followed}");
    // A cell typed where the person saw it lands there, though a row went in above meanwhile.
    let seen = api.ok("POST", &path, Some(serde_json::json!({"op": "read", "sheet": 1})));
    let (row, column) = (seen["rows"][0].clone(), seen["columns"][0].clone());
    api.ok("POST", &path, Some(serde_json::json!({"op": "insert", "sheet": 1, "axis": "rows", "at": 0})));
    let typed = serde_json::json!({"op": "set", "sheet": 1, "cells": {"A1": "kept"}, "ids": {"A1": [row, column]}});
    api.ok("POST", &path, Some(typed));
    let now = api.ok("POST", &path, Some(serde_json::json!({"op": "read", "sheet": 1})));
    assert_eq!((&now["raw"][0][0], &now["raw"][1][0]), (&"".into(), &"kept".into()), "{now}");
    // A comment on a cell follows it as a row goes in above.
    let on = serde_json::json!({"on": {"on": "cell", "artifact": sheet, "sheet": 1, "address": "A2"}, "text": "check this"});
    api.ok("POST", &format!("/api/threads/{thread}/comments"), Some(on));
    api.ok("POST", &path, Some(serde_json::json!({"op": "insert", "sheet": 1, "axis": "rows", "at": 0})));
    let comments = api.ok("GET", &format!("/api/threads/{thread}/comments"), None);
    let cell =
        comments.as_array().expect("comments").iter().find(|comment| comment["text"] == "check this").expect("the comment");
    assert_eq!((&cell["location"]["at"], &cell["location"]["address"]), (&"cell".into(), &"A3".into()), "{cell}");
    api.ok("POST", &path, Some(serde_json::json!({"op": "remove", "sheet": 1, "axis": "rows", "at": 0})));
    // A sheet is named, and a column sized and shown as a percentage.
    api.ok("POST", &path, Some(serde_json::json!({"op": "set", "sheet": 2, "cells": {"A1": "0.25"}})));
    let format = serde_json::json!({"op": "format", "sheet": 2, "name": "Shares", "widths": {"A": 140}, "formats": {"A": "0%"}});
    api.ok("POST", &path, Some(format));
    let shares = api.ok("POST", &path, Some(serde_json::json!({"op": "read", "sheet": 2})));
    assert_eq!(
        (&shares["values"][0][0], &shares["widths"][0], &shares["names"][2]),
        (&"25%".into(), &140.into(), &"Shares".into()),
        "{shares}"
    );
    let (refused, _) = api.call("POST", &path, Some(serde_json::json!({"op": "format", "sheet": 2, "formats": {"A": "=1"}})));
    assert!(!refused, "only a number format");
    let (too_many, _) = api.call("POST", &path, Some(serde_json::json!({"op": "read", "sheet": 16})));
    assert!(!too_many, "an artifact has at most 16 sheets");
    for word in ["fruit", "rhubarb"] {
        let found = api.ok("GET", &format!("/api/search?q={word}"), None);
        assert!(
            found
                .as_array()
                .is_some_and(|hits| hits.iter().any(|hit| hit["artifact"] == sheet.as_str() && hit["kind"] == "sheet")),
            "a sheet hit for {word}: {found}"
        );
    }
    let output = Command::new("curl")
        .args([
            "-s",
            "-H",
            &format!("Authorization: Bearer {}", api.token),
            &format!("{}/api/threads/{thread}/artifacts/{sheet}/sheet.xlsx", api.base.trim_end_matches('/')),
        ])
        .output()
        .expect("curl runs");
    assert!(output.stdout.starts_with(b"PK"), "a workbook: {}", String::from_utf8_lossy(&output.stdout));
    assert!(output.stdout.windows(b"worksheets/sheet2.xml".len()).any(|name| name == b"worksheets/sheet2.xml"), "both sheets");
}

/// A page calls a tool of an MCP server the person set up for pages, through
/// their daemon (ADR 0080): only a tool both declared and ticked, and its
/// description says whether it only reads.
#[test]
fn a_page_calls_a_ticked_tool_of_a_page_connector() {
    let env = Env::new();
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started = api.ok(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": env.repo, "agent": "v1", "prompt": "Artifact tools"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    settled_history(&api, &thread, "Done: Artifact tools");
    let id = artifacts_of(&api, &thread)[0]["id"].as_str().expect("id").to_owned();

    // A comment on one of the page's elements keeps its anchor (ADR 0085).
    let on = serde_json::json!({"on": {"on": "element", "artifact": id, "anchor": " #total "}, "text": "round this"});
    api.ok("POST", &format!("/api/threads/{thread}/comments"), Some(on));
    let comments = api.ok("GET", &format!("/api/threads/{thread}/comments"), None);
    let element = comments.as_array().and_then(|all| all.iter().find(|comment| comment["text"] == "round this")).cloned();
    assert_eq!(
        element.as_ref().map(|comment| (&comment["location"]["at"], &comment["location"]["anchor"])),
        Some((&"element".into(), &"#total".into())),
        "{comments}"
    );
    let blank = serde_json::json!({"on": {"on": "element", "artifact": id, "anchor": "a\nb"}, "text": "x"});
    assert!(!api.call("POST", &format!("/api/threads/{thread}/comments"), Some(blank)).0, "an anchor is one line");

    // `silt mcp` is an MCP server whose history tools only read.
    let silt = env!("CARGO_BIN_EXE_silt");
    let connector = serde_json::json!([{"name": "silt", "command": silt, "args": ["mcp", "--thread", thread], "tools": {}}]);
    api.ok("PUT", "/api/page-connectors", Some(connector.clone()));
    let tools = api.ok("GET", "/api/page-connectors/silt/tools", None);
    let search = tools
        .as_array()
        .and_then(|tools| tools.iter().find(|tool| tool["name"] == "silt_history_search"))
        .cloned()
        .expect("listed");
    assert_eq!(search["read_only"], true, "{search}");

    let mcp = format!("/api/threads/{thread}/artifacts/{id}/mcp");
    let grant = serde_json::json!({"capability": "mcp", "state": "granted"});
    api.ok("PUT", &format!("/api/threads/{thread}/artifacts/{id}/grants"), Some(grant));
    let call = serde_json::json!({"op": "call", "server": "silt", "tool": "silt_history_search", "input": {"query": "Artifact"}});
    let (refused, said) = api.call("POST", &mcp, Some(call.clone()));
    assert!(!refused && said.to_string().contains("not_granted"), "not ticked yet: {said}");
    let mut ticked = connector.clone();
    ticked[0]["tools"]["silt_history_search"] = true.into();
    api.ok("PUT", "/api/page-connectors", Some(ticked));
    let described =
        api.ok("POST", &mcp, Some(serde_json::json!({"op": "describe", "server": "silt", "tool": "silt_history_search"})));
    assert_eq!(described["read_only"], true);
    let result = api.ok("POST", &mcp, Some(call));
    assert!(result.to_string().contains("[Artifact] tools"), "the tool answered: {result}");
    let other = serde_json::json!({"op": "call", "server": "silt", "tool": "silt_history_read", "input": {}});
    let (refused, said) = api.call("POST", &mcp, Some(other));
    assert!(!refused && said.to_string().contains("doesn't declare"), "only what the page declares: {said}");
}

/// An agent the web UI runs reads the artifact guide, and its HTML artifact
/// takes the design system the repository names as its default, here Silt's
/// own Nous as its repository keeps it; the CSS its tokens compiled to is
/// served beside the page, a new version keeps it, and the agent reads the
/// latest version back (ADR 0071, ADR 0087).
#[test]
fn an_artifact_takes_the_repositorys_design_system() {
    let env = Env::new();
    let systems = env.repo.join(".silt/design-systems");
    copy_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.silt/design-systems/nous"), &systems.join("nous"));
    fs::write(systems.join("default"), "nous\n").expect("write");
    git(&env.repo, &["add", "."]);
    git(&env.repo, &["commit", "-q", "-m", "design system"]);
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started =
        api.ok("POST", "/api/agents/start", Some(serde_json::json!({"clone": env.repo, "agent": "v1", "prompt": "Guide me"})));
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    let said = settled_history(&api, &thread, "Done: Guide me");
    assert!(said.iter().any(|(_, text)| text.contains("# Making an artifact")), "the guide answered: {said:?}");

    let prompt = |text: &str| {
        api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": text})));
        settled_history(&api, &thread, &format!("Done: {text}"))
    };
    prompt("Artifact please");
    let made = artifacts_of(&api, &thread);
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(made[0]["design_system"], "nous");
    assert_eq!(made[0]["contract"], silt_proto::artifact::CONTRACT, "a page is made for the newest contract, and keeps it");
    let style = made[0]["style"].as_str().expect("a style blob");
    let css = api.ok("GET", &format!("/api/threads/{thread}/artifacts/{style}"), None);
    assert_eq!(css["media_type"], "text/css");
    let css = artifact_text(&api, &thread, &serde_json::json!({"blob": style}));
    assert!(css.contains("--bg: #f6f1e8;") && css.contains(":root[data-theme=\"dark\"]"), "{css}");

    prompt("Artifact again");
    let versions = artifacts_of(&api, &thread);
    assert_eq!((versions.len(), &versions[1]["design_system"]), (2, &"nous".into()), "a new version keeps the last one's");

    let read = prompt("Read back the artifact");
    let answer = read.iter().map(|(_, text)| text.as_str()).find(|text| text.contains("version 2 of 2")).unwrap_or_default();
    assert!(answer.contains("styled with the nous design system") && answer.contains("<h1>Rescan benchmark</h1>"), "{read:?}");

    prompt("Artifact numbers");
    let data = artifacts_of(&api, &thread).into_iter().find(|made| made["kind"] == "data").expect("a data page");
    assert!(data["design_system"] == "nous" && data["style"].is_string(), "a data page's charts take its colours: {data}");
}

/// Settings › Design systems lists the repository's systems and the
/// person's with what picks each, shows one with its components, tokens and
/// assets, and starts an agent that makes one, remembering which thread is
/// making which (ADR 0088).
#[test]
fn design_systems_are_listed_shown_and_made_by_an_agent() {
    let env = Env::new();
    let systems = env.repo.join(".silt/design-systems");
    copy_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.silt/design-systems/nous"), &systems.join("nous"));
    fs::create_dir_all(systems.join("nous/assets/Logos")).expect("mkdir");
    fs::write(systems.join("nous/assets/Logos/mark.svg"), "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").expect("mark");
    fs::write(systems.join("default"), "nous\n").expect("write");
    git(&env.repo, &["add", "."]);
    git(&env.repo, &["commit", "-q", "-m", "design system"]);
    let mine = env.silt_home.join("design-systems/mine");
    fs::create_dir_all(&mine).expect("mkdir");
    fs::write(
        mine.join("tokens.json"),
        r##"{"name":"Mine","color":{"themes":[{"id":"light"}],"tokens":[{"name":"bg","value":"#fff"}]}}"##,
    )
    .expect("tokens");
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");

    let catalog = api.ok("GET", "/api/design-systems/catalog", None);
    let groups = catalog["groups"].as_array().expect("groups");
    let project = &groups[0];
    let nous = &project["systems"][0];
    assert_eq!(
        (nous["name"].as_str(), nous["title"].as_str(), nous["components"].as_u64()),
        (Some("nous"), Some("Nous"), Some(17))
    );
    assert!(nous["uses"][0].as_str().is_some_and(|uses| uses.ends_with("'s default")), "{nous}");
    let personal = groups.last().expect("the person's");
    assert!(personal["project"].is_null() && personal["systems"][0]["name"] == "mine", "{personal}");

    let clone = env.repo.display().to_string();
    let page = api.ok("GET", &format!("/api/design-systems/system?source=project&name=nous&clone={clone}"), None);
    assert!(page["css"].as_str().is_some_and(|css| css.contains(".n-button")), "the frame's CSS");
    assert_eq!(page["components"].as_array().map(Vec::len), Some(17));
    assert!(page["tokens"].as_array().is_some_and(|tokens| tokens.iter().any(|token| token["name"] == "accent")));
    assert_eq!(page["assets"][0]["path"], "Logos/mark.svg");
    let asset =
        api.ok("GET", &format!("/api/design-systems/asset?source=project&name=nous&clone={clone}&path=Logos/mark.svg"), None);
    assert_eq!(asset["media_type"], "image/svg+xml");
    let (refused, _) =
        api.call("GET", &format!("/api/design-systems/asset?source=project&name=nous&clone={clone}&path=../tokens.json"), None);
    assert!(!refused, "only its assets");
    let (refused, _) = api.call("GET", "/api/design-systems/system?source=project&name=nous&clone=/tmp", None);
    assert!(!refused, "only a known clone");

    let (taken, said) = api.call(
        "POST",
        "/api/design-systems/start",
        Some(serde_json::json!({"title": "Nous", "agent": "v1", "keep": "repo", "clone": clone, "from": "code"})),
    );
    assert!(!taken && said.to_string().contains("already"), "{said}");
    let started = api.ok(
        "POST",
        "/api/design-systems/start",
        Some(serde_json::json!({"title": "Harbor", "agent": "v1", "keep": "repo", "clone": clone, "from": "code"})),
    );
    let thread = started.as_str().expect("the thread").to_owned();
    let mut prompt = String::new();
    eventually(|| {
        let history = api.ok("GET", &format!("/api/threads/{thread}/history"), None);
        let first =
            history["entries"].as_array().and_then(|entries| entries.iter().find(|entry| entry["kind"] == "user").cloned());
        prompt = first.and_then(|entry| entry["text"].as_str().map(str::to_owned)).unwrap_or_default();
        !prompt.is_empty()
    });
    assert!(prompt.starts_with("Make a design system called Harbor") && prompt.contains("silt_use_skill"), "{prompt}");
    settled_history(&api, &thread, &format!("Done: {prompt}"));
    let making = api.ok("GET", &format!("/api/threads/{thread}/design-system"), None);
    assert_eq!((making["name"].as_str(), making["keep"].as_str()), (Some("harbor"), Some("repo")), "{making}");
    assert!(making["detail"].is_null(), "nothing written yet");
    let (known, _) = api.call("GET", "/api/threads/none/design-system", None);
    assert!(!known, "an unknown thread is refused");
}

#[test]
fn a_design_system_made_in_a_chat_shows_beside_it_and_can_be_kept() {
    let env = Env::new();
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let prompt = "Do you know how to make a design system?";
    let started = api.ok("POST", "/api/agents/start", Some(serde_json::json!({"agent": "v1", "chat": true, "prompt": prompt})));
    let thread = started.as_str().expect("the thread").to_owned();
    settled_history(&api, &thread, &format!("Done: {prompt}"));
    let chats: Vec<PathBuf> =
        fs::read_dir(env.silt_home.join("chats")).expect("chats").flatten().map(|entry| entry.path()).collect();
    let [folder] = chats.as_slice() else { panic!("one chat folder: {chats:?}") };
    let tokens = |name: &str| {
        format!(r##"{{"name":"{name}","color":{{"themes":[{{"id":"light"}}],"tokens":[{{"name":"bg","value":"#fff"}}]}}}}"##)
    };
    let own = folder.join(".silt/design-systems");
    fs::create_dir_all(own.join("playback")).expect("mkdir");
    fs::write(own.join("playback/tokens.json"), tokens("Playback")).expect("tokens");
    fs::write(own.join("default"), "playback\n").expect("default");

    let made = api.ok("GET", &format!("/api/threads/{thread}/design-system"), None);
    assert_eq!((made["name"].as_str(), made["keep"].as_str()), (Some("playback"), Some("repo")), "the chat's own: {made}");
    assert_eq!(made["detail"]["summary"]["chat"], true, "{made}");

    let catalog = api.ok("GET", "/api/design-systems/catalog", None);
    let groups = catalog["groups"].as_array().expect("groups");
    let chat = groups
        .iter()
        .find(|group| group["chat"]["thread"] == thread.as_str())
        .unwrap_or_else(|| panic!("the chat's group: {catalog}"));
    assert_eq!(chat["systems"][0]["name"], "playback");
    assert_eq!(chat["systems"][0]["uses"][0], "the chat's default");
    let page = api.ok("GET", &format!("/api/design-systems/system?source=project&name=playback&thread={thread}"), None);
    assert_eq!((page["summary"]["chat"].as_bool(), page["summary"]["unmerged"].as_bool()), (Some(true), Some(false)), "{page}");

    let kept = api.ok("POST", "/api/design-systems/keep", Some(serde_json::json!({"thread": thread, "name": "playback"})));
    assert_eq!(kept["source"], "personal", "{kept}");
    assert!(env.silt_home.join("design-systems/playback/tokens.json").is_file(), "kept as the person's own");
    let (again, said) =
        api.call("POST", "/api/design-systems/keep", Some(serde_json::json!({"thread": thread, "name": "playback"})));
    assert!(!again && said.to_string().contains("already"), "{said}");

    std::thread::sleep(Duration::from_millis(20));
    let mut mcp =
        McpClient::spawn(&env, &["mcp", "--thread", &thread, "--checkout", &folder.display().to_string(), "--ui"], folder);
    let skill = mcp.tool("silt_use_skill", serde_json::json!({"name": "design-system"}));
    let personal = env.silt_home.join("design-systems");
    assert!(skill.contains(&format!("`{}`", personal.display())) && skill.contains("This run is a chat"), "{skill}");
    std::thread::sleep(Duration::from_millis(20));
    fs::create_dir_all(personal.join("harbor")).expect("mkdir");
    fs::write(personal.join("harbor/tokens.json"), tokens("Harbor")).expect("tokens");
    let made = api.ok("GET", &format!("/api/threads/{thread}/design-system"), None);
    assert_eq!(
        (made["name"].as_str(), made["keep"].as_str(), made["changing"].as_bool()),
        (Some("harbor"), Some("personal"), Some(false)),
        "what it wrote since loading the skill: {made}"
    );
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    fs::create_dir_all(to).expect("mkdir");
    for entry in fs::read_dir(from).expect("read dir").flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy");
        }
    }
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
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        Self::spawn_with(server["command"].as_str().expect("command"), env, &args, &env.repo)
    }

    /// `silt` with `args`, as an agent the web UI runs would start it.
    fn spawn(env: &Env, args: &[&str], cwd: &Path) -> Self {
        Self::spawn_with(env!("CARGO_BIN_EXE_silt"), env, args, cwd)
    }

    fn spawn_with(command: &str, env: &Env, args: &[&str], cwd: &Path) -> Self {
        let mut child = Command::new(command)
            .args(args)
            .current_dir(cwd)
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

    let listed = mcp.request("tools/list", serde_json::json!({}));
    let names: Vec<&str> =
        listed["result"]["tools"].as_array().expect("tools").iter().filter_map(|tool| tool["name"].as_str()).collect();
    assert!(
        names.contains(&"silt_history_read") && !names.contains(&"silt_show") && !names.contains(&"silt_artifact"),
        "{names:?}"
    );

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
fn a_diff_covers_a_range_of_turns_and_says_which_turn_wrote_each_line() {
    let env = Env::with_claude("tools");
    env.silt(&["claude"]);
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["diff", &thread, "--turn", "2"]).contains("+# Notes")));
    let both = env.stdout(&["diff", &thread, "--turn", "1", "--to-turn", "2"]);
    assert!(both.contains("+fn farewell() {}") && both.contains("+pub fn help() {}") && both.contains("+# Notes"), "{both}");
    let second = env.stdout(&["diff", &thread, "--turn", "2"]);
    assert!(!second.contains("farewell"), "{second}");
    let backwards = env.run(&["diff", &thread, "--turn", "2", "--to-turn", "1"]);
    assert!(String::from_utf8_lossy(&backwards.stderr).contains("comes before turn 2"));

    let api = Api::open(&env);
    let (ok, turns) = api.call("GET", &format!("/api/threads/{thread}/diff/turns"), None);
    assert!(ok, "{turns}");
    let runs =
        |path: &str| turns.as_array().expect("files").iter().find(|file| file["path"] == path).map(|file| file["runs"].clone());
    // The first line came from git, so no turn wrote it.
    assert_eq!(runs("src/main.rs"), Some(serde_json::json!([{"start": 2, "end": 2, "turn": 1}])));
    assert_eq!(runs("NOTES.md"), Some(serde_json::json!([{"start": 1, "end": 1, "turn": 2}])));
    let (_, second) = api.call("GET", &format!("/api/threads/{thread}/diff/turns?turn=2"), None);
    assert_eq!(second.as_array().map(Vec::len), Some(1), "{second}");
}

#[test]
fn a_file_marked_seen_comes_back_with_the_lines_written_since() {
    let env = Env::new();
    env.silt(&["claude", "--isolated"]);
    let thread = env.thread();
    let checkout = env.isolated_checkout();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
    let api = Api::open(&env);
    let seen = || {
        let (ok, view) = api.call("GET", &format!("/api/threads/{thread}/seen"), None);
        assert!(ok, "{view}");
        view
    };
    assert_eq!(seen()["files"], serde_json::json!([]));
    let head = seen()["head"].as_str().expect("a version").to_owned();
    let mark = serde_json::json!({"path": "src/main.rs", "seen": true, "at": head});
    let (ok, marked) = api.call("PUT", &format!("/api/threads/{thread}/seen"), Some(mark));
    assert!(ok, "{marked}");
    assert_eq!(seen()["files"], serde_json::json!([{"path": "src/main.rs", "seen": "seen", "new_lines": []}]));

    // The file changes after: it comes back, with the line written since.
    let main = checkout.join("src/main.rs");
    fs::write(&main, fs::read_to_string(&main).expect("read") + "fn later() {}\n").expect("write");
    assert!(eventually(|| seen()["files"][0]["seen"] == "changed"));
    assert_eq!(seen()["files"][0]["new_lines"], serde_json::json!([{"start": 3, "end": 3}]));

    // A version that isn't the thread's is refused.
    let elsewhere = serde_json::json!({"path": "src/main.rs", "seen": true, "at": "not a version"});
    let (ok, _) = api.call("PUT", &format!("/api/threads/{thread}/seen"), Some(elsewhere));
    assert!(!ok);
}

#[test]
fn a_pending_comments_anchor_follows_its_text_until_it_lands() {
    let env = Env::new();
    env.silt(&["claude", "--isolated"]);
    let thread = env.thread();
    let checkout = env.isolated_checkout();
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
    let api = Api::open(&env);
    let lines = serde_json::json!({"on": "lines", "path": "src/main.rs", "start": 2, "end": 2});
    let (ok, pending) = api.call("POST", &format!("/api/threads/{thread}/comments/anchor"), Some(lines));
    assert!(ok, "{pending}");
    let anchor = pending["anchor"].as_str().expect("an anchor").to_owned();
    let place = || {
        let body = serde_json::json!({"base": null, "anchors": [anchor]});
        let (ok, placed) = api.call("POST", &format!("/api/threads/{thread}/comments/place"), Some(body));
        assert!(ok, "{placed}");
        placed[0].clone()
    };
    assert_eq!(place()["start"], 2);
    let (_, recorded) = api.call("GET", &format!("/api/threads/{thread}/comments"), None);
    assert_eq!(recorded, serde_json::json!([]), "nothing is recorded until the review is sent");

    // A line written above it moves it down with its text.
    let main = checkout.join("src/main.rs");
    fs::write(&main, format!("// header\n{}", fs::read_to_string(&main).expect("read"))).expect("write");
    assert!(eventually(|| place()["start"] == 3));
    let comment = serde_json::json!({"on": {"on": "anchored", "anchor": anchor}, "text": "name it better"});
    let (ok, made) = api.call("POST", &format!("/api/threads/{thread}/comments"), Some(comment));
    assert!(ok, "{made}");
    assert_eq!(made["location"]["start"], 3, "{made}");

    // Once its text is gone it's outdated; an anchor that isn't this thread's isn't placed.
    fs::write(&main, "// header\nfn main() {}\n").expect("write");
    assert!(eventually(|| place()["outdated"] == true));
    let body = serde_json::json!({"base": null, "anchors": ["{\"Code\":{}}", "not json"]});
    let (_, placed) = api.call("POST", &format!("/api/threads/{thread}/comments/place"), Some(body));
    assert_eq!(placed, serde_json::json!([{"at": "unknown"}, {"at": "unknown"}]));
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

/// A sandboxed agent the web UI runs asks for a folder its workspace's
/// rules allow (ADR 0102): it doesn't see it until its turn ends, when Silt
/// starts its sandbox again with the folder, read-only, and tells it what's
/// there. A folder the rules never give stays out.
#[cfg(target_os = "linux")]
#[test]
fn a_sandboxed_agent_asks_for_a_folder_and_is_restarted_with_it() {
    if silt_daemon::sandbox::unavailable(silt_workspace::Backend::Bubblewrap).is_some() {
        eprintln!("skipped: this machine can't make Silt's sandbox");
        return;
    }
    let env = Env::new();
    let shared = tempfile::tempdir().expect("temp dir");
    let shared_root = silt_os::canonicalize(shared.path()).expect("canonical");
    let lib = shared_root.join("lib");
    let secret = shared_root.join("secret");
    for dir in [&lib, &secret] {
        fs::create_dir_all(dir).expect("mkdir");
        fs::write(dir.join("notes.md"), "kept\n").expect("write");
    }
    let rules = format!("{}:allow,{}:never", lib.display(), secret.display());
    let set = env.silt(&["workspace", "policy", "Default", "--silt-sandbox", "on", "--network", "none", "--ask-folders", &rules]);
    assert!(set.status.success(), "{}", String::from_utf8_lossy(&set.stderr));
    let api = Api::open(&env);
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    fs::write(
        env.silt_home.join("agents.json"),
        serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]).to_string(),
    )
    .expect("write agents.json");
    let look = format!("Look in {}", lib.display());
    let started =
        api.ok("POST", "/api/agents/start", Some(serde_json::json!({"clone": env.repo, "agent": "v1", "prompt": look})));
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    settled_history(&api, &thread, &format!("Done: {look}: nothing there"));

    let ask = format!("Folder {}", lib.display());
    api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": ask})));
    // Restarted with it, the agent is told what's there and goes on.
    let told = format!(
        "{} is now in your sandbox, read-only, at that path. You were restarted for this; carry on where you left off.",
        lib.display()
    );
    let mut entries = Vec::new();
    let restarted = eventually(|| {
        let history = api.ok("GET", &format!("/api/threads/{thread}/history"), None);
        entries = history["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| entry["text"].as_str().unwrap_or_default().to_owned())
            .collect();
        let idle = api.ok("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["state"] == "idle";
        // The fake can't load its session in a new process, so the note goes with the conversation it's primed with.
        idle && entries.iter().any(|text| text.starts_with("Done: ") && text.ends_with(&told))
    });
    assert!(restarted, "never restarted with the folder: {entries:?}");
    assert!(entries.iter().any(|text| text == &format!("Done: {ask}")), "{entries:?}");
    let sent = api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": look})));
    assert!(sent == "sent" || sent == "queued", "{sent}");
    settled_history(&api, &thread, &format!("Done: {look}: notes.md"));

    let never = format!("Folder {}", secret.display());
    api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": never})));
    settled_history(&api, &thread, &format!("Done: {never}"));
    let peek = format!("Look in {}", secret.display());
    api.ok("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": peek})));
    settled_history(&api, &thread, &format!("Done: {peek}: nothing there"));
}

/// The same from a terminal: `silt claude` stops the agent once the turn
/// that was granted a folder ends, and starts it again in a sandbox with
/// the folder, resuming its session (ADR 0102).
#[cfg(target_os = "linux")]
#[test]
fn a_terminals_sandboxed_agent_is_restarted_with_the_folder_it_was_granted() {
    if silt_daemon::sandbox::unavailable(silt_workspace::Backend::Bubblewrap).is_some() {
        eprintln!("skipped: this machine can't make Silt's sandbox");
        return;
    }
    let env = Env::with_claude("folder");
    let shared = tempfile::tempdir().expect("temp dir");
    let lib = silt_os::canonicalize(shared.path()).expect("canonical").join("lib");
    fs::create_dir_all(&lib).expect("mkdir");
    fs::write(lib.join("notes.md"), "kept\n").expect("write");
    let rules = format!("{}:allow", lib.display());
    env.silt(&["workspace", "policy", "Default", "--silt-sandbox", "on", "--network", "none", "--ask-folders", &rules]);
    // The sandbox passes only the variables it knows, and those a person lists.
    fs::write(env.silt_home.join("agent-env"), "FAKE_SCRIPT\nFAKE_FOLDER\n").expect("writes agent-env");
    env.silt(&["daemon", "start"]);
    let started = std::time::Instant::now();
    let ran = env.run_with(&["claude"], &[("FAKE_FOLDER", &lib.to_string_lossy())]);
    let said = String::from_utf8_lossy(&ran.stderr).into_owned();
    assert!(ran.status.success(), "{said}");
    assert!(
        said.contains(&format!("restarting Claude Code in its sandbox: {} is now in your sandbox, read-only", lib.display())),
        "{said}"
    );
    assert!(!env.repo.join("folder-not-restarted").exists(), "{said}");
    assert_eq!(fs::read_to_string(env.repo.join("folder-seen")).expect("resumed"), "notes.md", "{said}");
    let told = fs::read_to_string(env.repo.join("folder-told")).expect("resumed");
    assert!(told.starts_with(&format!("{} is now in your sandbox, read-only", lib.display())), "told: {told}");
    assert!(started.elapsed() < std::time::Duration::from_secs(25), "restarted at the turn's end, not after waiting");
}

/// Starts the fake agent speaking `protocol` (`v1` or `v2`) with
/// `FAKE_ACP_STEERING` set to `steering`, or unset when it's empty.
fn start_steerable(env: &Env, api: &Api, protocol: &str, steering: &str, prompt: &str) -> String {
    let fake = env.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let mut vars = serde_json::json!({});
    if protocol == "v2" {
        vars["FAKE_ACP_V2"] = "1".into();
    }
    if !steering.is_empty() {
        vars["FAKE_ACP_STEERING"] = steering.into();
    }
    let agents = serde_json::json!([{"id": "steerable", "name": "Fake steerable", "command": fake, "env": vars}]);
    fs::write(env.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started =
        api.ok("POST", "/api/agents/start", Some(serde_json::json!({"clone": env.repo, "agent": "steerable", "prompt": prompt})));
    started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("the thread").to_owned())
}

/// Steers `thread` once its agent runs the command of a `Long` prompt.
fn steer_while_the_command_runs(api: &Api, thread: &str, text: &str) {
    let running = eventually(|| serde_json::Value::Array(history_entries(api, thread)).to_string().contains("cargo build"));
    assert!(running, "the command never started: {:?}", history_entries(api, thread));
    let answer = api.ok("POST", &format!("/api/threads/{thread}/agent/steer"), Some(serde_json::json!({"text": text})));
    assert_eq!(answer, "steering");
}

/// A steer to an agent that takes them (`_session/steering`) goes into the
/// turn it runs: the command it runs goes on to its end, the agent hears the
/// steer while it runs, and the history has it as the person's message
/// there, not as a cancelled turn and a new prompt (ADR 0111).
#[test]
fn a_steer_reaches_an_agent_that_takes_steers_and_leaves_its_command_running() {
    let env = Env::new();
    let api = Api::open(&env);
    for protocol in ["v2", "v1"] {
        let thread = start_steerable(&env, &api, protocol, "injected", "Long build");
        steer_while_the_command_runs(&api, &thread, "use the release profile");
        let entries = settled_history(&api, &thread, "Done: Long build");
        let all = serde_json::Value::Array(history_entries(&api, &thread)).to_string();
        assert!(all.contains("The command ran to its end."), "{protocol}: the command was left to run: {all}");
        assert!(!all.contains("The command was killed."), "{protocol}: {all}");
        assert!(!all.contains("I interrupted"), "{protocol}: the turn wasn't interrupted: {all}");
        let said = said(&entries);
        let steer = said.iter().position(|&(kind, text)| kind == "user" && text == "use the release profile");
        let heard = said.iter().position(|&(kind, text)| {
            kind == "assistant" && text.contains("Heard while the command ran: use the release profile")
        });
        assert!(steer.is_some() && heard.is_some() && steer < heard, "{protocol}: {said:?}");
        assert_eq!(said.iter().filter(|&&(kind, _)| kind == "user").count(), 2, "{protocol}: one prompt and one steer: {said:?}");
    }
}

/// An agent that doesn't take steers, or refuses one, has its turn
/// cancelled, which kills the command it runs, and hears the steer as its
/// next prompt, as before steering was taken (ADR 0111).
#[test]
fn a_steer_an_agent_doesnt_take_cancels_its_turn_and_follows_it() {
    let env = Env::new();
    let api = Api::open(&env);
    for steering in ["", "failed", "error"] {
        let thread = start_steerable(&env, &api, "v2", steering, "Long build");
        steer_while_the_command_runs(&api, &thread, "stop and use the release profile");
        let interrupted = "I interrupted your last turn to say: stop and use the release profile";
        let entries = settled_history(&api, &thread, &format!("Done: {interrupted}"));
        let all = serde_json::Value::Array(history_entries(&api, &thread)).to_string();
        assert!(all.contains("The command was killed."), "{steering:?}: {all}");
        let said = said(&entries);
        assert!(said.contains(&("user", interrupted)), "{steering:?}: {said:?}");
        assert!(!said.iter().any(|(_, text)| text.contains("Heard while")), "{steering:?}: {said:?}");
    }
}

/// A steer that finds no turn running, as one sent as the turn ends, comes
/// back to Silt (`promptRequired`): the command runs to its end and the
/// steer is the next prompt, as the person wrote it.
#[test]
fn a_steer_that_finds_no_turn_is_sent_as_the_next_prompt() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_steerable(&env, &api, "v2", "promptRequired", "Long build");
    steer_while_the_command_runs(&api, &thread, "then run the tests");
    let entries = settled_history(&api, &thread, "Done: then run the tests");
    let all = serde_json::Value::Array(history_entries(&api, &thread)).to_string();
    assert!(all.contains("The command ran to its end."), "{all}");
    assert!(!all.contains("I interrupted"), "{all}");
    let said = said(&entries);
    let users: Vec<&str> = said.iter().filter(|(kind, _)| *kind == "user").map(|(_, text)| *text).collect();
    assert_eq!(users, ["Long build", "then run the tests"], "{said:?}");
    let ended = said.iter().position(|&(_, text)| text == "Done: Long build");
    let steered = said.iter().position(|&(_, text)| text == "then run the tests");
    assert!(ended < steered, "the steer came once the turn ended: {said:?}");
}

/// A steer while the agent asks a question still answers it in the
/// person's own words, without cancelling the turn: the question is
/// declined, and the steer, framed as its answer, goes to the agent (here
/// once the turn ends, as it finds none running).
#[test]
fn a_steer_while_an_agent_that_takes_steers_asks_answers_its_question() {
    let env = Env::new();
    let api = Api::open(&env);
    let thread = start_steerable(&env, &api, "v1", "injected", "Ask me what to count");
    let live = |api: &Api| api.ok("GET", &format!("/api/threads/{thread}/live"), None);
    assert!(eventually(|| live(&api)["agent"]["questions"].as_array().is_some_and(|asked| !asked.is_empty())), "{}", live(&api));
    api.ok("POST", &format!("/api/threads/{thread}/agent/steer"), Some(serde_json::json!({"text": "count the chunks"})));
    let framed = "to your question \"What should the progress bar count?\": count the chunks";
    let entries = settled_history(&api, &thread, &format!("Done: {framed}"));
    let said = said(&entries);
    assert!(said.contains(&("assistant", "You didn't choose.")), "the question was declined, not cancelled: {said:?}");
    assert!(said.contains(&("user", framed)), "{said:?}");
    let all = serde_json::Value::Array(history_entries(&api, &thread)).to_string();
    assert!(!all.contains("I interrupted"), "{all}");
}
