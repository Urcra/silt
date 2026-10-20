use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use exn::{Exn, ResultExt};
use serde::de::DeserializeOwned;
use silt_agents::claude;
use silt_daemon::client;
use silt_daemon::paths;
use silt_daemon::protocol::{DaemonStatus, Request, Response, Resumed, SearchResult, SessionStarted, ShowLine, ThreadSummary};

use crate::CliError;
use crate::render;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const HOOK_TIMEOUT: Duration = Duration::from_secs(20);

fn fail(message: impl Into<String>) -> Exn<CliError> {
    Exn::new(CliError(message.into()))
}

/// Sends a request to the running daemon and decodes its answer.
fn call<T: DeserializeOwned>(request: &Request) -> Result<T, Exn<CliError>> {
    let response = client::send(&paths::socket_path(), request, REQUEST_TIMEOUT)
        .or_raise(|| CliError("the Silt daemon is not answering; try `silt daemon start`".into()))?;
    match response {
        Response::Ok { data } => {
            serde_json::from_value(data).or_raise(|| CliError("unexpected answer from the daemon".into()))
        }
        Response::Error { message } => Err(fail(message)),
    }
}

fn is_running() -> bool {
    client::send(&paths::socket_path(), &Request::Ping, Duration::from_secs(2)).is_ok()
}

/// Starts the daemon in the background unless it already runs.
pub fn ensure_daemon() -> Result<bool, Exn<CliError>> {
    if is_running() {
        return Ok(false);
    }
    fs::create_dir_all(paths::data_dir()).or_raise(|| CliError("cannot create the Silt data directory".into()))?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::log_path())
        .or_raise(|| CliError("cannot open the daemon log".into()))?;
    let log_err = log.try_clone().or_raise(|| CliError("cannot open the daemon log".into()))?;
    let exe = env::current_exe().or_raise(|| CliError("cannot find the silt executable".into()))?;
    {
        use std::os::unix::process::CommandExt;
        Command::new(exe)
            .args(["daemon", "run"])
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(log_err)
            .process_group(0)
            .spawn()
            .or_raise(|| CliError("cannot start the Silt daemon".into()))?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if is_running() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(fail(format!(
        "the Silt daemon did not start; see {}",
        paths::log_path().display()
    )))
}

pub fn daemon_run() -> Result<(), Exn<CliError>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("SILT_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(io::stderr)
        .init();
    let runtime = tokio::runtime::Runtime::new().or_raise(|| CliError("cannot start the async runtime".into()))?;
    runtime
        .block_on(silt_daemon::run(&paths::data_dir(), &paths::socket_path()))
        .or_raise(|| CliError("the Silt daemon failed".into()))
}

pub fn daemon_stop() -> Result<(), Exn<CliError>> {
    if !is_running() {
        println!("not running");
        return Ok(());
    }
    call::<String>(&Request::Shutdown)?;
    println!("stopped");
    Ok(())
}

pub fn daemon_status() -> Result<(), Exn<CliError>> {
    if is_running() {
        println!("running ({})", paths::socket_path().display());
    } else {
        println!("not running");
    }
    Ok(())
}

pub fn claude(isolated: bool, args: &[String]) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let cwd = env::current_dir().or_raise(|| CliError("cannot read the current directory".into()))?;
    let session_id = uuid::Uuid::new_v4().to_string();
    let started: SessionStarted = call(&Request::StartSession {
        cwd,
        agent: claude::AGENT.into(),
        session_id: session_id.clone(),
        isolated,
    })?;
    eprintln!("silt: recording thread {} in {}", short(&started.thread), started.checkout.display());

    let status = run_claude(&["--session-id", &session_id], args, &started.agent_cwd)?;
    call::<String>(&Request::EndSession {
        agent: claude::AGENT.into(),
        session_id,
    })?;
    eprintln!("silt: thread {} recorded; see `silt show {}`", short(&started.thread), short(&started.thread));
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Runs Claude Code with Silt's hooks installed.
fn run_claude(session_args: &[&str], args: &[String], cwd: &Path) -> Result<std::process::ExitStatus, Exn<CliError>> {
    let exe = env::current_exe().or_raise(|| CliError("cannot find the silt executable".into()))?;
    let hook_command = format!("{} hook", shell_quote(&exe));
    let claude_bin = env::var_os("SILT_CLAUDE_BIN").unwrap_or_else(|| "claude".into());
    Command::new(&claude_bin)
        .args(session_args)
        .args(["--settings", &claude::hook_settings(&hook_command)])
        .args(args)
        .current_dir(cwd)
        .status()
        .or_raise(|| CliError(format!("cannot run {}", claude_bin.to_string_lossy())))
}

pub fn resume(thread: &str, here: bool, args: &[String]) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let cwd = env::current_dir().or_raise(|| CliError("cannot read the current directory".into()))?;
    let resumed: Resumed = call(&Request::Resume {
        thread: thread.into(),
        cwd,
        here,
    })?;
    eprintln!("silt: thread {} is now here, in {}", short(&resumed.thread), resumed.checkout.display());
    let Some(session_id) = resumed.session_id else {
        eprintln!("silt: the thread has no agent session to continue");
        return Ok(());
    };
    let status = run_claude(&["--resume", &session_id], args, &resumed.agent_cwd)?;
    call::<String>(&Request::EndSession {
        agent: claude::AGENT.into(),
        session_id,
    })?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

