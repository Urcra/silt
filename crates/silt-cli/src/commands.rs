use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use exn::{Exn, ResultExt};
use serde::de::DeserializeOwned;
use silt_agents::conventions as agent_conventions;
use silt_agents::{Agent, LaunchPlan, McpServer, SessionIds, Start};
use silt_git::conventions::{self as project, LinkOutcome, PrepareOutcome};
use silt_daemon::client;
use silt_daemon::paths;
use silt_daemon::protocol::{
    AgentStart, CommentOn, CommentView, DaemonStatus, HistoryPage, HookReply, InFlight, Mount, MrDetails, MrSummary, Pulled, Request, Response, Resumed, SearchFilters,
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
pub(crate) fn call<T: DeserializeOwned>(request: &Request) -> Result<T, Exn<CliError>> {
    let response = client::send(&paths::socket_path(), request, REQUEST_TIMEOUT)
        .or_raise(|| CliError("the Silt daemon is not answering; try `silt daemon start`".into()))?;
    match response {
        Response::Ok { data } => {
            serde_json::from_value(data).or_raise(|| CliError("unexpected answer from the daemon".into()))
        }
        Response::Error { message } => Err(fail(message)),
    }
}

pub(crate) fn is_running() -> bool {
    client::send(&paths::socket_path(), &Request::Ping, Duration::from_secs(2)).is_ok()
}

/// Starts the daemon in the background unless it already runs.
pub fn ensure_daemon() -> Result<bool, Exn<CliError>> {
    if is_running() {
        return Ok(false);
    }
    if silt_daemon::holds_lock(&paths::data_dir()) {
        return wait_for_busy_daemon().map(|()| false);
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

/// How long to wait for a daemon that runs but doesn't answer yet, say
/// while it records a long transcript.
const BUSY_DAEMON_WAIT: Duration = Duration::from_secs(60);

fn wait_for_busy_daemon() -> Result<(), Exn<CliError>> {
    eprintln!("the Silt daemon is busy; waiting for it");
    let deadline = Instant::now() + BUSY_DAEMON_WAIT;
    while Instant::now() < deadline {
        if is_running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(fail(format!(
        "the Silt daemon runs but does not answer; see {}",
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
    if !is_running() && silt_daemon::holds_lock(&paths::data_dir()) {
        wait_for_busy_daemon()?;
    }
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
    } else if silt_daemon::holds_lock(&paths::data_dir()) {
        println!("running, but busy");
    } else {
        println!("not running");
    }
    Ok(())
}

fn find_agent(name: &str) -> Result<&'static dyn Agent, Exn<CliError>> {
    silt_agents::agent(name).ok_or_else(|| {
        let known: Vec<&str> = silt_agents::AGENTS.iter().map(|agent| agent.command()).collect();
        fail(format!("unknown agent {name}; Silt knows {}", known.join(", ")))
    })
}

/// Runs an agent CLI in a new thread, with Silt recording its session and
/// every file edit.
pub fn run_agent(name: &str, isolated: bool, args: &[String]) -> Result<(), Exn<CliError>> {
    let agent = find_agent(name)?;
    ensure_daemon()?;
    let cwd = current_dir()?;
    let session_id = (agent.session_ids() == SessionIds::Chosen).then(|| uuid::Uuid::new_v4().to_string());
    let started: SessionStarted = call(&Request::StartSession {
        cwd,
        agent: agent.id().into(),
        session_id: session_id.clone(),
        isolated,
        existing: false,
    })?;
    eprintln!("silt: recording thread {} in {}", short(&started.thread), started.checkout.display());
    if let Some(mount) = &started.mount {
        mount_checkout(mount, &started.checkout, &started.thread);
    }
    let run = Run {
        agent,
        thread: &started.thread,
        checkout: &started.checkout,
        cwd: &started.agent_cwd,
    };
    let status = run.launch(Start::New { id: session_id }, args)?;
    call::<String>(&Request::EndSession {
        thread: started.thread.clone(),
    })?;
    eprintln!("silt: thread {} recorded; see `silt show {}`", short(&started.thread), short(&started.thread));
    exit_like(status)
}

fn exit_like(status: std::process::ExitStatus) -> Result<(), Exn<CliError>> {
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// One run of an agent in a thread's checkout.
struct Run<'a> {
    agent: &'static dyn Agent,
    thread: &'a str,
    checkout: &'a Path,
    cwd: &'a Path,
}

impl Run<'_> {
    /// Runs the agent with Silt's MCP server, hooks, and the repository's
    /// rules and skills it doesn't read by itself.
    fn launch(&self, start: Start, args: &[String]) -> Result<std::process::ExitStatus, Exn<CliError>> {
        let exe = env::current_exe().or_raise(|| CliError("cannot find the silt executable".into()))?;
        let scratch = paths::data_dir()
            .join("runs")
            .join(format!("{}-{}", short(self.thread), std::process::id()));
        fs::create_dir_all(&scratch).or_raise(|| CliError("cannot create a directory for the run".into()))?;
        let home = env::var_os("HOME").map(PathBuf::from);
        let native = self.agent.native();
        let skills = agent_conventions::discover_skills(self.checkout, home.as_deref());
        let rules = agent_conventions::discover_rules(self.checkout, self.cwd, home.as_deref());
        let plan = LaunchPlan {
            start,
            cwd: self.cwd.to_owned(),
            mcp: McpServer {
                command: exe.clone(),
                args: vec![
                    "mcp".into(),
                    "--thread".into(),
                    self.thread.to_owned(),
                    "--checkout".into(),
                    self.checkout.to_string_lossy().into_owned(),
                ],
            },
            hook_command: format!("{} hook", shell_quote(&exe)),
            instructions: agent_conventions::session_instructions(&rules, &skills, native, self.checkout),
            skills: agent_conventions::skills_to_give(&skills, native, self.checkout)
                .into_iter()
                .map(|skill| skill.dir.clone())
                .collect(),
            scratch: scratch.clone(),
            args: args.to_vec(),
        };
        let launch = self
            .agent
            .launch(&plan)
            .or_raise(|| CliError(format!("cannot prepare {}", self.agent.display_name())))?;
        let status = Command::new(&launch.program)
            .args(&launch.args)
            .envs(launch.envs.iter().map(|(key, value)| (key, value)))
            .current_dir(self.cwd)
            .status()
            .or_raise(|| CliError(format!("cannot run {}", launch.program.to_string_lossy())));
        let _ = fs::remove_dir_all(&scratch);
        status
    }
}

pub fn resume(thread: &str, here: bool, agent: Option<&str>, args: &[String]) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let resumed: Resumed = call(&Request::Resume {
        thread: thread.into(),
        cwd: current_dir()?,
        here,
        agent: agent.map(|name| find_agent(name).map(|agent| agent.id().to_owned())).transpose()?,
    })?;
    eprintln!("silt: thread {} is now here, in {}", short(&resumed.thread), resumed.checkout.display());
    continue_session(resumed, args)
}

pub fn fork(thread: &str, private: bool, here: bool, agent: Option<&str>, args: &[String]) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let forked: Resumed = call(&Request::Fork {
        thread: thread.into(),
        cwd: current_dir()?,
        here,
        agent: agent.map(|name| find_agent(name).map(|agent| agent.id().to_owned())).transpose()?,
        private,
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

/// Links the project's shared paths into a newly mounted managed checkout
/// and runs its `prepare` script there. Problems are reported, not fatal:
/// the agent can still work, and the user sees what went wrong.
/// Whether the checkout's prepare script may run (ADR 0016). It came with
/// the thread, so possibly from someone else's work or their agent's: it
/// runs unasked only when it is the one in this person's own clone, or one
/// they allowed before; otherwise they see it and decide.
fn prepare_trusted(script: &Path, checkout: &Path, source: &Path) -> bool {
    use std::io::{BufRead, IsTerminal, Write};
    let Ok(bytes) = fs::read(script) else {
        return false;
    };
    let relative = script.strip_prefix(checkout).unwrap_or(script);
    if fs::read(source.join(relative)).is_ok_and(|own| own == bytes) {
        return true;
    }
    let digest = silt_redact::id(&String::from_utf8_lossy(&bytes));
    let trusted = paths::data_dir().join("trusted-prepare");
    if fs::read_to_string(&trusted).is_ok_and(|known| known.lines().any(|line| line.trim() == digest)) {
        return true;
    }
    if !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
        eprintln!(
            "silt: not running {}: it differs from the one in your clone; run this again in a terminal to review it",
            relative.display()
        );
        return false;
    }
    eprintln!(
        "silt: this thread's {} differs from the one in your clone. It runs on this machine, outside any sandbox:",
        relative.display()
    );
    for line in String::from_utf8_lossy(&bytes).lines().take(40) {
        eprintln!("  | {line}");
    }
    eprint!("silt: run it? [y/N] ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    if io::stdin().lock().read_line(&mut answer).is_err() || !answer.trim().eq_ignore_ascii_case("y") {
        return false;
    }
    let remembered = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&trusted)
        .and_then(|mut file| writeln!(file, "{digest}"));
    if let Err(error) = remembered {
        eprintln!("silt: could not remember that: {error}");
    }
    true
}

fn mount_checkout(mount: &Mount, checkout: &Path, thread: &str) {
    match silt_git::Repo::discover(checkout)
        .and_then(|repo| project::link_shared_paths(&repo, &mount.source, &mount.shared))
    {
        Ok(outcomes) => {
            for outcome in outcomes {
                match outcome {
                    LinkOutcome::Linked { path, seeded: true } => {
                        eprintln!("silt: linked {path}, shared by this project's checkouts (copied from your clone)");
                    }
                    LinkOutcome::Linked { path, seeded: false } => eprintln!("silt: linked {path}"),
                    LinkOutcome::AlreadyLinked { .. } => {}
                    LinkOutcome::NotIgnored { path } => {
                        eprintln!("silt: not linking {path}: only git-ignored paths can be linked");
                    }
                    LinkOutcome::Occupied { path } => {
                        eprintln!("silt: not linking {path}: the checkout already has something there");
                    }
                }
            }
        }
        Err(error) => eprintln!("silt: could not link shared paths: {error}"),
    }
    if let Some(script) = project::project_file(checkout, "prepare")
        && !prepare_trusted(&script, checkout, &mount.source)
    {
        return;
    }
    let source = mount.source.to_string_lossy();
    let checkout_dir = checkout.to_string_lossy();
    let envs = [("SILT_THREAD", thread), ("SILT_CHECKOUT", &*checkout_dir), ("SILT_SOURCE", &*source)];
    match project::run_prepare(checkout, &envs) {
        Ok(None) => {}
        Ok(Some(PrepareOutcome::Ran { script, status })) if !status.success() => {
            eprintln!("silt: {} failed ({status}); continuing", script.display());
        }
        Ok(Some(PrepareOutcome::Ran { .. })) => {}
        Ok(Some(PrepareOutcome::NotExecutable { script })) => {
            eprintln!("silt: not running {}: it is not executable", script.display());
        }
        Err(error) => eprintln!("silt: could not run prepare: {error}"),
    }
}

/// Runs the agent on a resumed or forked session, if the thread has one.
fn continue_session(resumed: Resumed, args: &[String]) -> Result<(), Exn<CliError>> {
    if let Some(mount) = &resumed.mount {
        mount_checkout(mount, &resumed.checkout, &resumed.thread);
    }
    let Some(start) = resumed.start else {
        eprintln!("silt: the thread has no agent session to continue");
        return Ok(());
    };
    let agent = find_agent(&resumed.agent)?;
    let start = match start {
        AgentStart::Resume { id } => Start::Resume { id },
        AgentStart::Primed { id, prompt } => {
            eprintln!("silt: starting {} with the conversation so far", agent.display_name());
            Start::Primed { id, prompt }
        }
    };
    let run = Run {
        agent,
        thread: &resumed.thread,
        checkout: &resumed.checkout,
        cwd: &resumed.agent_cwd,
    };
    let status = run.launch(start, args)?;
    call::<String>(&Request::EndSession {
        thread: resumed.thread.clone(),
    })?;
    exit_like(status)
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
            for note in &status.recovered {
                println!("recovered: {note}");
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

pub fn track(agent: &str) -> Result<(), Exn<CliError>> {
    let agent = find_agent(agent)?;
    ensure_daemon()?;
    let cwd = current_dir()?;
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    let found = agent
        .discover(&cwd, 0, &[])
        .ok_or_else(|| fail(format!("no {} session found for {}", agent.display_name(), cwd.display())))?;
    let started: SessionStarted = call(&Request::StartSession {
        cwd,
        agent: agent.id().into(),
        session_id: Some(found.id),
        isolated: false,
        existing: true,
    })?;
    println!("tracking session in thread {}", short(&started.thread));
    Ok(())
}

/// Forwards an agent hook to the daemon, and prints any context the daemon
/// has for the agent in Claude Code's hook output format. Never fails, so a
/// missing daemon can't break the agent session.
pub fn hook(agent: Option<&str>, event: &str) {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(mut input) = serde_json::from_str::<serde_json::Value>(&input) else {
        return;
    };
    let copilot = agent == Some("copilot");
    if copilot {
        // Copilot names its fields in camelCase.
        let session = input.get("sessionId").cloned().unwrap_or_default();
        input = serde_json::json!({ "session_id": session, "hook_event_name": event });
    }
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
        let output = if copilot {
            serde_json::json!({ "additionalContext": context })
        } else {
            serde_json::json!({
                "hookSpecificOutput": { "hookEventName": event, "additionalContext": context }
            })
        };
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

pub fn thread_change(request: Request) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let message: String = call(&request)?;
    println!("{message}");
    Ok(())
}

pub fn trash() -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let threads: Vec<ThreadSummary> = call(&Request::Trash)?;
    if threads.is_empty() {
        println!("the trash is empty");
    } else {
        print!("{}", render::threads(&threads));
    }
    Ok(())
}

pub fn inbox(all: bool) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let entries: Vec<silt_daemon::protocol::InboxEntry> = call(&Request::Inbox { all })?;
    print!("{}", render::inbox(&entries));
    Ok(())
}

pub fn inbox_read(ids: Vec<i64>, all: bool) -> Result<(), Exn<CliError>> {
    if ids.is_empty() && !all {
        return Err(fail("name the notifications to mark read, or pass --all"));
    }
    ensure_daemon()?;
    call::<String>(&Request::InboxRead {
        read: silt_proto::InboxRead { ids, all },
    })?;
    println!("marked read");
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

pub fn diff(thread: &str, turn: Option<usize>, base: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let diff: String = call(&Request::Diff {
        thread: thread.into(),
        turn,
        base,
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

pub fn open(no_browser: bool) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let url: String = call(&Request::OpenWeb {
        cwd: env::current_dir().ok(),
    })?;
    println!("{url}");
    if !no_browser {
        // `wslview` opens the Windows browser from WSL; the others are the
        // usual desktop openers.
        let opened = ["xdg-open", "wslview", "open"].iter().any(|opener| {
            Command::new(opener)
                .arg(&url)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        });
        if !opened {
            eprintln!("silt: could not open a browser; open the address above");
        }
    }
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




/// Runs `command` in the sandbox in place of this process.
/// Runs `command` in the sandbox, exiting with its status. Silt's own data
/// is hidden too: its socket controls the daemon.
pub fn sandbox(mut sandbox: silt_sandbox::Sandbox, command: &[String]) -> Result<(), Exn<CliError>> {
    if command.is_empty() {
        return Err(fail("give a command to run"));
    }
    sandbox.hidden.push(paths::data_dir());
    sandbox.hidden.push(paths::socket_path());
    let exe = env::current_exe().or_raise(|| CliError("cannot find the silt binary".into()))?;
    let init = [exe.into_os_string(), "sandbox-init".into()];
    let code = sandbox.run(&init, command).or_raise(|| CliError("cannot set up the sandbox".into()))?;
    std::process::exit(code)
}

/// Serves the harness to the ACP client on stdin and stdout.
pub fn harness() -> Result<(), Exn<CliError>> {
    let config = silt_harness::Config {
        silt: env::current_exe().or_raise(|| CliError("cannot find the silt binary".into()))?,
        state_dir: paths::data_dir().join("harness"),
        unsandboxed: env::var_os("SILT_HARNESS_UNSANDBOXED").is_some_and(|value| value == "1"),
        secrets_file: Some(paths::data_dir().join("secrets")),
    };
    let runtime = tokio::runtime::Runtime::new().or_raise(|| CliError("cannot start the runtime".into()))?;
    runtime
        .block_on(silt_harness::serve(config))
        .or_raise(|| CliError("the harness stopped".into()))
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

