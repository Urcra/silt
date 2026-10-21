//! Who records the session of an agent the wrapper runs: the daemon, or the
//! wrapper itself when no daemon answers (ADR 0018). A wrapper recording on
//! its own hands the session to the daemon once one answers, and takes it
//! back if that daemon goes away.

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
use crate::commands::call;

/// How often the wrapper recording on its own runs the capture step.
const STEP_INTERVAL: Duration = Duration::from_secs(1);
/// How long a daemon may take to answer before it counts as not answering.
const PING_WAIT: Duration = Duration::from_millis(500);

/// Its end is recorded on every way out of the wrapper, errors included;
/// otherwise later edits would be credited to an agent that is gone.
pub(crate) enum Recording {
    Daemon { thread: String, ended: bool },
    Here { runtime: tokio::runtime::Runtime, local: Local },
}

/// A session this process records.
pub(crate) struct Local {
    thread: String,
    /// `None` once the session's end is recorded.
    recorder: Option<Box<silt_daemon::Recorder>>,
    /// Whether a daemon that answers may take the session over.
    hand_over: bool,
    handed_over: bool,
}

impl Recording {
    pub(crate) fn by_daemon(thread: &str) -> Self {
        Self::Daemon { thread: thread.to_owned(), ended: false }
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
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_raise(|| CliError("cannot start the async runtime".into()))?;
        let run = silt_daemon::NewRun { cwd, agent: agent.id(), session_id, isolated };
        let (recorder, started) = runtime
            .block_on(silt_daemon::Recorder::start(&paths::data_dir(), run))
            .or_raise(|| CliError("could not start recording the session".into()))?;
        let local = Local { thread: started.thread.clone(), recorder: Some(Box::new(recorder)), hand_over, handed_over: false };
        Ok((started, Self::Here { runtime, local }))
    }

    /// Runs `command` until it exits, running the capture step meanwhile
    /// when recording here.
    pub(crate) fn run(&mut self, command: Command) -> io::Result<std::process::ExitStatus> {
        match self {
            Self::Daemon { .. } => {
                tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(run_until_exit(command, None))
            }
            Self::Here { runtime, local } => runtime.block_on(run_until_exit(command, Some(local))),
        }
    }

    pub(crate) fn end(mut self) -> Result<(), Exn<CliError>> {
        self.end_once()
    }

    fn end_once(&mut self) -> Result<(), Exn<CliError>> {
        match self {
            Self::Daemon { ended: true, .. } => Ok(()),
            Self::Daemon { thread, ended } => {
                *ended = true;
                call::<String>(&Request::EndSession { thread: thread.clone() })?;
                Ok(())
            }
            Self::Here { runtime, local } => {
                let Some(recorder) = local.recorder.take() else {
                    return Ok(());
                };
                // A daemon that went away meanwhile leaves the end to this
                // process.
                if local.handed_over && call::<String>(&Request::EndSession { thread: local.thread.clone() }).is_ok() {
                    return Ok(());
                }
                let ended =
                    runtime.block_on((*recorder).end()).or_raise(|| CliError("could not record the end of the session".into()));
                push_in_background(&local.thread);
                ended
            }
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        let _ = self.end_once();
    }
}

impl Local {
    /// Runs the capture step unless a daemon records the session, and hands
    /// the session to a daemon that answers now.
    async fn step(&mut self) {
        let Some(recorder) = self.recorder.as_mut() else {
            return;
        };
        if self.handed_over {
            if silt_daemon::holds_lock(&paths::data_dir()) {
                return;
            }
            self.handed_over = false;
        }
        if let Err(error) = recorder.tick().await {
            eprintln!("silt: could not record: {}", silt_daemon::protocol::error_chain(&error));
        }
        if self.hand_over && recorder.found() && daemon_answers().await {
            self.handed_over = adopted(&self.thread).await;
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
/// wrapper still waits for it to exit. Recording `local`ly, it steps every
/// `STEP_INTERVAL` meanwhile.
async fn run_until_exit(command: Command, mut local: Option<&mut Local>) -> io::Result<std::process::ExitStatus> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut child = tokio::process::Command::from(command).spawn()?;
    let (mut interrupt, mut quit) = (signal(SignalKind::interrupt())?, signal(SignalKind::quit())?);
    let (mut hangup, mut terminate) = (signal(SignalKind::hangup())?, signal(SignalKind::terminate())?);
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
            _ = steps.tick(), if local.is_some() => local.as_mut().expect("only steps with one").step().await,
        }
    }
}

/// Starts `silt sync --once` for the thread, detached, when this machine
/// syncs with a server: what was recorded without the daemon reaches it
/// without the agent waiting.
pub(crate) fn push_in_background(thread: &str) {
    use std::os::unix::process::CommandExt;
    if silt_daemon::remote::load_config(&paths::data_dir()).is_none() {
        return;
    }
    let Ok(exe) = env::current_exe() else {
        return;
    };
    let started = Command::new(exe)
        .args(["sync", "--once", thread])
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    if let Err(error) = started {
        eprintln!("silt: could not start pushing to the server: {error}");
    }
}
