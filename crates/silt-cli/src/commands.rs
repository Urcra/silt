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
use silt_daemon::client;
use silt_daemon::paths;
use silt_daemon::protocol::{
    AgentStart, BlameLine, CommentOn, CommentView, DaemonBuild, DaemonStatus, HistoryPage, HookReply, ImportCandidate,
    ImportedThread, InFlight, Mount, MrDetails, MrSummary, Pulled, Request, Response, Resumed, Reverted, SearchFilters,
    SearchResult, Sent, SessionStarted, ShowLine, ThreadSummary,
};
use silt_git::conventions::{self as project, LinkOutcome, PrepareOutcome};
use silt_types::CloudSessionId;

use crate::CliError;
use crate::recording::{Recording, push_in_background};
use crate::render::{self, short};

/// How long a hook waits for the daemon before it captures in its own
/// process instead (ADR 0018).
const HOOK_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn fail(message: impl Into<String>) -> Exn<CliError> {
    Exn::new(CliError(message.into()))
}

/// Sends a request to the running daemon and decodes its answer.
pub(crate) fn call<T: DeserializeOwned>(request: &Request) -> Result<T, Exn<CliError>> {
    let response = client::send(&paths::socket_path(), request, None)
        .or_raise(|| CliError("could not get an answer from the Silt daemon; check `silt daemon status`".into()))?;
    match response {
        Response::Ok { data } => serde_json::from_value(data).or_raise(|| CliError("unexpected answer from the daemon".into())),
        Response::Error { message } => Err(fail(message)),
    }
}

pub(crate) fn is_running() -> bool {
    client::send(&paths::socket_path(), &Request::Ping, Some(Duration::from_secs(2))).is_ok()
}

/// Starts the daemon in the background unless it already runs.
pub fn ensure_daemon() -> Result<bool, Exn<CliError>> {
    if is_running() {
        warn_of_other_agent_homes();
        if !stop_replaced_daemon()? {
            return Ok(false);
        }
    } else if silt_daemon::holds_lock(&paths::data_dir()) && wait_for_busy_daemon()? == Waited::Answered {
        return Ok(false);
    }
    spawn_daemon()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if is_running() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(fail(format!("the Silt daemon did not start; see {}", paths::log_path().display())))
}

/// Starts the daemon in the background, without waiting for it.
fn spawn_daemon() -> Result<(), Exn<CliError>> {
    fs::create_dir_all(paths::data_dir()).or_raise(|| CliError("cannot create the Silt data directory".into()))?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::log_path())
        .or_raise(|| CliError("cannot open the daemon log".into()))?;
    let log_err = log.try_clone().or_raise(|| CliError("cannot open the daemon log".into()))?;
    let exe = env::current_exe().or_raise(|| CliError("cannot find the silt executable".into()))?;
    // The daemon outlives this command: it starts from `/`, not from
    // wherever this ran. On Windows it gets no handles, and writes its log
    // itself.
    silt_os::spawn_detached(
        Command::new(exe)
            .args(["daemon", "run"])
            .env(DAEMON_LOG, paths::log_path())
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(log_err),
    )
    .or_raise(|| CliError("cannot start the Silt daemon".into()))?;
    Ok(())
}

/// The daemon finds agents' sessions where its own environment says, which
/// is the environment of the command that started it. Says so when this
/// command's agent homes differ, as its sessions would then go unseen.
fn warn_of_other_agent_homes() {
    let Ok(homes) = call::<Vec<(String, Option<String>)>>(&Request::AgentHomes) else {
        return;
    };
    for (name, daemons) in homes {
        let mine = env::var(&name).ok();
        if mine != daemons {
            eprintln!(
                "silt: the daemon runs with {name}={}, this command with {name}={}; restart it with `silt daemon stop` to use this one",
                daemons.as_deref().unwrap_or("(unset)"),
                mine.as_deref().unwrap_or("(unset)")
            );
        }
    }
}

/// Stops the daemon if its program was replaced since it started, as
/// installing Silt again does, so this command starts the new build. Only
/// when no agent works in it, and when it's this program: running another
/// build beside it never swaps it. Says whether it stopped it.
fn stop_replaced_daemon() -> Result<bool, Exn<CliError>> {
    let Ok(build) = call::<DaemonBuild>(&Request::DaemonBuild) else {
        return Ok(false);
    };
    if !build.replaced {
        return Ok(false);
    }
    let canonical = |path: PathBuf| fs::canonicalize(&path).unwrap_or(path);
    let mine = env::current_exe().ok().map(canonical);
    if build.working > 0 || build.exe.map(canonical) != mine {
        let why = match build.working {
            0 => String::new(),
            1 => ", as an agent works in it".to_owned(),
            working => format!(", as {working} agents work in it"),
        };
        eprintln!(
            "silt: the daemon still runs the build installed before this one{why}; `silt daemon stop` restarts it on this one"
        );
        return Ok(false);
    }
    stop_daemon()?;
    eprintln!("silt: restarted the daemon on the build installed since it started");
    Ok(true)
}

