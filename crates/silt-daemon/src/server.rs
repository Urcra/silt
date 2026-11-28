use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use silt_store::Store;
use silt_types::ActorId;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use crate::capture_lock::CaptureLock;
use crate::core::{Core, CoreError, Event};
use crate::protocol::{Request, Response};

const TAIL_INTERVAL: Duration = Duration::from_millis(300);
/// How often a daemon with nothing going on looks for new commits and for
/// what was recorded without it: often enough to be soon, rare enough to
/// cost nothing.
const IDLE_TICK: Duration = Duration::from_secs(2);
const SHUTDOWN_REPLY_WAIT: Duration = Duration::from_secs(2);
/// How long one event may hold the core before it's logged: every other
/// event waits behind it, a thread being opened in the web UI too.
const SLOW_EVENT: Duration = Duration::from_millis(250);

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Store(#[from] silt_store::StoreError),
    #[error("could not listen on {path}")]
    Listen { path: String, source: io::Error },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("another Silt daemon already runs for {0}")]
    AlreadyRunning(String),
}

const LOCK_ATTEMPTS: u32 = 10;
const LOCK_RETRY: Duration = Duration::from_millis(100);

fn lock_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("daemon.lock")
}

/// Whether a daemon runs for `data_dir`, answering or not.
pub fn holds_lock(data_dir: &Path) -> bool {
    fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path(data_dir))
        .is_ok_and(|file| matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock)))
}

