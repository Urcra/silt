//! Real processes for multi-machine tests: a Silt server, and machines
//! with their own Silt and Claude Code homes, a clone and a scripted
//! stand-in for Claude Code.
#![allow(dead_code, reason = "each test file uses part of the harness")]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

pub mod fakes;

/// Records a first session, or on `--resume` checks that the restored
/// transcript is where Claude looks for it and points at this directory.
pub const FAKE_CLAUDE: &str = r#"#!/bin/sh
set -eu
session=""
resume=""
while [ $# -gt 0 ]; do
    case "$1" in
        --session-id) session="$2"; shift 2 ;;
        --resume) session="$2"; resume=1; shift 2 ;;
        --settings) shift 2 ;;
        --append-system-prompt) printf '%s' "$2" > "$FAKE_LOG_DIR/claude-instructions"; shift 2 ;;
        --plugin-dir) ls "$2/skills" > "$FAKE_LOG_DIR/claude-skills"; shift 2 ;;
        *) shift ;;
    esac
done
slug=$(pwd | sed 's/[^A-Za-z0-9]/-/g')
dir="$CLAUDE_CONFIG_DIR/projects/$slug"
transcript="$dir/$session.jsonl"
hook() {
    printf '{"session_id":"%s","hook_event_name":"%s","tool_name":"Edit","tool_use_id":"%s"}' "$session" "$1" "$2" | $SILT_HOOK "$1"
}
say() {
    printf '%s\n' "$1" >> "$transcript"
}
if [ -n "$resume" ]; then
    grep -q "\"cwd\":\"$(pwd)\"" "$transcript" || { echo "transcript not restored for $(pwd)" >&2; exit 3; }
    grep -q "Please add a farewell line" "$transcript" || { echo "conversation missing" >&2; exit 3; }
    if [ -z "${FAKE_TRANSLATED:-}" ]; then
        saved=$(sed -n 's/.*saved to \([^"]*\)".*/\1/p' "$transcript" | head -n 1)
        [ "$saved" = "$dir/$session/tool-results/t1.txt" ] || { echo "tool result path not rewritten: $saved" >&2; exit 4; }
        grep -q "big output" "$saved" || { echo "tool result missing" >&2; exit 4; }
    fi
    grep -q "\"sessionId\":\"$session\"" "$transcript" || { echo "session id not rewritten" >&2; exit 5; }
    for needle in ${FAKE_EXPECT:-}; do
        needle=$(printf '%s' "$needle" | tr '-' ' ')
        grep -q "$needle" "$transcript" || { echo "transcript lacks $needle" >&2; cat "$transcript" >&2; exit 6; }
    done
    printf '%s' "$session" > "$CLAUDE_CONFIG_DIR/last-session"
    say "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"${FAKE_SAY:-Continuing at home: add a home line}\"}}"
    hook UserPromptSubmit toolu_2 > "$CLAUDE_CONFIG_DIR/last-hook-output"
    say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_2","name":"Edit","input":{"file_path":"src/main.rs"}}]}}'
    hook PreToolUse toolu_2
    printf '%s\n' "${FAKE_LINE:-fn home() {\}}" >> src/main.rs
    hook PostToolUse toolu_2
    say '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_2","content":"edited","is_error":false}]}}'
    hook Stop toolu_2
    exit 0
fi
mkdir -p "$dir"
say '{"type":"ai-title","aiTitle":"Add a farewell","sessionId":"x"}'
say "{\"type\":\"user\",\"cwd\":\"$(pwd)\",\"sessionId\":\"$session\",\"message\":{\"role\":\"user\",\"content\":\"Please add a farewell line to main\"}}"
hook UserPromptSubmit toolu_1
say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Edit","input":{"file_path":"src/main.rs"}}]}}'
hook PreToolUse toolu_1
printf 'fn farewell() {}\n' >> src/main.rs
hook PostToolUse toolu_1
mkdir -p "$dir/$session/tool-results"
printf 'big output\n' > "$dir/$session/tool-results/t1.txt"
say "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_1\",\"content\":\"saved to $dir/$session/tool-results/t1.txt\"}]}}"
say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Done: farewell added. I used ghp_abcdefghijklmnopqrstuvwxyz0123 to fetch it."}]}}'
hook Stop toolu_1
"#;

pub const WAIT: Duration = Duration::from_secs(30);

pub fn eventually(mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn silt_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_silt"));
    command.env_remove("SILT_HOME").env_remove("CLAUDE_CONFIG_DIR");
    command
}

pub struct Server {
    pub data: PathBuf,
    pub url: String,
    listen: String,
    child: Option<Child>,
}

