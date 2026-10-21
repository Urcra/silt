//! Turns at writing to the store (ADR 0018). Whoever records something, the
//! daemon handling one event or a hook capturing without it, holds
//! `capture.lock` while it does, so writers take turns and each catches up
//! with the others before it writes. `daemon.lock` only says that a daemon
//! runs.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use silt_types::ThreadId;

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

/// Held by a wrapper recording its agent's thread on its own, for as long
/// as it does: a daemon leaves that thread's checkout and transcripts to
/// it, unless it took the session over.
pub struct RecordingLock {
    file: File,
    path: PathBuf,
}

impl RecordingLock {
    pub fn hold(data_dir: &Path, thread: ThreadId) -> io::Result<Self> {
        let path = recording_lock_path(data_dir, thread);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path)?;
        file.try_lock()?;
        Ok(Self { file, path })
    }

    /// Whether a wrapper records `thread` on its own.
    pub(crate) fn held(data_dir: &Path, thread: ThreadId) -> bool {
        fs::OpenOptions::new()
            .write(true)
            .open(recording_lock_path(data_dir, thread))
            .is_ok_and(|file| matches!(file.try_lock(), Err(TryLockError::WouldBlock)))
    }
}

impl Drop for RecordingLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = self.file.unlock();
    }
}

fn recording_lock_path(data_dir: &Path, thread: ThreadId) -> PathBuf {
    data_dir.join("recording").join(format!("{thread}.lock"))
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

    #[test]
    fn a_thread_is_recorded_elsewhere_while_its_lock_is_held() {
        let dir = tempfile::tempdir().expect("temp dir");
        let thread = ThreadId(7);
        assert!(!RecordingLock::held(dir.path(), thread));
        let lock = RecordingLock::hold(dir.path(), thread).expect("holds");
        assert!(RecordingLock::held(dir.path(), thread));
        assert!(!RecordingLock::held(dir.path(), ThreadId(8)));
        drop(lock);
        assert!(!RecordingLock::held(dir.path(), thread));
    }
}
