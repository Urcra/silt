//! Who records the session of an agent the wrapper runs: the daemon, or the
//! wrapper itself when no daemon answers (ADR 0018). A wrapper recording on
//! its own hands the session to a daemon once one answers, and takes it
//! over itself whenever the daemon recording it goes away.

use std::env;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use exn::{Exn, ResultExt};
use silt_agents::Agent;
use silt_daemon::client;
use silt_daemon::paths;
use silt_daemon::protocol::{Request, Response, SessionStarted};

use crate::CliError;

/// How often the wrapper looks whether to run the capture step.
const STEP_INTERVAL: Duration = Duration::from_secs(1);
/// How long a daemon may take to answer before it counts as not answering.
const PING_WAIT: Duration = Duration::from_millis(500);

/// Its end is recorded on every way out of the wrapper, errors included;
/// otherwise later edits would be credited to an agent that is gone.
pub(crate) struct Recording {
    runtime: tokio::runtime::Runtime,
    local: Local,
    signals: Signals,
}

/// The hangup and termination the wrapper passes on, listened for from the
/// start: one that comes before the agent runs is passed on once it does,
/// rather than ending the wrapper with the session unrecorded.
#[cfg(unix)]
struct Signals {
    hangup: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl Signals {
    fn listen(runtime: &tokio::runtime::Runtime) -> io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        let _entered = runtime.enter();
        Ok(Self { hangup: signal(SignalKind::hangup())?, terminate: signal(SignalKind::terminate())? })
    }
}

#[cfg(windows)]
struct Signals;

#[cfg(windows)]
impl Signals {
    fn listen(_runtime: &tokio::runtime::Runtime) -> io::Result<Self> {
        Ok(Self)
    }
}

struct Local {
    thread: String,
    /// While the daemon records the session, its recorder here, if any, is
    /// kept to take the session back.
    recorder: Option<Box<silt_daemon::Recorder>>,
    by_daemon: bool,
    /// Whether a daemon that answers may take the session over.
    hand_over: bool,
    ended: bool,
}

impl Recording {
    /// A session the daemon started and records.
    pub(crate) fn by_daemon(thread: &str) -> Result<Self, Exn<CliError>> {
        let local = Local { thread: thread.to_owned(), recorder: None, by_daemon: true, hand_over: true, ended: false };
        let runtime = runtime()?;
        let signals = listen(&runtime)?;
        Ok(Self { runtime, local, signals })
    }

    /// Starts the agent's session in this process. With `hand_over`, a
    /// daemon that starts answering takes it over.
    pub(crate) fn here(
        agent: &'static dyn Agent,
        cwd: &Path,
        session_id: Option<&str>,
        isolated: bool,
        hand_over: bool,
    ) -> Result<(SessionStarted, Self), Exn<CliError>> {
        let runtime = runtime()?;
        let signals = listen(&runtime)?;
        let run = silt_daemon::NewRun { cwd, agent: agent.id(), session_id, isolated };
        let (recorder, started) = runtime
            .block_on(silt_daemon::Recorder::start(&paths::data_dir(), run))
            .or_raise(|| CliError("could not start recording the session".into()))?;
        let local = Local {
            thread: started.thread.clone(),
            recorder: Some(Box::new(recorder)),
            by_daemon: false,
            hand_over,
            ended: false,
        };
        Ok((started, Self { runtime, local, signals }))
    }

    /// Runs `command` until it exits, stepping meanwhile.
    pub(crate) fn run(&mut self, command: Command) -> io::Result<std::process::ExitStatus> {
        self.runtime.block_on(run_until_exit(command, &mut self.local, &mut self.signals))
    }

    pub(crate) fn end(mut self) -> Result<(), Exn<CliError>> {
        self.end_once()
    }

    fn end_once(&mut self) -> Result<(), Exn<CliError>> {
        let local = &mut self.local;
        if local.ended {
            return Ok(());
        }
        local.ended = true;
        let thread = local.thread.clone();
        let by_daemon = local.by_daemon
            && self.runtime.block_on(daemon_answers())
            && client::send(&paths::socket_path(), &Request::EndSession { thread: thread.clone() }, None)
                .is_ok_and(|response| matches!(response, Response::Ok { .. }));
        if by_daemon {
            return Ok(());
        }
        // The daemon recording the session went away, or stopped answering.
        let recorder = match local.recorder.take() {
            Some(recorder) => *recorder,
            None => self
                .runtime
                .block_on(silt_daemon::Recorder::attach(&paths::data_dir(), &thread))
                .or_raise(|| CliError("could not record the end of the session".into()))?,
        };
        let ended = self.runtime.block_on(recorder.end()).or_raise(|| CliError("could not record the end of the session".into()));
        push_in_background(&thread);
        ended
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        let _ = self.end_once();
    }
}

