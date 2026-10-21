use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
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
    // Someone else's socket in a shared directory would be an impostor.
    if socket.exists() && !crate::paths::owned_by_me(socket) {
        return Err(ClientError::Connect(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} belongs to another user", socket.display()),
        )));
    }
    let mut stream = UnixStream::connect(socket).map_err(ClientError::Connect)?;
    stream.set_read_timeout(timeout)?;
    let mut line = serde_json::to_vec(request)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    Ok(serde_json::from_str(&response)?)
}
