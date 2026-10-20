//! Two machines and a Silt server, all real processes: a thread recorded on
//! a laptop continues on a home machine with its conversation and files.

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Records a first session, or on `--resume` checks that the restored
/// transcript is where Claude looks for it and points at this directory.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
set -eu
session=""
resume=""
while [ $# -gt 0 ]; do
    case "$1" in
        --session-id) session="$2"; shift 2 ;;
        --resume) session="$2"; resume=1; shift 2 ;;
        --settings) shift 2 ;;
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
    say '{"type":"user","message":{"role":"user","content":"Continuing at home: add a home line"}}'
    hook UserPromptSubmit toolu_2
    say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_2","name":"Edit","input":{"file_path":"src/main.rs"}}]}}'
    hook PreToolUse toolu_2
    printf 'fn home() {}\n' >> src/main.rs
    hook PostToolUse toolu_2
    say '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_2","content":"edited","is_error":false}]}}'
    hook Stop toolu_2
    exit 0
fi
mkdir -p "$dir"
say '{"type":"ai-title","aiTitle":"Add a farewell","sessionId":"x"}'
say "{\"type\":\"user\",\"cwd\":\"$(pwd)\",\"message\":{\"role\":\"user\",\"content\":\"Please add a farewell line to main\"}}"
hook UserPromptSubmit toolu_1
say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Edit","input":{"file_path":"src/main.rs"}}]}}'
hook PreToolUse toolu_1
printf 'fn farewell() {}\n' >> src/main.rs
hook PostToolUse toolu_1
say '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"edited","is_error":false}]}}'
say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Done: farewell added. I used ghp_abcdefghijklmnopqrstuvwxyz0123 to fetch it."}]}}'
hook Stop toolu_1
"#;

const WAIT: Duration = Duration::from_secs(30);

fn eventually(mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn silt_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_silt"));
    command.env_remove("SILT_HOME").env_remove("CLAUDE_CONFIG_DIR");
    command
}

struct Server {
    data: PathBuf,
    url: String,
    listen: String,
    child: Option<Child>,
}