fn listen(runtime: &tokio::runtime::Runtime) -> Result<Signals, Exn<CliError>> {
    Signals::listen(runtime).or_raise(|| CliError("cannot listen for signals".into()))
}

fn runtime() -> Result<tokio::runtime::Runtime, Exn<CliError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_raise(|| CliError("cannot start the async runtime".into()))
}

impl Local {
    /// Runs the capture step unless the daemon records the session, taking
    /// the session over from a daemon that went away, and hands it to a
    /// daemon that answers now.
    async fn step(&mut self) {
        if self.by_daemon {
            if silt_daemon::holds_lock(&paths::data_dir()) {
                return;
            }
            if self.recorder.is_none() {
                match silt_daemon::Recorder::attach(&paths::data_dir(), &self.thread).await {
                    Ok(recorder) => self.recorder = Some(Box::new(recorder)),
                    Err(error) => {
                        eprintln!("silt: could not take over recording: {}", silt_daemon::protocol::error_chain(&error));
                        return;
                    }
                }
            }
            self.by_daemon = false;
        }
        let recorder = self.recorder.as_mut().expect("recording here");
        if let Err(error) = recorder.tick().await {
            eprintln!("silt: could not record: {}", silt_daemon::protocol::error_chain(&error));
        }
        if self.hand_over && recorder.found() && daemon_answers().await {
            self.by_daemon = adopted(&self.thread).await;
        }
    }
}

async fn daemon_answers() -> bool {
    if !silt_daemon::holds_lock(&paths::data_dir()) {
        return false;
    }
    let pinged = tokio::task::spawn_blocking(|| client::send(&paths::socket_path(), &Request::Ping, Some(PING_WAIT)).is_ok());
    pinged.await.unwrap_or(false)
}

/// Whether the daemon took over the thread's session. It is waited for
/// however long it takes, as it may have taken over by the time it answers.
async fn adopted(thread: &str) -> bool {
    let request = Request::Adopt { thread: thread.to_owned() };
    let sent = tokio::task::spawn_blocking(move || client::send(&paths::socket_path(), &request, None));
    matches!(sent.await, Ok(Ok(Response::Ok { .. })))
}

/// Runs the agent until it exits. Ctrl-C and Ctrl-\ reach the agent from
/// the terminal, which handles them; this wrapper stays to record the end of
/// the session. A hangup or termination is passed on to the agent, and the
/// wrapper still waits for it to exit. It steps every `STEP_INTERVAL`
/// meanwhile.
#[cfg(unix)]
async fn run_until_exit(command: Command, local: &mut Local, signals: &mut Signals) -> io::Result<std::process::ExitStatus> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut child = tokio::process::Command::from(command).spawn()?;
    let (mut interrupt, mut quit) = (signal(SignalKind::interrupt())?, signal(SignalKind::quit())?);
    let Signals { hangup, terminate } = signals;
    let pass_on = |child: &tokio::process::Child, sent: rustix::process::Signal| {
        if let Some(pid) = child.id().and_then(|id| rustix::process::Pid::from_raw(id.cast_signed())) {
            let _ = rustix::process::kill_process(pid, sent);
        }
    };
    let mut steps = tokio::time::interval(STEP_INTERVAL);
    steps.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            status = child.wait() => return status,
            _ = interrupt.recv() => {}
            _ = quit.recv() => {}
            _ = hangup.recv() => pass_on(&child, rustix::process::Signal::HUP),
            _ = terminate.recv() => pass_on(&child, rustix::process::Signal::TERM),
            _ = steps.tick() => local.step().await,
        }
    }
}

/// On Windows, Ctrl-C and Ctrl-Break reach every process on the console,
/// the agent's too; this wrapper only stays to record the end.
#[cfg(windows)]
async fn run_until_exit(command: Command, local: &mut Local, _signals: &mut Signals) -> io::Result<std::process::ExitStatus> {
    use tokio::signal::windows::{ctrl_break, ctrl_c};
    let mut child = tokio::process::Command::from(command).spawn()?;
    let (mut interrupt, mut break_key) = (ctrl_c()?, ctrl_break()?);
    let mut steps = tokio::time::interval(STEP_INTERVAL);
    steps.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            status = child.wait() => return status,
            _ = interrupt.recv() => {}
            _ = break_key.recv() => {}
            _ = steps.tick() => local.step().await,
        }
    }
}

/// Starts `silt sync --once` for the thread, detached, when this machine
/// syncs with a server: what was recorded without the daemon reaches it
/// without the agent waiting.
pub(crate) fn push_in_background(thread: &str) {
    if silt_daemon::remote::load_config(&paths::data_dir()).is_none() {
        return;
    }
    let Ok(exe) = env::current_exe() else {
        return;
    };
    let started = silt_os::spawn_detached(
        Command::new(exe)
            .args(["sync", "--once", thread])
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    );
    if let Err(error) = started {
        eprintln!("silt: could not start pushing to the server: {error}");
    }
}