/// How long to wait for a daemon that runs but doesn't answer yet, say
/// while it records a long transcript.
const BUSY_DAEMON_WAIT: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Eq)]
enum Waited {
    /// It answers now.
    Answered,
    /// It let go of the data directory: it stopped, or was stopping.
    Gone,
}

fn wait_for_busy_daemon() -> Result<Waited, Exn<CliError>> {
    eprintln!("the Silt daemon is busy; waiting for it");
    let deadline = Instant::now() + BUSY_DAEMON_WAIT;
    while Instant::now() < deadline {
        if is_running() {
            return Ok(Waited::Answered);
        }
        if !silt_daemon::holds_lock(&paths::data_dir()) {
            return Ok(Waited::Gone);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(fail(format!("the Silt daemon runs but does not answer; see {}", paths::log_path().display())))
}

/// Set for a daemon started in the background: where it logs when it has
/// no stderr of its own.
const DAEMON_LOG: &str = "SILT_DAEMON_LOG";

pub fn daemon_run() -> Result<(), Exn<CliError>> {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("SILT_LOG").unwrap_or_else(|_| "info".into()));
    let log = env::var_os(DAEMON_LOG)
        .filter(|_| cfg!(windows))
        .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());
    match log {
        Some(log) => subscriber.with_ansi(false).with_writer(std::sync::Mutex::new(log)).init(),
        None => subscriber.with_writer(io::stderr).init(),
    }
    // Panics go to the log too, which on Windows is all the daemon has.
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("{info}");
        report(info);
    }));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(DAEMON_STACK)
        .build()
        .or_raise(|| CliError("cannot start the async runtime".into()))?;
    // Requests are handled on the thread that runs the event loop, and
    // Windows gives a main thread 1 MiB.
    std::thread::Builder::new()
        .name("silt-daemon".into())
        .stack_size(DAEMON_STACK)
        .spawn(move || runtime.block_on(silt_daemon::run(&paths::data_dir(), &paths::socket_path())))
        .or_raise(|| CliError("cannot start the Silt daemon's thread".into()))?
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .or_raise(|| CliError("the Silt daemon failed".into()))
}

const DAEMON_STACK: usize = 8 << 20;

pub fn daemon_stop() -> Result<(), Exn<CliError>> {
    if !is_running() && silt_daemon::holds_lock(&paths::data_dir()) {
        wait_for_busy_daemon()?;
    }
    if !is_running() {
        println!("not running");
        return Ok(());
    }
    stop_daemon()?;
    println!("stopped");
    Ok(())
}