/// Runs the daemon for the data in `data_dir` until asked to shut down.
pub async fn run(data_dir: &Path, socket: &Path) -> Result<(), RunError> {
    // The data dir holds the device's token and everyone's work. Closing it
    // to other users covers every file in it, whatever its own mode; a
    // process-wide umask would also reach files written into checkouts.
    crate::paths::make_private(data_dir)?;
    #[cfg(unix)]
    if let Some(dir) = socket.parent() {
        crate::paths::make_private(dir)?;
    }
    // One daemon per data directory: a second would take over the socket
    // and fight the first over the store.
    let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(lock_path(data_dir))?;
    // `holds_lock` takes the lock for a moment to look, and a daemon that is
    // stopping lets go shortly: only a lock held for a while means another
    // daemon runs.
    let mut attempts = 0;
    while let Err(error) = lock.try_lock() {
        match error {
            fs::TryLockError::WouldBlock if attempts < LOCK_ATTEMPTS => {
                attempts += 1;
                tokio::time::sleep(LOCK_RETRY).await;
            }
            fs::TryLockError::WouldBlock => return Err(RunError::AlreadyRunning(data_dir.display().to_string())),
            fs::TryLockError::Error(error) => return Err(error.into()),
        }
    }
    let store = Store::open_long_lived(data_dir).await?;
    let actor = load_actor(data_dir)?;
    let (events, mut inbox) = mpsc::unbounded_channel();
    let capture_lock = CaptureLock::open(data_dir)?;

    // Taking requests before everything is loaded: they wait in the queue
    // for the load, and come before the rescans it queues (ADR 0034).
    let views = crate::core::Views::default();
    let queues = Queues { events: events.clone(), views: views.sender.clone() };
    let (answered, mut answered_rx) = mpsc::unbounded_channel();
    listen(socket, queues, answered)?;
    #[cfg(unix)]
    listen_for_agents(&crate::paths::agents_socket_in(data_dir), events.clone())?;
    // Without a turn, so hooks capturing meanwhile don't wait on a long
    // load; what they store is taken in at the first turn.
    let mut core = match Core::load(store, actor, data_dir, events.clone()).await {
        Ok(core) => core,
        Err(error) => {
            remove_sockets(socket, data_dir);
            return Err(error.into());
        }
    };
    // After the requests that came during the load: rows a crash left
    // unwritten, then a store from an older Silt compacted.
    core.views = views;
    core.queue_index_pending();
    core.start_lan_if_wanted().await;
    let _ = events.send(Event::Compact);
    let tick_events = events.clone();
    let idle = Arc::new(AtomicBool::new(false));
    let ticking_idle = Arc::clone(&idle);
    tokio::spawn(async move {
        loop {
            let wait = if ticking_idle.load(Ordering::Relaxed) { IDLE_TICK } else { TAIL_INTERVAL };
            tokio::time::sleep(wait).await;
            if tick_events.send(Event::Tick).is_err() {
                break;
            }
        }
    });

    loop {
        // What a page shows comes first: a thread switched to waits for no
        // capture queued before it.
        let event = tokio::select! {
            biased;
            Some(event) = core.views.receiver.recv() => event,
            event = inbox.recv() => match event {
                Some(event) => event,
                None => break,
            },
        };
        let began = Instant::now();
        let what = label(&event);
        idle.store(core.idle(), Ordering::Relaxed);
        let _turn = capture_lock.turn().await?;
        if let Err(error) = core.catch_up_with_store().await {
            warn!(%error, "could not take in what was recorded without the daemon");
        }
        if let Err(error) = core.capture_spooled().await {
            warn!(%error, "could not record a hook left for later");
        }
        let Some(event) = core.unless_loading(event) else {
            continue;
        };
        match event {
            Event::Request(Request::Shutdown, reply) => {
                core.end_agents().await;
                core.tick().await;
                core.keep_digests().await;
                if let Err(error) = core.store.checkpoint().await {
                    warn!(%error, "could not copy the WAL into the database");
                }
                remove_sockets(socket, data_dir);
                let _ = reply.send(Response::ok("shutting down"));
                let _ = tokio::time::timeout(SHUTDOWN_REPLY_WAIT, answered_rx.recv()).await;
                hold_until_exit(lock);
                return Ok(());
            }
            Event::Request(request, reply) => core.request(request, reply).await,
            Event::Changed { checkout, paths, noticed, settled } => core.changed(checkout, paths, noticed, settled).await,
            Event::Loaded { family, result, size, histories } => core.loaded(family, *result, size, histories),
            Event::HistoryRead { thread, whole, copy } => core.history_read(thread, whole.map(|whole| *whole), copy),
            Event::Digests => core.take_stale_digest().await,
            Event::Tick => {
                core.tick().await;
                core.restart_if_idle();
            }
            Event::Discovered { token, found } => core.discovered(token, found).await,
            Event::Importing { run, reply } => core.import_next(run, reply).await,
            Event::Snapshotted { session_id, result } => core.snapshotted(&session_id, result).await,
            Event::Drain { session_id } => core.drain(&session_id).await,
            Event::Compact => {
                core.finish_erasures().await;
                core.compact_if_worth_it().await;
            }
            Event::IndexPending => {
                if let Err(error) = core.index_pending().await {
                    warn!(%error, "could not write the search rows of pulled deltas; they're written at the next start");
                }
            }
            Event::Acp { thread, run, event } => core.acp_event(thread, run, event).await,
            Event::AgentProbed { agent, offered } => core.agent_probed(agent, offered),
            Event::Judged(judged) => core.judged(*judged).await,
            Event::EscalationWaited { thread, id } => core.escalation_waited(thread, id),
            Event::Pushed { family, epoch, generation, outcome, error } => {
                core.pushed(family, epoch, generation, outcome, error).await
            }
            Event::Pulled { family, epoch, generation, outcome, error } => {
                core.pulled(family, epoch, generation, outcome, error).await
            }
            Event::Listed { server, listing, requested } => core.listed(&server, listing, requested).await,
            Event::OrgPolicies { server, policies } => core.org_policies_fetched(&server, policies).await,
            Event::Personal(synced) => core.personal_synced(*synced).await,
            Event::NotesSettings { server, projects } => core.notes_settings_read(&server, projects),
            Event::Control(event) => core.control_event(event).await,
        }
        if let Err(error) = core.end_turn().await {
            warn!(%error, "could not note what the store holds");
        }
        let took = began.elapsed();
        if took >= SLOW_EVENT {
            warn!(event = %what, ms = took.as_millis(), "an event held the daemon; everything else waited");
        }
    }
    remove_sockets(socket, data_dir);
    hold_until_exit(lock);
    Ok(())
}

