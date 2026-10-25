//! Real processes for multi-machine tests: a Silt server, and machines
//! with their own Silt and Claude Code homes, a clone and a scripted
//! stand-ins for the agent CLIs, `silt-fake-agent`'s programs, which run everywhere.
#![allow(dead_code, reason = "each test file uses part of the harness")]

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The stand-in for an agent CLI, `silt-fake-<agent>`, built beside `silt`
/// by `cargo build` or `cargo test` of the workspace.
pub fn fake_agent(agent: &str) -> PathBuf {
    let silt = Path::new(env!("CARGO_BIN_EXE_silt"));
    let fake = silt.with_file_name(format!("silt-fake-{agent}{}", std::env::consts::EXE_SUFFIX));
    assert!(fake.is_file(), "{} isn't built; run `cargo build -p silt-fake-agent`", fake.display());
    fake
}

pub fn fake_claude() -> PathBuf {
    fake_agent("claude")
}

/// `path` with links resolved, as a shell's `pwd` names it: on Windows
/// without the `\\?\` prefix `canonicalize` adds, which agents never see.
pub fn canonical(path: &Path) -> PathBuf {
    silt_os::canonicalize(path).expect("canonical")
}

/// A directory for one test's machines: under `/tmp` on Unix, which keeps
/// the daemons' socket paths short enough, else where the system says.
pub fn temp_root(prefix: &str) -> tempfile::TempDir {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    if cfg!(unix) { builder.tempdir_in("/tmp") } else { builder.tempdir() }.expect("temp dir")
}

pub const WAIT: Duration = Duration::from_secs(30);