/// Stops the running daemon, and waits until it let go of the data dir.
fn stop_daemon() -> Result<(), Exn<CliError>> {
    call::<String>(&Request::Shutdown)?;
    // The daemon answers before it closes the store, and lets go of its lock
    // only after: until then the data dir isn't safe to copy.
    let deadline = Instant::now() + BUSY_DAEMON_WAIT;
    while silt_daemon::holds_lock(&paths::data_dir()) {
        if Instant::now() >= deadline {
            return Err(fail(format!("the Silt daemon does not exit; see {}", paths::log_path().display())));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
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

pub fn daemon_compact() -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let compacted: silt_daemon::protocol::Compaction = call(&Request::Compact)?;
    println!(
        "compacted the store: {} MB, from {} MB; {} chunks compressed",
        compacted.after / 1_000_000,
        compacted.before / 1_000_000,
        compacted.recompressed
    );
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
    let cwd = current_dir()?;
    if silt_git::Repo::discover(&cwd).is_err() {
        eprintln!(
            "silt: {} isn't in a git repository, so nothing can continue this thread elsewhere; `silt chat` starts one in a folder of its own",
            cwd.display()
        );
    }
    run_agent_in(name, cwd, isolated, args)
}

/// Starts a chat (ADR 0059): `name`'s agent in a new folder of its own,
/// named after what it's `about`, recorded as `silt <agent>` records.
pub fn chat(name: &str, about: Option<&str>, workspace: Option<&str>, args: &[String]) -> Result<(), Exn<CliError>> {
    find_agent(name)?;
    let folder = silt_daemon::chats::new_folder_in(&paths::data_dir(), about.unwrap_or_default(), workspace)
        .or_raise(|| CliError("could not make the chat's folder".into()))?;
    eprintln!("silt: chatting in {}", folder.display());
    run_agent_in(name, folder, false, args)
}

fn run_agent_in(name: &str, cwd: PathBuf, isolated: bool, args: &[String]) -> Result<(), Exn<CliError>> {
    let agent = find_agent(name)?;
    // Before the session starts, so a refused run leaves no thread behind.
    let held = workspace_rules(None, Some(&cwd), agent, args)?;
    let session_id = (agent.session_ids() == SessionIds::Chosen).then(|| uuid::Uuid::new_v4().to_string());
    let (started, mut recording) = if daemonless_capture() {
        Recording::here(agent, &cwd, session_id.as_deref(), isolated, false)?
    } else if is_running() {
        warn_of_other_agent_homes();
        let started: SessionStarted = call(&Request::StartSession {
            cwd,
            agent: agent.id().into(),
            session_id: session_id.clone(),
            isolated,
            existing: false,
            cloud: None,
        })?;
        let recording = Recording::by_daemon(&started.thread)?;
        (started, recording)
    } else {
        // The agent starts without waiting for the daemon, which takes the
        // session over once it answers.
        let here = Recording::here(agent, &cwd, session_id.as_deref(), isolated, true)?;
        if !silt_daemon::holds_lock(&paths::data_dir()) {
            spawn_daemon()?;
        }
        here
    };
    eprintln!("silt: recording thread {} in {}", short(&started.thread), started.checkout.display());
    if let Some(mount) = &started.mount {
        mount_checkout(mount, &started.checkout, &started.thread);
    }
    let run = Run { agent, thread: &started.thread, checkout: &started.checkout, cwd: &started.agent_cwd, held: &held };
    let status = run.launch(Start::New { id: session_id }, args, &mut recording)?;
    recording.end()?;
    eprintln!("silt: thread {} recorded; see `silt show {}`", short(&started.thread), short(&started.thread));
    exit_like(status)
}

/// Whether to capture in this process even when a daemon runs.
fn daemonless_capture() -> bool {
    env::var_os("SILT_DAEMONLESS_CAPTURE").is_some_and(|value| value == "1")
}

/// Exits as the agent did: with its code, or 128 + the signal that ended it,
/// as a shell would report.
fn exit_like(status: std::process::ExitStatus) -> Result<(), Exn<CliError>> {
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        std::process::exit(128 + signal);
    }
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// A directory for one run, removed however the run ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// One run of an agent in a thread's checkout.
struct Run<'a> {
    agent: &'static dyn Agent,
    thread: &'a str,
    checkout: &'a Path,
    cwd: &'a Path,
    /// What its workspace's rules hold it to, checked before it started.
    held: &'a silt_daemon::workspaces::RunRules,
}

impl Run<'_> {
    /// Runs the agent with Silt's MCP server, hooks, and the repository's
    /// rules and skills it doesn't read by itself.
    fn launch(
        &self,
        start: Start,
        args: &[String],
        recording: &mut Recording,
    ) -> Result<std::process::ExitStatus, Exn<CliError>> {
        let held = self.held;
        let exe = env::current_exe().or_raise(|| CliError("cannot find the silt executable".into()))?;
        let scratch = paths::data_dir().join("runs").join(format!("{}-{}", short(self.thread), std::process::id()));
        fs::create_dir_all(&scratch).or_raise(|| CliError("cannot create a directory for the run".into()))?;
        let _scratch = Scratch(scratch.clone());
        let home = silt_os::home_dir();
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
                ]
                .into_iter()
                .chain((!held.token.is_empty()).then(|| ["--token".to_owned(), held.token.clone()]).into_iter().flatten())
                .collect(),
            },
            hook_command: format!("{} hook", silt_agents::shell_quote(&exe.to_string_lossy())),
            hook_program: exe.clone(),
            instructions: agent_conventions::session_instructions(&rules, &skills, native, self.checkout),
            skills: agent_conventions::skills_to_give(&skills, native, self.checkout)
                .into_iter()
                .map(|skill| skill.dir.clone())
                .collect(),
            scratch: scratch.clone(),
            args: args.to_vec(),
            locks: held.locks.clone(),
            env: held.env.clone(),
        };
        let launch = self.agent.launch(&plan).or_raise(|| CliError(format!("cannot prepare {}", self.agent.display_name())))?;
        let mut command = Command::new(&launch.program);
        command
            .args(&launch.args)
            .envs(held.env.iter().map(|(key, value)| (key, value)))
            .envs(launch.envs.iter().map(|(key, value)| (key, value)))
            .current_dir(self.cwd);
        if held.locks.no_bypass {
            command.env_remove("COPILOT_ALLOW_ALL");
        }
        recording.run(command).or_raise(|| CliError(format!("cannot run {}", launch.program.to_string_lossy())))
    }
}