impl Server {
    pub fn start(data: &Path) -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("free port")
            .local_addr()
            .expect("address")
            .port();
        let listen = format!("127.0.0.1:{port}");
        let mut server = Self {
            data: data.to_owned(),
            url: format!("http://{listen}"),
            listen,
            child: None,
        };
        server.run();
        server
    }

    pub fn run(&mut self) {
        let child = silt_command()
            .args(["server", "run", "--listen", &self.listen, "--data"])
            .arg(&self.data)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("server starts");
        self.child = Some(child);
        assert!(eventually(|| std::net::TcpStream::connect(&self.listen).is_ok()), "server listens");
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn pairing_code(&self) -> String {
        self.pairing_code_for("owner")
    }

    pub fn pairing_code_for(&self, user: &str) -> String {
        let output = silt_command()
            .args(["server", "pair", "--user", user, "--data"])
            .arg(&self.data)
            .output()
            .expect("pair runs");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// Runs `silt server <args> --data <this server's data>` on the host.
    pub fn admin(&self, args: &[&str]) -> String {
        let output = silt_command()
            .arg("server")
            .args(args)
            .arg("--data")
            .arg(&self.data)
            .output()
            .expect("silt server runs");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct Machine {
    pub silt_home: PathBuf,
    pub claude_dir: PathBuf,
    pub repo: PathBuf,
    pub fake: PathBuf,
    /// Homes of the other agents and the fakes' logs.
    pub agents: PathBuf,
}

impl Machine {
    pub fn new(root: &Path, name: &str, origin: &Path, server: &Server) -> Self {
        Self::new_for(root, name, "owner", origin, server)
    }

    /// A machine of `user`, with a clone of `origin`, logged in to `server`.
    pub fn new_for(root: &Path, name: &str, user: &str, origin: &Path, server: &Server) -> Self {
        let home = root.join(name);
        let silt_home = home.join("silt");
        let claude_dir = home.join("claude");
        fs::create_dir_all(&claude_dir).expect("mkdir");
        git(root, &["clone", "-q", &origin.to_string_lossy(), &home.join("repo").to_string_lossy()]);
        let fake = home.join("claude-bin");
        fs::write(&fake, FAKE_CLAUDE).expect("write fake claude");
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod");
        let agents = home.join("agents");
        for (name, script) in [
            ("codex", fakes::FAKE_CODEX),
            ("gemini", fakes::FAKE_GEMINI),
            ("copilot", fakes::FAKE_COPILOT),
            ("opencode", fakes::FAKE_OPENCODE),
        ] {
            fs::create_dir_all(agents.join(name)).expect("mkdir");
            let bin = agents.join(format!("{name}-bin"));
            fs::write(&bin, script).expect("write fake agent");
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        fs::create_dir_all(agents.join("logs")).expect("mkdir");
        let machine = Self {
            silt_home,
            claude_dir,
            repo: home.join("repo").canonicalize().expect("canonical"),
            fake,
            agents,
        };
        let code = server.pairing_code_for(user);
        machine.silt(&["auth", "login", "--server", &server.url, "--code", &code, "--name", name]);
        machine
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.run_with(args, &[])
    }

    pub fn run_with(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        silt_command()
            .args(args)
            .current_dir(&self.repo)
            .env("SILT_HOME", &self.silt_home)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("SILT_CLAUDE_BIN", &self.fake)
            .env("SILT_CODEX_BIN", self.agents.join("codex-bin"))
            .env("SILT_GEMINI_BIN", self.agents.join("gemini-bin"))
            .env("SILT_COPILOT_BIN", self.agents.join("copilot-bin"))
            .env("SILT_OPENCODE_BIN", self.agents.join("opencode-bin"))
            .env("CODEX_HOME", self.agents.join("codex"))
            .env("GEMINI_CLI_HOME", self.agents.join("gemini"))
            .env("COPILOT_HOME", self.agents.join("copilot"))
            .env("FAKE_OPENCODE_DIR", self.agents.join("opencode"))
            .env("FAKE_LOG_DIR", self.agents.join("logs"))
            .env("HOME", self.agents.parent().expect("the machine's home"))
            .env("SILT_HOOK", format!("{} hook", env!("CARGO_BIN_EXE_silt")))
            .envs(envs.iter().copied())
            .output()
            .expect("silt runs")
    }

    pub fn silt(&self, args: &[&str]) -> Output {
        self.silt_with(args, &[])
    }

    pub fn silt_with(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        let output = self.run_with(args, envs);
        assert!(
            output.status.success(),
            "silt {args:?} failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    /// Output of a command that may fail for now, for polling.
    pub fn try_stdout(&self, args: &[&str]) -> String {
        let output = self.run(args);
        if output.status.success() {
            String::from_utf8_lossy(&output.stdout).into_owned()
        } else {
            String::new()
        }
    }

    pub fn stdout(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&self.silt(args).stdout).into_owned()
    }

    pub fn log_line(&self, thread: &str) -> Option<String> {
        self.stdout(&["log"]).lines().find(|line| line.starts_with(thread)).map(str::to_owned)
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = silt_command()
            .args(["daemon", "stop"])
            .env("SILT_HOME", &self.silt_home)
            .output();
    }
}


/// An origin with one pushed commit, `main.rs` holding `fn main() {}`.
pub fn origin(root: &Path) -> PathBuf {
    let seed = root.join("seed");
    fs::create_dir_all(seed.join("src")).expect("mkdir");
    git(&seed, &["init", "-q", "-b", "main"]);
    fs::write(seed.join("src/main.rs"), "fn main() {}\n").expect("write");
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "init"]);
    let origin = root.join("origin.git");
    git(root, &["clone", "-q", "--bare", &seed.to_string_lossy(), &origin.to_string_lossy()]);
    origin
}

pub fn thread_of(machine: &Machine) -> String {
    machine
        .stdout(&["log"])
        .split_whitespace()
        .next()
        .expect("a thread is listed")
        .to_owned()
}

/// The checkout path `silt resume` or `silt fork` reported.
pub fn checkout_from(stderr: &[u8]) -> PathBuf {
    let stderr = String::from_utf8_lossy(stderr);
    let line = stderr
        .lines()
        .find(|line| line.contains("is now here, in ") || line.contains(", in "))
        .expect("the command reports the checkout");
    PathBuf::from(line.rsplit(", in ").next().expect("path").trim_start_matches("is now here, in "))
}
