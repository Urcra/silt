use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::time::Duration;

use crate::protocol::{Request, Response};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("could not reach the Silt daemon")]
    Connect(#[source] io::Error),
    #[error("the Silt daemon took too long to answer; it may still finish the request")]
    TimedOut,
    #[error("lost the connection to the Silt daemon")]
    Io(#[source] io::Error),
    #[error("the Silt daemon sent an invalid response")]
    Decode(#[from] serde_json::Error),
}

impl From<io::Error> for ClientError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Self::TimedOut,
            _ => Self::Io(error),
        }
    }
}

/// Sends one request and waits for its response, at most `timeout`: none
/// for a request whose work must not be abandoned half way, since the
/// daemon carries it out whether or not anyone waits.
pub fn send(socket: &Path, request: &Request, timeout: Option<Duration>) -> Result<Response, ClientError> {
    let mut line = serde_json::to_vec(request)?;
    line.push(b'\n');
    let response = exchange(socket, line, timeout)?;
    Ok(serde_json::from_str(&response)?)
}

#[cfg(unix)]
fn exchange(socket: &Path, line: Vec<u8>, timeout: Option<Duration>) -> Result<String, ClientError> {
    // Someone else's socket in a shared directory would be an impostor.
    if socket.exists() && !silt_os::owned_by_me(socket) {
        return Err(ClientError::Connect(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} belongs to another user", socket.display()),
        )));
    }
    let mut stream = std::os::unix::net::UnixStream::connect(socket).map_err(ClientError::Connect)?;
    stream.set_read_timeout(timeout)?;
    stream.write_all(&line)?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    Ok(response)
}

/// A pipe has no read timeout, so the answer is waited for on a thread of
/// its own, left behind if it takes too long.
#[cfg(windows)]
fn exchange(socket: &Path, line: Vec<u8>, timeout: Option<Duration>) -> Result<String, ClientError> {
    let mut pipe = silt_os::pipe::connect(socket).map_err(ClientError::Connect)?;
    let (sender, answer) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let exchanged = pipe.write_all(&line).and_then(|()| {
            let mut response = String::new();
            BufReader::new(pipe).read_line(&mut response).map(|_| response)
        });
        let _ = sender.send(exchanged);
    });
    let answered = match timeout {
        Some(timeout) => answer.recv_timeout(timeout).map_err(|_| ClientError::TimedOut)?,
        None => answer.recv().map_err(|_| ClientError::TimedOut)?,
    };
    Ok(answered?)
}