/// The rules of the thread's workspace for this run (ADR 0065): refused
/// with the rule that forbids it, else the locks and variables to pass.
fn workspace_rules(
    thread: Option<&str>,
    cwd: Option<&Path>,
    agent: &'static dyn Agent,
    args: &[String],
) -> Result<silt_daemon::workspaces::RunRules, Exn<CliError>> {
    // A daemon running holds the store; opening it beside one slows its capture.
    let rules = if !daemonless_capture() && is_running() {
        call::<silt_daemon::workspaces::RunRules>(&Request::RunRules {
            thread: thread.map(str::to_owned),
            cwd: cwd.map(Path::to_owned),
            agent: agent.id().to_owned(),
            args: args.to_vec(),
        })?
    } else {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_raise(|| CliError("cannot start the async runtime".into()))?;
        runtime
            .block_on(silt_daemon::workspaces::run_rules(&paths::data_dir(), thread, cwd, agent, args))
            .map_err(|error| fail(error.to_string()))?
    };
    if rules.locks != silt_agents::Locks::default() || !rules.env.is_empty() {
        let mut held = Vec::new();
        if rules.locks.sandbox {
            held.push("sandboxed");
        }
        if !rules.locks.deny.is_empty() {
            held.push("history: this workspace only");
        }
        if rules.locks.no_bypass {
            held.push("asks before it acts");
        }
        eprintln!("silt: {} · {}", rules.workspace.name, held.join(" · "));
    }
    Ok(rules)
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

pub fn fork(
    thread: &str,
    private: bool,
    turn: Option<usize>,
    here: bool,
    agent: Option<&str>,
    args: &[String],
) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let forked: Resumed = call(&Request::Fork {
        thread: thread.into(),
        cwd: current_dir()?,
        here,
        agent: agent.map(|name| find_agent(name).map(|agent| agent.id().to_owned())).transpose()?,
        private,
        turn,
    })?;
    eprintln!("silt: forked {} as thread {}, in {}", short(thread), short(&forked.thread), forked.checkout.display());
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
    let remembered = OpenOptions::new().create(true).append(true).open(&trusted).and_then(|mut file| writeln!(file, "{digest}"));
    if let Err(error) = remembered {
        eprintln!("silt: could not remember that: {error}");
    }
    true
}

