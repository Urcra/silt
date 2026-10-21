//! Recording an agent's session from the wrapper that runs it, without the
//! daemon (ADR 0018). The wrapper starts the thread, or takes it over from
//! a daemon that went away, runs the capture step every second for an agent
//! without hooks, and once more when the agent exits; an agent with hooks
//! captures through them. Each step is a turn at writing that first takes
//! in what others stored.

use std::path::Path;

use silt_store::Store;
use silt_types::ThreadId;

use crate::capture::turn_in_time;
use crate::capture_lock::{CaptureLock, RecordingLock};
use crate::core::{Core, CoreError};
use crate::protocol::SessionStarted;

pub struct Recorder {
    core: Core,
    lock: CaptureLock,
    thread: ThreadId,
    _recording: RecordingLock,
}

/// The session a wrapper starts, as `Request::StartSession` asks the daemon.
pub struct NewRun<'a> {
    pub cwd: &'a Path,
    pub agent: &'a str,
    pub session_id: Option<&'a str>,
    pub isolated: bool,
}

impl Recorder {
    /// Starts a thread for the agent's session in the clone at `run.cwd`,
    /// for the data in `data_dir`.
    pub async fn start(data_dir: &Path, run: NewRun<'_>) -> Result<(Self, SessionStarted), CoreError> {
        // The first thing Silt runs on a machine may be this.
        crate::paths::make_private(data_dir)?;
        let lock = CaptureLock::open(data_dir)?;
        let turn = turn_in_time(&lock).await?;
        let store = Store::open(data_dir).await?;
        let actor = crate::server::load_actor(data_dir)?;
        let mut core = Core::visiting(store, actor, data_dir).await?;
        let started = core.start_session(run.cwd, run.agent, run.session_id, run.isolated, false).await?;
        let thread = core.find_thread(&started.thread)?;
        // Before this turn ends, so a daemon catching up never takes the
        // thread for one it records.
        let recording = RecordingLock::hold(data_dir, thread)?;
        core.end_turn().await?;
        drop(turn);
        Ok((Self { core, lock, thread, _recording: recording }, started))
    }

    /// Takes over recording the running agent of `thread` from a daemon
    /// that went away.
    pub async fn attach(data_dir: &Path, thread: &str) -> Result<Self, CoreError> {
        let lock = CaptureLock::open(data_dir)?;
        let turn = turn_in_time(&lock).await?;
        let store = Store::open(data_dir).await?;
        let actor = crate::server::load_actor(data_dir)?;
        let mut core = Core::visiting(store, actor, data_dir).await?;
        let thread = core.find_thread(thread)?;
        let recording = RecordingLock::hold(data_dir, thread)?;
        core.hold_family(core.threads[&thread].family).await?;
        let sessions = core.store.sessions().await?.into_iter().filter(|record| record.thread == thread).collect();
        core.take_sessions(sessions).await?;
        core.take_over_sessions(thread).await?;
        core.end_turn().await?;
        drop(turn);
        Ok(Self { core, lock, thread, _recording: recording })
    }

    /// Runs the capture step, when the agent needs one.
    pub async fn tick(&mut self) -> Result<(), CoreError> {
        if !self.core.steps(self.thread) {
            return Ok(());
        }
        let _turn = turn_in_time(&self.lock).await?;
        self.core.catch_up_with_store().await?;
        self.core.capture_spooled().await?;
        let stepped = self.core.record_step(self.thread).await;
        self.core.end_turn().await?;
        stepped
    }

    /// Whether the agent's session has been found, so the daemon can take
    /// it over.
    pub fn found(&self) -> bool {
        !self.core.pending.iter().any(|pending| pending.thread == self.thread)
    }

    /// Records the session's end, once the agent exited.
    pub async fn end(mut self) -> Result<(), CoreError> {
        let _turn = turn_in_time(&self.lock).await?;
        self.core.catch_up_with_store().await?;
        self.core.capture_spooled().await?;
        self.core.end_session(self.thread).await?;
        self.core.end_turn().await
    }
}
