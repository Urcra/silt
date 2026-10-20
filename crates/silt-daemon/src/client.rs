use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::protocol::{Request, Response};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("could not reach the Silt daemon")]
    Connect(#[source] io::Error),
    #[error("lost the connection to the Silt daemon")]
    Io(#[from] io::Error),
    #[error("the Silt daemon sent an invalid response")]
    Decode(#[from] serde_json::Error),
}

/// Sends one request and waits for its response.
pub fn send(socket: &Path, request: &Request, timeout: Duration) -> Result<Response, ClientError> {
    let mut stream = UnixStream::connect(socket).map_err(ClientError::Connect)?;
    stream.set_read_timeout(Some(timeout))?;
    let mut line = serde_json::to_vec(request)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    Ok(serde_json::from_str(&response)?)
}