fn mount_checkout(mount: &Mount, checkout: &Path, thread: &str) {
    match silt_git::Repo::discover(checkout).and_then(|repo| project::link_shared_paths(&repo, &mount.source, &mount.shared)) {
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
    let held = workspace_rules(Some(&resumed.thread), None, agent, args)?;
    let run = Run { agent, thread: &resumed.thread, checkout: &resumed.checkout, cwd: &resumed.agent_cwd, held: &held };
    let mut recording = Recording::by_daemon(&resumed.thread)?;
    let status = run.launch(start, args, &mut recording)?;
    recording.end()?;
    exit_like(status)
}

pub fn auth_status() -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let status: DaemonStatus = call(&Request::Status)?;
    match status.server {
        Some(_) => {
            for (index, server) in status.servers.iter().enumerate() {
                let default = if index == 0 { " (the default for new projects)" } else { "" };
                println!("syncing with {} as {}, device {} ({}){default}", server.url, server.user, server.device, server.name);
            }
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

pub fn track(agent: &str, session: Option<&str>) -> Result<(), Exn<CliError>> {
    let agent = find_agent(agent)?;
    let cwd = current_dir()?;
    let cwd = silt_os::canonicalize(&cwd).unwrap_or(cwd);
    let id = match session {
        Some(id) => id.to_owned(),
        None => {
            agent
                .discover(&cwd, 0, &[])
                .ok_or_else(|| fail(format!("no {} session found for {}", agent.display_name(), cwd.display())))?
                .id
        }
    };
    let thread = start_tracking(agent, cwd, id, None)?;
    println!("tracking session in thread {}", short(&thread));
    Ok(())
}

/// Imports the sessions on disk Silt hasn't recorded, one request each so
/// the daemon answers others between them (ADR 0043).
pub fn import(paths: &[PathBuf], agent: Option<&str>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let cwd = current_dir()?;
    let paths: Vec<PathBuf> = paths.iter().map(|path| cwd.join(path)).collect();
    let candidates: Vec<ImportCandidate> = call(&Request::ImportCandidates { paths, agent: agent.map(str::to_owned) })?;
    if candidates.is_empty() {
        println!("no sessions to import: every session on disk is recorded or imported already");
        return Ok(());
    }
    let (mut imported, mut failed) = (0, 0);
    for candidate in candidates {
        let (agent, id) = (candidate.agent.clone(), candidate.id.clone());
        match call::<Option<ImportedThread>>(&Request::Import { candidate }) {
            Ok(Some(thread)) => {
                imported += 1;
                let place = match (&thread.project, &thread.cwd) {
                    (project, _) if !project.is_empty() => project.clone(),
                    (_, Some(cwd)) => format!("{} (no project)", cwd.display()),
                    _ => "(no project)".to_owned(),
                };
                println!(
                    "{}  {agent} session from {}  {}",
                    short(&thread.thread),
                    silt_agents::time::format_timestamp(thread.started_ms).get(..10).unwrap_or_default(),
                    render::safe(&place)
                );
            }
            Ok(None) => {}
            Err(error) => {
                failed += 1;
                eprintln!(
                    "silt: could not import {agent} session {}: {}",
                    render::safe(&id),
                    render::safe(&crate::explain(&error))
                );
            }
        }
    }
    println!("imported {imported} sessions into threads kept on this machine; `silt send --all` sends them to your server");
    if failed > 0 {
        return Err(fail(format!("{failed} sessions could not be imported")));
    }
    Ok(())
}

pub fn send(threads: Vec<String>, project: Option<String>, server: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let sent: Sent = call(&Request::Send { threads, project, server })?;
    println!("sending {} imported threads to {}", sent.threads, sent.server);
    if sent.redacted > 0 {
        println!("{} secrets in them are redacted before they leave this machine", sent.redacted);
    }
    Ok(())
}

fn start_tracking(agent: &dyn Agent, cwd: PathBuf, id: String, cloud: Option<CloudSessionId>) -> Result<String, Exn<CliError>> {
    ensure_daemon()?;
    let request =
        Request::StartSession { cwd, agent: agent.id().into(), session_id: Some(id), isolated: false, existing: true, cloud };
    let started: SessionStarted = call(&request)?;
    Ok(started.thread)
}

/// Starts recording the session whose `SessionStart` hook input is on
/// stdin, then captures that hook. Its stdout is the agent's context, so it
/// prints only what the hook replies.
pub fn track_from_hook(agent: &str) -> Result<(), Exn<CliError>> {
    let agent = find_agent(agent)?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).or_raise(|| CliError("could not read the hook input".into()))?;
    let input: serde_json::Value =
        serde_json::from_str(&input).or_raise(|| CliError("the hook input on stdin isn't JSON".into()))?;
    let (Some(id), Some(cwd)) = (input["session_id"].as_str(), input["cwd"].as_str()) else {
        return Err(fail("the hook input names no session_id and cwd"));
    };
    let cwd = PathBuf::from(cwd);
    let cwd = silt_os::canonicalize(&cwd).unwrap_or(cwd);
    // Claude Code sets it in a cloud session's container (ADR 0040).
    let cloud = std::env::var("CLAUDE_CODE_REMOTE_SESSION_ID").ok().and_then(|id| id.parse().ok());
    let thread = start_tracking(agent, cwd, id.to_owned(), cloud)?;
    eprintln!("silt: recording this session in thread {}", short(&thread));
    let event = input["hook_event_name"].as_str().unwrap_or("SessionStart").to_owned();
    hook_input(Some(agent.id()), &event, input);
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
    let Ok(input) = serde_json::from_str::<serde_json::Value>(&input) else {
        return;
    };
    hook_input(agent, event, input);
}

fn hook_input(agent: Option<&str>, event: &str, mut input: serde_json::Value) {
    let copilot = agent == Some("copilot");
    if copilot {
        // Copilot names its fields in camelCase.
        let session = input.get("sessionId").cloned().unwrap_or_default();
        input = serde_json::json!({ "session_id": session, "hook_event_name": event });
    }
    let reply = match forwarded(event, &input) {
        Some(reply) => reply,
        None => captured_here(event, input),
    };
    if let HookReply { additional_context: Some(context) } = reply {
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

/// The running daemon's answer to the hook, unless no daemon runs, it
/// didn't answer in time, or capture in process was asked for.
fn forwarded(event: &str, input: &serde_json::Value) -> Option<HookReply> {
    if daemonless_capture() || !silt_daemon::holds_lock(&paths::data_dir()) {
        return None;
    }
    let request = Request::Hook { event: event.to_owned(), input: input.clone() };
    match client::send(&paths::socket_path(), &request, Some(HOOK_TIMEOUT)) {
        Ok(Response::Ok { data }) => Some(serde_json::from_value(data).unwrap_or_default()),
        // It answered, so it captured what it could; capturing here too
        // would find the same.
        Ok(_) => Some(HookReply::default()),
        Err(_) => None,
    }
}

/// Captures the hook in this process, and once the agent's turn stops,
/// pushes what was recorded. A hook never fails the agent: what it couldn't
/// record is told on stderr.
fn captured_here(event: &str, input: serde_json::Value) -> HookReply {
    let captured = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(silt_daemon::CoreError::from)
        .and_then(|runtime| runtime.block_on(silt_daemon::capture_here(&paths::data_dir(), event, input)));
    match captured {
        Ok(captured) => {
            if captured.deferred {
                eprintln!("silt: another process kept Silt's data busy; the {event} hook is recorded once it lets go");
            }
            if let (Some(thread), "Stop") = (&captured.thread, event) {
                push_in_background(thread);
            }
            if captured.thread.is_some() {
                start_daemon_for_later_hooks();
            }
            captured.reply
        }
        Err(error) => {
            eprintln!("silt: could not record the {event} hook: {}", silt_daemon::protocol::error_chain(&error));
            HookReply::default()
        }
    }
}

/// Starts the daemon after a hook of a recorded session captured without
/// one, so the session's later hooks are forwarded rather than each loading
/// its family (ADR 0036). Capture in process asked for starts none.
fn start_daemon_for_later_hooks() {
    if daemonless_capture() || silt_daemon::holds_lock(&paths::data_dir()) {
        return;
    }
    if let Err(error) = spawn_daemon() {
        eprintln!("silt: could not start the daemon for this session's next hooks: {error}");
    }
}

pub fn sync_once(thread: Option<&str>) -> Result<(), Exn<CliError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_raise(|| CliError("cannot start the async runtime".into()))?;
    let synced = runtime
        .block_on(silt_daemon::sync_once(&paths::data_dir(), thread))
        .or_raise(|| CliError("could not sync with the server".into()))?;
    println!("pushed {} deltas of {} thread families", synced.deltas, synced.families);
    Ok(())
}

pub fn project_home(repo: String, home: String) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let said: String = call(&Request::ProjectHome { repo, home })?;
    println!("{said}");
    Ok(())
}

pub fn revert(thread: Option<String>, turn: usize) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let reverted: Reverted = call(&Request::Revert { thread, cwd: current_dir()?, turn })?;
    println!(
        "reverted thread {} to turn {}: {} files written back; the later turns are kept, marked rewound",
        short(&reverted.thread),
        reverted.turn,
        reverted.files.len()
    );
    for file in &reverted.files {
        println!("  {file}");
    }
    Ok(())
}

pub fn pull(thread: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let pulled: Pulled = call(&Request::Pull { thread, cwd: current_dir()? })?;
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
    let mr: MrSummary = call(&Request::MrCreate { thread, cwd: current_dir()?, title, summary })?;
    println!("opened merge request {}: {} → {}", mr.id, short(&mr.fork), short(&mr.target));
    Ok(())
}

pub fn mr_list(thread: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let requests: Vec<MrSummary> = call(&Request::MrList { thread })?;
    print!("{}", render::safe(&render::merge_requests(&requests)));
    Ok(())
}

pub fn mr_show(mr: &str, hide_git: bool) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let details: MrDetails = call(&Request::MrShow { mr: mr.into(), hide_git })?;
    print!("{}", render::safe(&render::merge_request(&details)));
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

pub fn mr_request_changes(mr: &str, reason: &str) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let asked: MrSummary = call(&Request::MrRequestChanges { mr: mr.into(), reason: reason.into() })?;
    println!("asked fork {} for changes to {}; it stays open", short(&asked.fork), asked.id);
    Ok(())
}