impl Server {
    fn start(data: &Path) -> Self {
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

    fn run(&mut self) {
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

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn pairing_code(&self) -> String {
        let output = silt_command()
            .args(["server", "pair", "--data"])
            .arg(&self.data)
            .output()
            .expect("pair runs");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Machine {
    silt_home: PathBuf,
    claude_dir: PathBuf,
    repo: PathBuf,
    fake: PathBuf,
}

impl Machine {
    fn new(root: &Path, name: &str, origin: &Path, server: &Server) -> Self {
        let home = root.join(name);
        let silt_home = home.join("silt");
        let claude_dir = home.join("claude");
        fs::create_dir_all(&claude_dir).expect("mkdir");
        git(root, &["clone", "-q", &origin.to_string_lossy(), &home.join("repo").to_string_lossy()]);
        let fake = home.join("claude-bin");
        fs::write(&fake, FAKE_CLAUDE).expect("write fake claude");
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod");
        let machine = Self {
            silt_home,
            claude_dir,
            repo: home.join("repo").canonicalize().expect("canonical"),
            fake,
        };
        let code = server.pairing_code();
        machine.silt(&["auth", "login", "--server", &server.url, "--code", &code, "--name", name]);
        machine
    }

    fn run(&self, args: &[&str]) -> Output {
        silt_command()
            .args(args)
            .current_dir(&self.repo)
            .env("SILT_HOME", &self.silt_home)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("SILT_CLAUDE_BIN", &self.fake)
            .env("SILT_HOOK", format!("{} hook", env!("CARGO_BIN_EXE_silt")))
            .output()
            .expect("silt runs")
    }

    fn silt(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "silt {args:?} failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn stdout(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&self.silt(args).stdout).into_owned()
    }

    fn log_line(&self, thread: &str) -> Option<String> {
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

struct World {
    root: tempfile::TempDir,
    server: Server,
    laptop: Machine,
    home: Machine,
}

/// An origin with one pushed commit, and a laptop clone with one more that
/// only the laptop has.
fn world() -> World {
    let root = tempfile::Builder::new().prefix("silt-sync").tempdir_in("/tmp").expect("temp dir");
    let seed = root.path().join("seed");
    fs::create_dir_all(seed.join("src")).expect("mkdir");
    git(&seed, &["init", "-q", "-b", "main"]);
    fs::write(seed.join("src/main.rs"), "fn main() {}\n").expect("write");
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "init"]);
    let origin = root.path().join("origin.git");
    git(root.path(), &["clone", "-q", "--bare", &seed.to_string_lossy(), &origin.to_string_lossy()]);

    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let home = Machine::new(root.path(), "home", &origin, &server);
    fs::write(laptop.repo.join("src/lib.rs"), "pub fn wip() {}\n").expect("write");
    git(&laptop.repo, &["add", "."]);
    git(&laptop.repo, &["commit", "-q", "-m", "wip, not pushed"]);
    World {
        root,
        server,
        laptop,
        home,
    }
}

fn thread_of(machine: &Machine) -> String {
    machine
        .stdout(&["log"])
        .split_whitespace()
        .next()
        .expect("a thread is listed")
        .to_owned()
}

fn checkout_from(stderr: &[u8]) -> PathBuf {
    let stderr = String::from_utf8_lossy(stderr);
    let line = stderr.lines().find(|line| line.contains("is now here, in ")).expect("resume reports the checkout");
    PathBuf::from(line.split("is now here, in ").nth(1).expect("path"))
}

#[test]
fn a_thread_continues_on_another_machine() {
    let world = world();
    let (laptop, home) = (&world.laptop, &world.home);
    laptop.silt(&["claude"]);
    let thread = thread_of(laptop);
    assert!(fs::read_to_string(laptop.repo.join("src/main.rs")).expect("read").contains("fn farewell"));

    assert!(
        eventually(|| home.log_line(&thread).is_some_and(|line| line.contains("remote") && line.contains("2 msgs"))),
        "the home machine sees the laptop's thread: {}",
        home.stdout(&["log"])
    );
    let resumed = home.silt(&["resume", &thread]);
    let checkout = checkout_from(&resumed.stderr);
    assert!(checkout.starts_with(&home.repo));
    let main = fs::read_to_string(checkout.join("src/main.rs")).expect("read");
    assert_eq!(main, "fn main() {}\nfn farewell() {}\nfn home() {}\n");
    assert_eq!(
        fs::read_to_string(checkout.join("src/lib.rs")).expect("the unpushed commit came in a bundle"),
        "pub fn wip() {}\n"
    );
    assert_eq!(fs::read_to_string(home.repo.join("src/main.rs")).expect("read"), "fn main() {}\n");

    assert!(eventually(|| home.stdout(&["diff", &thread]).contains("+fn home() {}")));
    let show = home.stdout(&["show", &thread]);
    assert!(show.contains("> Please add a farewell line to main"), "{show}");
    assert!(show.contains("> Continuing at home: add a home line"), "{show}");
    assert!(show.contains("I used [REDACTED] to fetch it"), "{show}");
    assert!(laptop.stdout(&["show", &thread]).contains("ghp_abcdefghijklmnopqrstuvwxyz0123"));
    assert!(!dir_contains(&world.server.data, b"ghp_abcdefghijklmnopqrstuvwxyz0123"), "secrets stay off the server");
    assert!(home.log_line(&thread).is_some_and(|line| line.contains("here")));

    assert!(
        eventually(|| laptop.stdout(&["show", &thread]).contains("Continuing at home")),
        "the laptop sees what happened at home"
    );
    assert!(laptop.log_line(&thread).is_some_and(|line| line.contains("moved")), "{}", laptop.stdout(&["log"]));
    let log = laptop.stdout(&["log"]);
    assert!(
        log.lines().any(|line| line.contains(&format!("fork of {thread}")) && line.contains("here")),
        "the laptop's checkout continues in a fork: {log}"
    );
    assert!(!fs::read_to_string(laptop.repo.join("src/main.rs")).expect("read").contains("fn home"));

}

#[test]
fn sync_recovers_from_server_restarts_and_rollbacks() {
    let mut world = world();
    let backup = world.root.path().join("server-backup");
    world.laptop.silt(&["claude"]);
    let thread = thread_of(&world.laptop);
    assert!(eventually(|| world.home.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))));

    world.server.stop();
    copy_dir(&world.server.data, &backup);
    world.server.run();
    let status = world.laptop.stdout(&["auth", "status"]);
    assert!(status.contains(&world.server.url), "{status}");

    world.home.silt(&["resume", &thread]);
    assert!(eventually(|| world.laptop.stdout(&["show", &thread]).contains("Continuing at home")));

    world.server.stop();
    fs::remove_dir_all(&world.server.data).expect("remove server data");
    copy_dir(&backup, &world.server.data);
    world.server.run();
    let machine = Machine {
        silt_home: world.root.path().join("fresh"),
        claude_dir: world.root.path().join("fresh-claude"),
        repo: world.home.repo.clone(),
        fake: world.home.fake.clone(),
    };
    fs::create_dir_all(&machine.claude_dir).expect("mkdir");
    let code = world.server.pairing_code();
    machine.silt(&["auth", "login", "--server", &world.server.url, "--code", &code, "--name", "fresh"]);
    assert!(
        eventually(|| machine.log_line(&thread).is_some_and(|line| line.contains("3 msgs"))),
        "after a rollback the clients push what the server lost: {}",
        machine.stdout(&["log"])
    );
}

fn dir_contains(dir: &Path, needle: &[u8]) -> bool {
    fs::read_dir(dir).expect("read dir").flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            dir_contains(&path, needle)
        } else {
            fs::read(&path).is_ok_and(|bytes| bytes.windows(needle.len()).any(|window| window == needle))
        }
    })
}

fn copy_dir(from: &Path, to: &Path) {
    let status = Command::new("cp").arg("-a").arg(from).arg(to).status().expect("cp runs");
    assert!(status.success());
}

#[test]
fn resume_here_uses_the_current_clone() {
    let world = world();
    let (laptop, home) = (&world.laptop, &world.home);
    laptop.silt(&["claude"]);
    let thread = thread_of(laptop);
    assert!(eventually(|| home.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))));

    fs::write(home.repo.join("scratch.txt"), "untracked\n").expect("write");
    let refused = home.run(&["resume", "--here", &thread]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("clean working tree"));
    assert!(!laptop.stdout(&["log"]).contains("moved"), "a refused resume leaves the lease alone");
    fs::remove_file(home.repo.join("scratch.txt")).expect("remove");

    let resumed = home.silt(&["resume", "--here", &thread]);
    assert_eq!(checkout_from(&resumed.stderr), home.repo);
    assert_eq!(
        fs::read_to_string(home.repo.join("src/main.rs")).expect("read"),
        "fn main() {}\nfn farewell() {}\nfn home() {}\n"
    );
    assert_eq!(git(&home.repo, &["log", "-1", "--format=%s"]).trim(), "wip, not pushed");
}
