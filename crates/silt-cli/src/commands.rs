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
use silt_daemon::protocol::{
    CommentOn, CommentView, DaemonStatus, HistoryPage, HookReply, InFlight, MrDetails, MrSummary, Pulled, Request, Response, Resumed, SearchFilters,
    SearchResult, SessionStarted, ShowLine, ThreadSummary,
};

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

    let status = run_claude(&["--session-id", &session_id], args, &started.agent_cwd, &started.thread)?;
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

/// Runs Claude Code with Silt's hooks and history tools installed.
fn run_claude(
    session_args: &[&str],
    args: &[String],
    cwd: &Path,
    thread: &str,
) -> Result<std::process::ExitStatus, Exn<CliError>> {
    let exe = env::current_exe().or_raise(|| CliError("cannot find the silt executable".into()))?;
    let hook_command = format!("{} hook", shell_quote(&exe));
    let mcp_config = serde_json::json!({
        "mcpServers": { "silt": { "command": exe, "args": ["mcp", "--thread", thread] } }
    });
    let claude_bin = env::var_os("SILT_CLAUDE_BIN").unwrap_or_else(|| "claude".into());
    Command::new(&claude_bin)
        .args(session_args)
        .args(["--mcp-config", &mcp_config.to_string()])
        .args(["--settings", &claude::hook_settings(&hook_command)])
        .args(args)
        .current_dir(cwd)
        .status()
        .or_raise(|| CliError(format!("cannot run {}", claude_bin.to_string_lossy())))
}

pub fn resume(thread: &str, here: bool, args: &[String]) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let resumed: Resumed = call(&Request::Resume {
        thread: thread.into(),
        cwd: current_dir()?,
        here,
    })?;
    eprintln!("silt: thread {} is now here, in {}", short(&resumed.thread), resumed.checkout.display());
    continue_session(resumed, args)
}

pub fn fork(thread: &str, here: bool, args: &[String]) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let forked: Resumed = call(&Request::Fork {
        thread: thread.into(),
        cwd: current_dir()?,
        here,
    })?;
    eprintln!(
        "silt: forked {} as thread {}, in {}",
        short(thread),
        short(&forked.thread),
        forked.checkout.display()
    );
    continue_session(forked, args)
}

fn current_dir() -> Result<PathBuf, Exn<CliError>> {
    env::current_dir().or_raise(|| CliError("cannot read the current directory".into()))
}

/// Runs the agent on a resumed or forked session, if the thread has one.
fn continue_session(resumed: Resumed, args: &[String]) -> Result<(), Exn<CliError>> {
    let Some(session_id) = resumed.session_id else {
        eprintln!("silt: the thread has no agent session to continue");
        return Ok(());
    };
    let status = run_claude(&["--resume", &session_id], args, &resumed.agent_cwd, &resumed.thread)?;
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
    println!(
        "logged in to {} as {}, device {} ({name})",
        config.url, config.user, config.device
    );
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

/// Forwards an agent hook to the daemon, and prints any context the daemon
/// has for the agent in Claude Code's hook output format. Never fails, so a
/// missing daemon can't break the agent session.
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
    let Ok(Response::Ok { data }) = client::send(&paths::socket_path(), &request, HOOK_TIMEOUT) else {
        return;
    };
    if let Ok(HookReply {
        additional_context: Some(context),
    }) = serde_json::from_value(data)
    {
        let output = serde_json::json!({
            "hookSpecificOutput": { "hookEventName": event, "additionalContext": context }
        });
        println!("{output}");
    }
}

pub fn pull(thread: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let pulled: Pulled = call(&Request::Pull {
        thread,
        cwd: current_dir()?,
    })?;
    if pulled.messages == 0 && pulled.files.is_empty() {
        println!("thread {} is up to date with {}", short(&pulled.thread), short(&pulled.from));
    } else {
        println!(
            "pulled {} messages and {} changed files from {} into {}",
            pulled.messages,
            pulled.files.len(),
            short(&pulled.from),
            short(&pulled.thread)
        );
        for file in &pulled.files {
            println!("  {file}");
        }
    }
    Ok(())
}

pub fn mr_create(thread: Option<String>, title: Option<String>, summary: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let mr: MrSummary = call(&Request::MrCreate {
        thread,
        cwd: current_dir()?,
        title,
        summary,
    })?;
    println!("opened merge request {}: {} → {}", mr.id, short(&mr.fork), short(&mr.target));
    Ok(())
}

pub fn mr_list(thread: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let requests: Vec<MrSummary> = call(&Request::MrList { thread })?;
    print!("{}", render::merge_requests(&requests));
    Ok(())
}

pub fn mr_show(mr: &str) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let details: MrDetails = call(&Request::MrShow { mr: mr.into() })?;
    print!("{}", render::merge_request(&details));
    Ok(())
}