pub fn mr_guide(mr: &str, text: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let answer: String = call(&Request::MrGuide { mr: mr.into(), text })?;
    match answer.as_str() {
        "asked" => println!("asked the fork's agent to write the guide; it shows with `silt mr show {mr}` once its turn ends"),
        _ => println!("kept the change guide of {mr}"),
    }
    Ok(())
}

pub fn visibility(visibility: silt_proto::Visibility, thread: Option<String>) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let message: String = call(&Request::SetVisibility { cwd: current_dir()?, thread, visibility })?;
    println!("{message}");
    Ok(())
}

pub fn thread_change(request: Request) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let message: String = call(&request)?;
    println!("{message}");
    Ok(())
}

pub fn thread_link(thread: String, days: u32, frozen: bool) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let link: silt_daemon::protocol::ThreadLink =
        call(&Request::ThreadLink { thread, link: silt_daemon::protocol::NewLink { days, frozen, artifact: None } })?;
    println!("{}", link.url.unwrap_or_default());
    eprintln!("anyone with this link reads the thread for {days} days, until revoked from its Share dialog");
    Ok(())
}

pub fn trash() -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let threads: Vec<ThreadSummary> = call(&Request::Trash)?;
    if threads.is_empty() {
        println!("the trash is empty");
    } else {
        print!("{}", render::safe(&render::threads(&threads)));
    }
    Ok(())
}

