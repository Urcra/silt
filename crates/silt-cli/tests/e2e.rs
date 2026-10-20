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
while [ $# -gt 0 ]; do
    case "$1" in
        --session-id) session="$2"; shift 2 ;;
        --settings) shift 2 ;;
        *) shift ;;
    esac
done
slug=$(pwd | sed 's/[^A-Za-z0-9]/-/g')
dir="$CLAUDE_CONFIG_DIR/projects/$slug"
mkdir -p "$dir"
transcript="$dir/$session.jsonl"
hook() {
    printf '{"session_id":"%s","hook_event_name":"%s","tool_name":"Edit","tool_use_id":"toolu_1"}' "$session" "$1" | $SILT_HOOK "$1"
}
printf '%s\n' '{"type":"ai-title","aiTitle":"Add a greeting","sessionId":"x"}' >> "$transcript"
printf '%s\n' '{"type":"user","message":{"role":"user","content":"Please add a farewell line to main"}}' >> "$transcript"
hook UserPromptSubmit
printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Adding it now."},{"type":"tool_use","id":"toolu_1","name":"Edit","input":{"file_path":"src/main.rs"}}]}}' >> "$transcript"
hook PreToolUse
printf 'fn farewell() {}\n' >> src/main.rs
hook PostToolUse
printf '%s\n' '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"edited","is_error":false}]}}' >> "$transcript"
printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Done: farewell added."}]}}' >> "$transcript"
printf '%s\n' '{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}' >> "$transcript"
printf '%s\n' '{"type":"user","isCompactSummary":true,"message":{"role":"user","content":"Summary: farewell was added to main."}}' >> "$transcript"
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
        fs::write(&fake, FAKE_CLAUDE).expect("write fake claude");
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
        let output = Command::new(env!("CARGO_BIN_EXE_silt"))
            .args(args)
            .current_dir(&self.repo)
            .env("SILT_HOME", &self.silt_home)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("SILT_CLAUDE_BIN", &self.fake)
            .env("SILT_HOOK", format!("{} hook", env!("CARGO_BIN_EXE_silt")))
            .output()
            .expect("silt runs");
        assert!(
            output.status.success(),
            "silt {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
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
        let _ = Command::new(env!("CARGO_BIN_EXE_silt"))
            .args(["daemon", "stop"])
            .env("SILT_HOME", &self.silt_home)
            .output();
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

fn eventually(mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if check() {
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
    assert!(log.contains("Add a greeting"), "{log}\n{}\n{}", env.stdout(&["show", &thread]), fs::read_to_string(env.silt_home.join("daemon.log")).unwrap_or_default());
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
fn isolated_sessions_work_in_a_managed_checkout() {
    let env = Env::new();
    env.silt(&["claude", "--isolated"]);
    let thread = env.thread();
    let checkout = env.repo.join(".silt/checkouts").join(
        fs::read_dir(env.repo.join(".silt/checkouts"))
            .expect("checkouts")
            .flatten()
            .next()
            .expect("one checkout")
            .file_name(),
    );
    assert!(fs::read_to_string(checkout.join("src/main.rs")).expect("read").contains("fn farewell"));
    assert_eq!(fs::read_to_string(env.repo.join("src/main.rs")).expect("read"), "fn main() {}\n");
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
}

#[test]
fn track_attaches_to_a_session_started_without_silt() {
    let env = Env::new();
    let output = Command::new(&env.fake)
        .args(["--session-id", "11111111-2222-4333-8444-555555555555"])
        .current_dir(&env.repo)
        .env("CLAUDE_CONFIG_DIR", &env.claude_dir)
        .env("SILT_HOME", &env.silt_home)
        .env("SILT_HOOK", "true")
        .output()
        .expect("fake claude runs");
    assert!(output.status.success());

    assert!(env.stdout(&["track"]).contains("tracking session"));
    let thread = env.thread();
    assert!(eventually(|| env.stdout(&["show", &thread]).contains("> Please add a farewell line to main")));
    assert!(eventually(|| env.stdout(&["diff", &thread]).contains("+fn farewell() {}")));
}
