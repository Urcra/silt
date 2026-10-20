use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use silt_store::Store;
use silt_types::ActorId;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use crate::core::{Core, CoreError, Event};
use crate::protocol::{Request, Response};

const TAIL_INTERVAL: Duration = Duration::from_millis(300);
const SHUTDOWN_REPLY_WAIT: Duration = Duration::from_secs(2);

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
}

/// Runs the daemon for the data in `data_dir` until asked to shut down.
pub async fn run(data_dir: &Path, socket: &Path) -> Result<(), RunError> {
    fs::create_dir_all(data_dir)?;
    let store = Store::open(data_dir).await?;
    let actor = load_actor(data_dir)?;
    let (events, mut inbox) = mpsc::unbounded_channel();
    let mut core = Core::load(store, actor, data_dir, events.clone()).await?;

    if socket.exists() {
        fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket).map_err(|source| RunError::Listen {
        path: socket.display().to_string(),
        source,
    })?;
    let accept_events = events.clone();
    let (answered, mut answered_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve(stream, accept_events.clone(), answered.clone()));
        }
    });
    let tick_events = events.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TAIL_INTERVAL);
        loop {
            interval.tick().await;
            if tick_events.send(Event::Tick).is_err() {
                break;
            }
        }
    });

    while let Some(event) = inbox.recv().await {
        match event {
            Event::Request(Request::Shutdown, reply) => {
                core.tick().await;
                let _ = fs::remove_file(socket);
                let _ = reply.send(Response::ok("shutting down"));
                let _ = tokio::time::timeout(SHUTDOWN_REPLY_WAIT, answered_rx.recv()).await;
                return Ok(());
            }
            Event::Request(request, reply) => core.request(request, reply).await,
            Event::Changed { checkout, paths } => core.changed(checkout, paths).await,
            Event::Tick => core.tick().await,
            Event::CloseTool { session_id, reply } => core.close_tool(&session_id, reply).await,
            Event::Discovered { token, found } => core.discovered(token, found).await,
            Event::Snapshotted { session_id, result } => core.snapshotted(&session_id, result).await,
            Event::Pushed {
                family,
                generation,
                outcome,
                error,
            } => core.pushed(family, generation, outcome, error).await,
            Event::Pulled {
                family,
                generation,
                outcome,
                error,
            } => core.pulled(family, generation, outcome, error).await,
            Event::Listed(listing) => core.listed(listing).await,
        }
    }
    let _ = fs::remove_file(socket);
    Ok(())
}

/// Serves one connection. `answered` hears when a shutdown request has been
/// answered, since the daemon exits, and drops this task, right after.
async fn serve(stream: UnixStream, events: mpsc::UnboundedSender<Event>, answered: mpsc::UnboundedSender<()>) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let request = serde_json::from_str::<Request>(&line);
        let shutdown = matches!(request, Ok(Request::Shutdown));
        let response = match request {
            Ok(request) => {
                let (reply, response) = oneshot::channel();
                if events.send(Event::Request(request, reply)).is_err() {
                    break;
                }
                response.await.unwrap_or_else(|_| Response::error("the daemon stopped"))
            }
            Err(error) => Response::error(format!("invalid request: {error}")),
        };
        let written = write_response(&mut write, &response).await;
        if shutdown {
            let _ = answered.send(());
        }
        if let Err(error) = written {
            warn!(%error, "client went away");
            break;
        }
    }
}

async fn write_response(write: &mut OwnedWriteHalf, response: &Response) -> io::Result<()> {
    let mut encoded = serde_json::to_vec(response)?;
    encoded.push(b'\n');
    write.write_all(&encoded).await?;
    write.flush().await
}

/// This daemon's actor, kept across restarts so its deltas stay one chain.
fn load_actor(data_dir: &Path) -> io::Result<ActorId> {
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
