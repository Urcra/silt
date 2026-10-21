//! Turns at writing to the store (ADR 0018). Whoever records something, the
//! daemon handling one event or a hook capturing without it, holds
//! `capture.lock` while it does, so writers take turns and each catches up
//! with the others before it writes. `daemon.lock` only says that a daemon
//! runs.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::Path;
use std::time::Duration;

/// How long a writer waits between tries: steps take milliseconds.
const RETRY: Duration = Duration::from_millis(2);

pub struct CaptureLock(File);

/// Writing is this process's turn until it is dropped.
pub struct Turn<'a>(&'a File);

impl CaptureLock {
    pub fn open(data_dir: &Path) -> io::Result<Self> {
        let file = fs::OpenOptions::new().create(true).truncate(false).write(true).open(data_dir.join("capture.lock"))?;
        Ok(Self(file))
    }

    /// Waits for this process's turn, without holding up the runtime.
    pub async fn turn(&self) -> io::Result<Turn<'_>> {
        loop {
            match self.0.try_lock() {
                Ok(()) => return Ok(Turn(&self.0)),
                Err(TryLockError::WouldBlock) => tokio::time::sleep(RETRY).await,
                Err(TryLockError::Error(error)) => return Err(error),
            }
        }
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writers_take_turns() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (daemon, hook) = (CaptureLock::open(dir.path()).expect("opens"), CaptureLock::open(dir.path()).expect("opens"));
        let turn = daemon.turn().await.expect("takes");
        let waiting = tokio::time::timeout(Duration::from_millis(50), hook.turn()).await;
        assert!(waiting.is_err(), "the hook waits while the daemon writes");
        drop(turn);
        let _turn = tokio::time::timeout(Duration::from_secs(1), hook.turn()).await.expect("gets its turn").expect("takes");
    }
}