pub fn auth_login(server: &str, code: &str, name: Option<String>) -> Result<(), Exn<CliError>> {
    let name = name
        .or_else(|| fs::read_to_string("/etc/hostname").ok().map(|name| name.trim().to_owned()))
        .unwrap_or_else(|| "device".into());
    let runtime = tokio::runtime::Runtime::new().or_raise(|| CliError("cannot start the async runtime".into()))?;
    let config = runtime
        .block_on(silt_daemon::remote::login(server, code, &name))
        .or_raise(|| CliError(format!("could not log in to {server}")))?;
    silt_daemon::remote::save_config(&paths::data_dir(), &config)
        .or_raise(|| CliError("could not save the server settings".into()))?;
    if is_running() {
        call::<String>(&Request::Reload)?;
    }
    println!("logged in to {} as device {} ({name})", config.url, config.device);
    Ok(())
}

pub fn auth_status() -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let status: DaemonStatus = call(&Request::Status)?;
    match status.server {
        Some(server) => {
            println!("syncing with {server} as device {}", status.device.unwrap_or_default());
            if let Some(error) = status.sync_error {
                println!("last sync error: {error}");
            }
            for thread in &status.diverged {
                println!(
                    "thread {} differs from the server's copy (was Silt's data here restored from a backup?) and no longer syncs from here",
                    short(thread)
                );
            }
        }
        None => println!("not connected to a server; use `silt auth login`"),
    }
    Ok(())
}

pub fn track() -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let cwd = env::current_dir().or_raise(|| CliError("cannot read the current directory".into()))?;
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    let config = claude::config_dir().ok_or_else(|| fail("HOME is not set"))?;
    let project = config.join("projects").join(claude::project_slug(&cwd));
    let session_id = newest_transcript(&project)
        .and_then(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
        .ok_or_else(|| fail(format!("no Claude Code session found for {}", cwd.display())))?;
    let started: SessionStarted = call(&Request::StartSession {
        cwd,
        agent: claude::AGENT.into(),
        session_id,
        isolated: false,
    })?;
    println!("tracking session in thread {}", short(&started.thread));
    Ok(())
}

fn newest_transcript(project: &Path) -> Option<PathBuf> {
    fs::read_dir(project)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "jsonl"))
        .max_by_key(|entry| entry.metadata().and_then(|metadata| metadata.modified()).ok())
        .map(|entry| entry.path())
}

/// Forwards an agent hook to the daemon. Never fails, so a missing daemon
/// can't break the agent session.
pub fn hook(event: &str) {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(input) = serde_json::from_str(&input) else {
        return;
    };
    let request = Request::Hook {
        event: event.to_owned(),
        input,
    };
    let _ = client::send(&paths::socket_path(), &request, HOOK_TIMEOUT);
}

pub fn log() -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let threads: Vec<ThreadSummary> = call(&Request::Threads)?;
    print!("{}", render::threads(&threads));
    Ok(())
}

pub fn show(thread: &str) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let lines: Vec<ShowLine> = call(&Request::Show { thread: thread.into() })?;
    print!("{}", render::show(&lines));
    Ok(())
}

pub fn diff(thread: &str, turn: Option<usize>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let diff: String = call(&Request::Diff {
        thread: thread.into(),
        turn,
    })?;
    print!("{diff}");
    Ok(())
}

pub fn search(query: &str) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let hits: Vec<SearchResult> = call(&Request::Search {
        query: query.into(),
        limit: 20,
    })?;
    print!("{}", render::search(&hits));
    Ok(())
}

fn short(thread: &str) -> &str {
    &thread[thread.len().saturating_sub(12)..]
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn server_data(data: Option<PathBuf>) -> PathBuf {
    data.unwrap_or_else(|| paths::data_dir().join("server"))
}

pub fn server_run(listen: &str, data: Option<PathBuf>) -> Result<(), Exn<CliError>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("SILT_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(io::stderr)
        .init();
    let data = server_data(data);
    let runtime = tokio::runtime::Runtime::new().or_raise(|| CliError("cannot start the async runtime".into()))?;
    runtime.block_on(async {
        let server = silt_server::Server::open(&data)
            .await
            .or_raise(|| CliError(format!("cannot open server data in {}", data.display())))?;
        let listener = tokio::net::TcpListener::bind(listen)
            .await
            .or_raise(|| CliError(format!("cannot listen on {listen}")))?;
        eprintln!("silt server listening on {listen}, data in {}", data.display());
        server.serve(listener).await.or_raise(|| CliError("the server stopped".into()))
    })
}

pub fn server_pair(data: Option<PathBuf>) -> Result<(), Exn<CliError>> {
    let data = server_data(data);
    let runtime = tokio::runtime::Runtime::new().or_raise(|| CliError("cannot start the async runtime".into()))?;
    let code = runtime
        .block_on(silt_server::create_pairing_code(&data))
        .or_raise(|| CliError("cannot create a pairing code".into()))?;
    println!("{code}");
    eprintln!("valid for 15 minutes; on the device run: silt auth login --server <url> --code {code}");
    Ok(())
}