/// What kind of event it is, to log. Only a request's name: its fields
/// may hold a secret, or a hook's whole payload.
fn label(event: &Event) -> String {
    let name = match event {
        Event::Request(request, _) => {
            let mut name = Name(String::new());
            let _ = write!(name, "{request:?}");
            return name.0;
        }
        Event::Digests => "Digests",
        Event::Loaded { .. } => "Loaded",
        Event::HistoryRead { .. } => "HistoryRead (whole)",
        Event::Changed { paths, .. } => return format!("Changed ({} paths)", paths.len()),
        Event::Tick => "Tick",
        Event::Pushed { .. } => "Pushed",
        Event::Pulled { .. } => "Pulled",
        Event::Listed { .. } => "Listed",
        Event::OrgPolicies { .. } => "OrgPolicies",
        Event::Personal(_) => "Personal",
        Event::Discovered { .. } => "Discovered",
        Event::Importing { .. } => "Importing",
        Event::Snapshotted { .. } => "Snapshotted",
        Event::Drain { .. } => "Drain",
        Event::Compact => "Compact",
        Event::IndexPending => "IndexPending",
        Event::Acp { .. } => "Acp",
        Event::AgentProbed { .. } => "AgentProbed",
        Event::Judged(_) => "Judged",
        Event::EscalationWaited { .. } => "EscalationWaited",
        Event::Control(_) => "Control",
        Event::NotesSettings { .. } => "NotesSettings",
    };
    name.to_owned()
}

/// A value's `Debug` up to its variant's name, and no further: formatting
/// stops there, so a large request costs nothing to name.
struct Name(String);

impl std::fmt::Write for Name {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = text.find(|char: char| !char.is_alphanumeric() && char != '_').unwrap_or(text.len());
        self.0.push_str(&text[..end]);
        if end < text.len() { Err(std::fmt::Error) } else { Ok(()) }
    }
}

/// Keeps the daemon's lock until the process exits. A free lock tells
/// `silt daemon stop` the store is left alone, and the store's connections
/// close on their own threads after `run` returns.
/// Removes the daemon's sockets, so nothing finds one no daemon answers,
/// and a copy of the data dir copies none.
fn remove_sockets(socket: &Path, data_dir: &Path) {
    let _ = fs::remove_file(socket);
    #[cfg(unix)]
    let _ = fs::remove_file(crate::paths::agents_socket_in(data_dir));
    #[cfg(not(unix))]
    let _ = data_dir;
}

fn hold_until_exit(lock: fs::File) {
    std::mem::forget(lock);
}

/// Takes connections on the socket, from this user's own processes only:
/// whoever reaches it controls the daemon.
#[cfg(unix)]
fn listen(socket: &Path, queues: Queues, answered: mpsc::UnboundedSender<()>) -> Result<(), RunError> {
    use std::os::unix::fs::PermissionsExt as _;
    if socket.exists() {
        fs::remove_file(socket)?;
    }
    let listener = tokio::net::UnixListener::bind(socket)
        .map_err(|source| RunError::Listen { path: socket.display().to_string(), source })?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    let me = silt_os::uid();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            if stream.peer_cred().is_ok_and(|peer| peer.uid() == me) {
                tokio::spawn(serve(stream, queues.clone(), answered.clone()));
            }
        }
    });
    Ok(())
}

/// Takes connections from sandboxed agents' runs (ADR 0093). Whatever
/// reaches it may ask only what a run's token allows, so the person's
/// authority never enters a sandbox.
#[cfg(unix)]
fn listen_for_agents(socket: &Path, events: mpsc::UnboundedSender<Event>) -> Result<(), RunError> {
    use std::os::unix::fs::PermissionsExt as _;
    if socket.exists() {
        fs::remove_file(socket)?;
    }
    let listener = tokio::net::UnixListener::bind(socket)
        .map_err(|source| RunError::Listen { path: socket.display().to_string(), source })?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    let me = silt_os::uid();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            if stream.peer_cred().is_ok_and(|peer| peer.uid() == me) {
                let (answered, _) = mpsc::unbounded_channel();
                // A sandboxed run's requests get no place ahead of the rest.
                let queues = Queues { events: events.clone(), views: events.clone() };
                tokio::spawn(serve_as(stream, queues, answered, Authority::Run));
            }
        }
    });
    Ok(())
}