pub fn inbox(all: bool) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let entries: Vec<silt_daemon::protocol::InboxEntry> = call(&Request::Inbox { all })?;
    print!("{}", render::safe(&render::inbox(&entries)));
    Ok(())
}

pub fn inbox_read(ids: Vec<i64>, all: bool) -> Result<(), Exn<CliError>> {
    if ids.is_empty() && !all {
        return Err(fail("name the notifications to mark read, or pass --all"));
    }
    ensure_daemon()?;
    call::<String>(&Request::InboxRead { read: silt_proto::InboxRead { ids, all } })?;
    println!("marked read");
    Ok(())
}

pub fn log() -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let threads: Vec<ThreadSummary> = call(&Request::Threads)?;
    print!("{}", render::safe(&render::threads(&threads)));
    Ok(())
}

pub fn find_commit(commit: &str) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let found: Vec<(String, Option<usize>)> = call(&Request::FindCommit { commit: commit.into() })?;
    if found.is_empty() {
        println!("no thread here made {commit}");
    }
    for (thread, turn) in found {
        match turn {
            Some(turn) => println!("{thread} turn {turn}"),
            None => println!("{thread} before its first turn"),
        }
    }
    Ok(())
}

pub fn show(thread: &str) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let lines: Vec<ShowLine> = call(&Request::Show { thread: thread.into() })?;
    print!("{}", render::safe(&render::show(&lines)));
    Ok(())
}

pub fn diff(
    thread: &str,
    turn: Option<usize>,
    to_turn: Option<usize>,
    base: Option<String>,
    hide_git: bool,
) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let diff: String = call(&Request::Diff { thread: thread.into(), turn, to_turn, base, files: None, path: None, hide_git })?;
    print!("{}", render::safe(&diff));
    Ok(())
}

pub fn search(query: &str, filters: SearchFilters) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let hits: Vec<SearchResult> = call(&Request::Search { query: query.into(), limit: 20, filters })?;
    print!("{}", render::safe(&render::search(&hits)));
    Ok(())
}

pub fn in_flight(path: &str) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let found: Vec<InFlight> = call(&Request::InFlight { cwd: current_dir()?, path: path.into() })?;
    print!("{}", render::safe(&render::in_flight(&found)));
    Ok(())
}

/// From the daemon when one runs or starts, else from the notes alone.
pub fn blame(file: &str, notes_only: bool) -> Result<(), Exn<CliError>> {
    let cwd = current_dir()?;
    let lines: Vec<BlameLine> = if !notes_only && ensure_daemon().is_ok() {
        call(&Request::Blame { cwd, path: file.into() })?
    } else {
        let repo =
            silt_git::Repo::discover(&cwd).or_raise(|| CliError(format!("{} is not in a git repository", cwd.display())))?;
        let path = cwd.join(file);
        let relative = path.strip_prefix(repo.root()).unwrap_or(&path).to_string_lossy().into_owned();
        let text = std::fs::read(&path).or_raise(|| CliError(format!("cannot read {file}")))?;
        let disk: Vec<String> = String::from_utf8_lossy(&text).lines().map(str::to_owned).collect();
        silt_daemon::blame_from_notes(&repo, &relative, &disk)
    };
    print!("{}", render::safe(&render::blame(&lines)));
    Ok(())
}

pub fn open(no_browser: bool) -> Result<(), Exn<CliError>> {
    let cwd = env::current_dir().ok();
    // `--no-browser` is how the app itself asks, so it never opens the app.
    if cfg!(windows)
        && let Some(cwd) = &cwd
        && silt_os::wsl_path(&cwd.to_string_lossy()).is_some()
    {
        if !no_browser && crate::desktop::open_wsl_folder(cwd) {
            return Ok(());
        }
        return crate::desktop::refuse_wsl_folder(cwd).map_err(fail);
    }
    ensure_daemon()?;
    if !no_browser && cwd.as_deref().is_some_and(crate::desktop::open_from_wsl) {
        eprintln!("silt: opened in the Silt app on Windows");
        return Ok(());
    }
    if !no_browser && cwd.as_deref().is_some_and(crate::desktop::open_native) {
        eprintln!("silt: opened in the Silt app");
        return Ok(());
    }
    // A browser's arguments are visible to other users, so it gets an
    // address that works once rather than one with the lasting token.
    let url: String = call(&Request::OpenWeb { cwd, once: !no_browser })?;
    println!("{url}");
    if !no_browser && !open_in_browser(&url) {
        eprintln!("silt: could not open a browser; open the address above within a minute");
    }
    Ok(())
}