/// Whether `check` holds within [`WAIT`]. A check that panics, as a command
/// that fails while the daemon catches up does, counts as not yet.
pub fn eventually(mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut check)).unwrap_or(false) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Runs a script a test just wrote. Another test forking while it was being
/// written can hold it open for a moment, which fails the exec.
pub fn output_once_released(command: &mut Command) -> Output {
    (0..50)
        .find_map(|_| match command.output() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(20));
                None
            }
            other => Some(other),
        })
        .expect("the script was released")
        .expect("the script runs")
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
    /// A server on a free port. The port is free when picked but could be
    /// taken before the server binds it, so a server that can't listen is
    /// started again on another.
    pub fn start(data: &Path) -> Self {
        for _ in 0..5 {
            let port = TcpListener::bind("127.0.0.1:0").expect("free port").local_addr().expect("address").port();
            let listen = format!("127.0.0.1:{port}");
            let mut server = Self { data: data.to_owned(), url: format!("http://{listen}"), listen, child: None };
            if server.try_run() {
                return server;
            }
        }
        panic!("the server never listened: {}", fs::read_to_string(Self::log_path(data)).unwrap_or_default());
    }

    /// Starts the server again on its port, which a restart keeps.
    pub fn run(&mut self) {
        assert!(self.try_run(), "the server listens: {}", fs::read_to_string(Self::log_path(&self.data)).unwrap_or_default());
    }

    fn log_path(data: &Path) -> PathBuf {
        data.with_extension("log")
    }

    /// Whether the server listens, rather than exiting first. It says so
    /// once it has the port: something else answering there may be another
    /// test's server that took the port first.
    fn try_run(&mut self) -> bool {
        let log_path = Self::log_path(&self.data);
        let before = fs::metadata(&log_path).map_or(0, |log| usize::try_from(log.len()).unwrap_or(usize::MAX));
        let announced = format!("listening on {}", self.listen);
        let log = fs::OpenOptions::new().create(true).append(true).open(&log_path).expect("server log");
        let mut child = silt_command()
            .args(["server", "run", "--listen", &self.listen, "--data"])
            .arg(&self.data)
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("server starts");
        let says_so = || {
            fs::read(&log_path)
                .is_ok_and(|log| log.get(before..).is_some_and(|new| String::from_utf8_lossy(new).contains(&announced)))
        };
        let listens = eventually(|| says_so() || child.try_wait().is_ok_and(|exited| exited.is_some()))
            && child.try_wait().is_ok_and(|exited| exited.is_none());
        if listens {
            self.child = Some(child);
        }
        listens
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
        let output =
            silt_command().args(["server", "pair", "--user", user, "--data"]).arg(&self.data).output().expect("pair runs");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// Runs `silt server <args> --data <this server's data>` on the host.
    pub fn admin(&self, args: &[&str]) -> String {
        let output = silt_command().arg("server").args(args).arg("--data").arg(&self.data).output().expect("silt server runs");
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
        let fake = fake_claude();
        let agents = home.join("agents");
        for name in ["codex", "gemini", "copilot", "opencode"] {
            fs::create_dir_all(agents.join(name)).expect("mkdir");
        }
        fs::create_dir_all(agents.join("logs")).expect("mkdir");
        let machine = Self { silt_home, claude_dir, repo: canonical(&home.join("repo")), fake, agents };
        let code = server.pairing_code_for(user);
        machine.silt(&["auth", "login", "--server", &server.url, "--code", &code, "--name", name]);
        machine
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.run_with(args, &[])
    }

    pub fn run_with(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        self.command(args, envs).output().expect("silt runs")
    }

    /// `silt` with `args` as this machine runs it, to start and watch.
    pub fn command(&self, args: &[&str], envs: &[(&str, &str)]) -> Command {
        let mut command = silt_command();
        command
            .args(args)
            .current_dir(&self.repo)
            .env("SILT_HOME", &self.silt_home)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("SILT_CLAUDE_BIN", &self.fake)
            .env("SILT_CODEX_BIN", fake_agent("codex"))
            .env("SILT_GEMINI_BIN", fake_agent("gemini"))
            .env("SILT_COPILOT_BIN", fake_agent("copilot"))
            .env("SILT_OPENCODE_BIN", fake_agent("opencode"))
            .env("CODEX_HOME", self.agents.join("codex"))
            .env("GEMINI_CLI_HOME", self.agents.join("gemini"))
            .env("COPILOT_HOME", self.agents.join("copilot"))
            .env("FAKE_OPENCODE_DIR", self.agents.join("opencode"))
            .env("FAKE_LOG_DIR", self.agents.join("logs"))
            .env("HOME", self.agents.parent().expect("the machine's home"))
            .envs(envs.iter().copied());
        command
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
        if output.status.success() { String::from_utf8_lossy(&output.stdout).into_owned() } else { String::new() }
    }

    pub fn stdout(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&self.silt(args).stdout).into_owned()
    }

    pub fn log_line(&self, thread: &str) -> Option<String> {
        self.stdout(&["log"]).lines().find(|line| line.starts_with(thread)).map(str::to_owned)
    }

    /// Asks this machine's daemon over HTTP, as the web UI does; panics
    /// unless it answers 200.
    pub fn web(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> serde_json::Value {
        let url = self.stdout(&["open", "--no-browser"]);
        let (base, token) = web_ui(&url);
        let mut command = Command::new("curl");
        command
            .args(["-s", "-w", "\n%{http_code}", "-X", method, "-H", "Content-Type: application/json", "-H"])
            .arg(format!("Authorization: Bearer {token}"));
        if let Some(body) = body {
            command.arg("--data").arg(body.to_string());
        }
        let output = command.arg(format!("{base}{path}")).output().expect("curl runs");
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let (json, status) = text.rsplit_once('\n').expect("a status line");
        assert_eq!(status, "200", "{method} {path}: {json}");
        serde_json::from_str(json).unwrap_or(serde_json::Value::Null)
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = silt_command().args(["daemon", "stop"]).env("SILT_HOME", &self.silt_home).output();
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
    machine.stdout(&["log"]).split_whitespace().next().expect("a thread is listed").to_owned()
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

/// The daemon's address and token from `silt open`'s URL, which names the
/// project it ran in when that's a clone.
pub fn web_ui(url: &str) -> (&str, &str) {
    let (page, token) = url.trim().split_once("#token=").expect("a URL with a token");
    (page.split_once("/?").map_or(page.trim_end_matches('/'), |(base, _project)| base), token)
}