/// Takes connections on the pipe, which only this user can open. Each
/// client gets the waiting instance, and a new one waits for the next.
#[cfg(windows)]
fn listen(socket: &Path, queues: Queues, answered: mpsc::UnboundedSender<()>) -> Result<(), RunError> {
    let listen_error = |source| RunError::Listen { path: socket.display().to_string(), source };
    let name = socket.as_os_str().to_owned();
    let mut waiting = silt_os::pipe::serve(&name, true).map_err(listen_error)?;
    tokio::spawn(async move {
        while waiting.connect().await.is_ok() {
            let next = match silt_os::pipe::serve(&name, false) {
                Ok(next) => next,
                Err(error) => {
                    warn!(%error, "could not wait for the next client");
                    break;
                }
            };
            tokio::spawn(serve(std::mem::replace(&mut waiting, next), queues.clone(), answered.clone()));
        }
    });
    Ok(())
}

/// Whose requests a connection carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authority {
    /// The person's: anything.
    Person,
    /// A sandboxed run's: only requests that carry its token.
    Run,
}

/// Serves one connection. `answered` hears when a shutdown request has been
/// answered, since the daemon exits, and drops this task, right after.
async fn serve(stream: impl AsyncRead + AsyncWrite + Send + 'static, queues: Queues, answered: mpsc::UnboundedSender<()>) {
    serve_as(stream, queues, answered, Authority::Person).await;
}

/// Where a connection's requests go: those that only read what a page shows
/// ahead of the rest (ADR 0127).
#[derive(Clone)]
struct Queues {
    events: mpsc::UnboundedSender<Event>,
    views: mpsc::UnboundedSender<Event>,
}

async fn serve_as(
    stream: impl AsyncRead + AsyncWrite + Send + 'static,
    queues: Queues,
    answered: mpsc::UnboundedSender<()>,
    authority: Authority,
) {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let request = serde_json::from_str::<Request>(&line);
        let shutdown = matches!(request, Ok(Request::Shutdown));
        let response = match request {
            // Asks only whether the daemon is there, so it is answered even
            // while the daemon loads or works through a long event.
            Ok(Request::Ping) => Response::ok("pong"),
            Ok(request) if authority == Authority::Run && !matches!(request, Request::Scoped { .. }) => {
                Response::error("a sandboxed agent's run may only ask with its run's token")
            }
            Ok(request) => {
                let hook = matches!(request, Request::Hook { .. });
                let (reply, response) = oneshot::channel();
                let to = if request.is_view() { &queues.views } else { &queues.events };
                if to.send(Event::Request(request, reply)).is_err() {
                    break;
                }
                let answer = if hook {
                    // A hook's connection carries it alone and closes once the
                    // hook stops waiting, which the daemon then leaves to it.
                    tokio::select! {
                        answer = response => answer,
                        _ = lines.next_line() => break,
                    }
                } else {
                    response.await
                };
                answer.unwrap_or_else(|_| Response::error("the daemon stopped"))
            }
            Err(error) => Response::error(format!("invalid request: {error}")),
        };
        let written = write_response(&mut write, &response).await;
        if shutdown {
            // Windows drops what a pipe still holds once its server closes it,
            // so the daemon exits only after the client read the answer and left.
            if written.is_ok() {
                let _ = lines.next_line().await;
            }
            let _ = answered.send(());
        }
        if let Err(error) = written {
            warn!(%error, "client went away");
            break;
        }
    }
}

async fn write_response(write: &mut (impl AsyncWrite + Unpin), response: &Response) -> io::Result<()> {
    let mut encoded = serde_json::to_vec(response)?;
    encoded.push(b'\n');
    write.write_all(&encoded).await?;
    write.flush().await
}

/// This daemon's actor, kept across restarts so its deltas stay one chain.
pub(crate) fn load_actor(data_dir: &Path) -> io::Result<ActorId> {
    let path = data_dir.join("actor");
    if let Ok(text) = fs::read_to_string(&path)
        && let Ok(actor) = text.trim().parse::<ActorId>()
    {
        return Ok(actor);
    }
    let actor = ActorId(uuid::Uuid::new_v4().as_u128());
    fs::write(&path, actor.to_string())?;
    Ok(actor)
}