/// Opens `url` with the desktop's opener. `wslview` opens the Windows
/// browser from WSL. `open` is only tried on macOS: elsewhere it can be
/// another program entirely, such as Debian's `openvt`.
pub fn open_in_browser(url: &str) -> bool {
    // rundll32 hands the address to Windows' default browser without cmd,
    // which would split it at its `&`.
    let openers: &[&[&str]] = if cfg!(windows) {
        &[&["rundll32", "url.dll,FileProtocolHandler"]]
    } else if cfg!(target_os = "macos") {
        &[&["open"]]
    } else {
        &[&["xdg-open"], &["wslview"]]
    };
    openers.iter().any(|opener| {
        Command::new(opener[0])
            .args(&opener[1..])
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

pub fn history(thread: &str, offset: usize, limit: usize) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let page: HistoryPage = call(&Request::HistoryRead { thread: thread.into(), offset, limit })?;
    print!("{}", render::safe(&render::history(&page)));
    Ok(())
}

pub fn comment(
    thread: &str,
    file: Option<String>,
    lines: Option<(usize, usize)>,
    entry: Option<usize>,
    (base, hide_git): (Option<String>, bool),
    text: &str,
) -> Result<(), Exn<CliError>> {
    let on = match (file, lines, entry) {
        (Some(path), Some((start, end)), None) => CommentOn::Lines { path, start, end, base, hide_git },
        (None, None, Some(index)) => CommentOn::Entry { index },
        _ => return Err(fail("comment on either --file with --lines, or --entry")),
    };
    ensure_daemon()?;
    let comment: CommentView = call(&Request::Comment { thread: thread.into(), on, text: text.into(), cwd: current_dir()? })?;
    println!("added comment {} at {}", comment.id, render::comment_location(&comment.location));
    Ok(())
}

pub fn reply(thread: &str, comment: &str, text: &str) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let request = Request::Reply { thread: thread.into(), comment: comment.into(), text: text.into(), cwd: current_dir()? };
    let answered: CommentView = call(&request)?;
    println!("replied to {}", answered.id);
    Ok(())
}

pub fn comments(thread: &str, resolve: Option<&str>, base: Option<String>, hide_git: bool) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    if let Some(comment) = resolve {
        let resolved: CommentView =
            call(&Request::ResolveComment { thread: thread.into(), comment: comment.into(), cwd: current_dir()? })?;
        println!("resolved {}", resolved.id);
        return Ok(());
    }
    let comments: Vec<CommentView> = call(&Request::Comments { thread: thread.into(), base, hide_git, cwd: current_dir()? })?;
    print!("{}", render::safe(&render::comments(&comments)));
    Ok(())
}

/// Milliseconds since the epoch for `YYYY-MM-DD` (UTC midnight) or `<n>d`
/// (n days ago).
pub fn parse_date(text: &str) -> Result<i64, String> {
    const DAY_MS: i64 = 86_400_000;
    if let Some(days) = text.strip_suffix('d') {
        let days: u32 = days.parse().map_err(|_| format!("invalid number of days in {text:?}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| i64::try_from(duration.as_millis()).unwrap_or_default());
        return Ok(now - i64::from(days) * DAY_MS);
    }
    let parts: Option<Vec<i64>> =
        text.split('-').map(|part| part.bytes().all(|byte| byte.is_ascii_digit()).then(|| part.parse().ok()).flatten()).collect();
    let Some([year, month, day]) = parts.as_deref().and_then(|parts| <[i64; 3]>::try_from(parts).ok()) else {
        return Err(format!("expected YYYY-MM-DD or a number of days like 7d, not {text:?}"));
    };
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&day) {
        return Err(format!("{text:?} is not a date"));
    }
    Ok(days_from_civil(year, month, day) * DAY_MS)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A line, or a range such as `10-14`, numbered from 1.
pub fn parse_lines(text: &str) -> Result<(usize, usize), String> {
    let number = |part: &str| part.trim().parse::<usize>().ok().filter(|line| *line >= 1);
    let (start, end) = match text.split_once('-') {
        Some((start, end)) => (number(start), number(end)),
        None => (number(text), number(text)),
    };
    match (start, end) {
        (Some(start), Some(end)) if start <= end => Ok((start, end)),
        _ => Err(format!("expected a line or a range like 10-14, lines counted from 1, not {text:?}")),
    }
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
        assert!(parse_date("2026-02-31").is_err(), "no such day");
        assert!(parse_date("2028-02-29").is_ok(), "a leap day");
        assert!(parse_date("2026-x-10-20").is_err());
        assert!(parse_date("99999999999999999d").is_err(), "too many days to count");
        assert_eq!(parse_lines("10-14"), Ok((10, 14)));
        assert_eq!(parse_lines("7"), Ok((7, 7)));
        assert!(parse_lines("14-10").is_err());
        assert!(parse_lines("0").is_err());
    }
}