pub fn mr_approve(mr: &str) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let approved: MrSummary = call(&Request::MrApprove { mr: mr.into() })?;
    println!("merged {} into thread {}", approved.id, short(&approved.target));
    Ok(())
}

pub fn mr_decline(mr: &str, reason: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let declined: MrSummary = call(&Request::MrDecline { mr: mr.into(), reason })?;
    println!("declined {}; fork {} carries on", declined.id, short(&declined.fork));
    Ok(())
}

pub fn visibility(visibility: silt_proto::Visibility, thread: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let message: String = call(&Request::SetVisibility {
        cwd: current_dir()?,
        thread,
        visibility,
    })?;
    println!("{message}");
    Ok(())
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

pub fn search(query: &str, filters: SearchFilters) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let hits: Vec<SearchResult> = call(&Request::Search {
        query: query.into(),
        limit: 20,
        filters,
    })?;
    print!("{}", render::search(&hits));
    Ok(())
}

pub fn in_flight(path: &str) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let found: Vec<InFlight> = call(&Request::InFlight {
        cwd: current_dir()?,
        path: path.into(),
    })?;
    print!("{}", render::in_flight(&found));
    Ok(())
}

pub fn history(thread: &str, offset: usize, limit: usize) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let page: HistoryPage = call(&Request::HistoryRead {
        thread: thread.into(),
        offset,
        limit,
    })?;
    print!("{}", render::history(&page));
    Ok(())
}

pub fn comment(
    thread: &str,
    file: Option<String>,
    lines: Option<&str>,
    entry: Option<usize>,
    text: &str,
) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let on = match (file, lines, entry) {
        (Some(path), Some(lines), None) => {
            let (start, end) = match lines.split_once('-') {
                Some((start, end)) => (start.trim().parse(), end.trim().parse()),
                None => (lines.trim().parse(), lines.trim().parse()),
            };
            let (Ok(start), Ok(end)) = (start, end) else {
                return Err(fail(format!("--lines takes a line or a range like 10-14, not {lines:?}")));
            };
            CommentOn::Lines { path, start, end }
        }
        (None, None, Some(index)) => CommentOn::Entry { index },
        _ => return Err(fail("comment on either --file with --lines, or --entry")),
    };
    let comment: CommentView = call(&Request::Comment {
        thread: thread.into(),
        on,
        text: text.into(),
        cwd: current_dir()?,
    })?;
    println!("added comment {} at {}", comment.id, render::comment_location(&comment.location));
    Ok(())
}

pub fn comments(thread: &str, resolve: Option<&str>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    if let Some(comment) = resolve {
        let resolved: CommentView = call(&Request::ResolveComment {
            thread: thread.into(),
            comment: comment.into(),
            cwd: current_dir()?,
        })?;
        println!("resolved {}", resolved.id);
        return Ok(());
    }
    let comments: Vec<CommentView> = call(&Request::Comments {
        thread: thread.into(),
        cwd: current_dir()?,
    })?;
    print!("{}", render::comments(&comments));
    Ok(())
}

/// Milliseconds since the epoch for `YYYY-MM-DD` (UTC midnight) or `<n>d`
/// (n days ago).
pub fn parse_date(text: &str) -> Result<i64, Exn<CliError>> {
    const DAY_MS: i64 = 86_400_000;
    if let Some(days) = text.strip_suffix('d') {
        let days: i64 = days.parse().map_err(|_| fail(format!("invalid number of days in {text:?}")))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| i64::try_from(duration.as_millis()).unwrap_or_default());
        return Ok(now - days * DAY_MS);
    }
    let parts: Vec<i64> = text.split('-').filter_map(|part| part.parse().ok()).collect();
    let [year, month, day] = parts[..] else {
        return Err(fail(format!("expected YYYY-MM-DD or a number of days like 7d, not {text:?}")));
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(fail(format!("invalid date {text:?}")));
    }
    Ok(days_from_civil(year, month, day) * DAY_MS)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
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

pub fn server_pair(data: Option<PathBuf>, user: &str) -> Result<(), Exn<CliError>> {
    let data = server_data(data);
    let runtime = tokio::runtime::Runtime::new().or_raise(|| CliError("cannot start the async runtime".into()))?;
    let code = runtime
        .block_on(silt_server::create_pairing_code(&data, user))
        .or_raise(|| CliError("cannot create a pairing code".into()))?;
    println!("{code}");
    eprintln!("valid for 15 minutes; on the device run: silt auth login --server <url> --code {code}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_parse_as_utc_midnight() {
        assert_eq!(parse_date("1970-01-02").ok(), Some(86_400_000));
        assert_eq!(parse_date("2026-10-20").ok(), Some(1_792_454_400_000));
        assert!(parse_date("2026-13-01").is_err());
        assert!(parse_date("yesterday").is_err());
        assert!(parse_date("7d").is_ok_and(|ms| ms > 1_700_000_000_000));
    }
}
