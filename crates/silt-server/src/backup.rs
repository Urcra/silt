//! Backups of a server's data dir (ADR 0013), safe while it runs: the
//! database is written with `VACUUM INTO` first, then the blobs, git
//! mirrors and config are copied. Blobs and mirror objects are only ever
//! added, so everything the database snapshot refers to is in the copy.

use std::fs;
use std::io;
use std::path::Path;

use crate::config::CONFIG_FILE;
use crate::{ServerError, now_ms, open_db};

const DATABASE: &str = "server.sqlite";
const COPIED: [&str; 2] = ["blobs", "git"];
const MARKER: &str = "SILT-BACKUP";

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("{0} is not empty")]
    NotEmpty(String),
    #[error("{0} is not a Silt server backup")]
    NotABackup(String),
    #[error(transparent)]
    Server(#[from] ServerError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<sqlx::Error> for BackupError {
    fn from(error: sqlx::Error) -> Self {
        Self::Server(error.into())
    }
}

fn is_empty_dir(path: &Path) -> io::Result<bool> {
    match fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<u64> {
    let mut bytes = 0;
    if !from.exists() {
        return Ok(bytes);
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            bytes += copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            bytes += fs::copy(entry.path(), &target)?;
        }
    }
    Ok(bytes)
}

/// What a backup or restore copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Copied {
    pub bytes: u64,
}

/// Writes a backup of `data_dir` into `dest`, which must be new or empty.
pub async fn backup(data_dir: &Path, dest: &Path) -> Result<Copied, BackupError> {
    if !is_empty_dir(dest)? {
        return Err(BackupError::NotEmpty(dest.display().to_string()));
    }
    fs::create_dir_all(dest)?;
    let db = open_db(data_dir).await?;
    let database = dest.join(DATABASE);
    sqlx::query("VACUUM INTO ?").bind(database.to_string_lossy().into_owned()).execute(&db).await?;
    db.close().await;
    let mut bytes = fs::metadata(&database)?.len();
    for dir in COPIED {
        bytes += copy_tree(&data_dir.join(dir), &dest.join(dir))?;
    }
    if data_dir.join(CONFIG_FILE).exists() {
        bytes += fs::copy(data_dir.join(CONFIG_FILE), dest.join(CONFIG_FILE))?;
    }
    fs::write(dest.join(MARKER), format!("silt {} backup at {} ms\n", env!("CARGO_PKG_VERSION"), now_ms()))?;
    Ok(Copied { bytes })
}

/// Puts a backup in place as `data_dir`, which must hold no server data
/// unless `replace` is set. The server must not be running.
pub fn restore(backup: &Path, data_dir: &Path, replace: bool) -> Result<Copied, BackupError> {
    if !backup.join(MARKER).exists() || !backup.join(DATABASE).exists() {
        return Err(BackupError::NotABackup(backup.display().to_string()));
    }
    if data_dir.join(DATABASE).exists() && !replace {
        return Err(BackupError::NotEmpty(data_dir.display().to_string()));
    }
    fs::create_dir_all(data_dir)?;
    for stale in ["server.sqlite-wal", "server.sqlite-shm"] {
        let path = data_dir.join(stale);
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    let mut bytes = fs::copy(backup.join(DATABASE), data_dir.join(DATABASE))?;
    for dir in COPIED {
        let target = data_dir.join(dir);
        if target.exists() {
            fs::remove_dir_all(&target)?;
        }
        bytes += copy_tree(&backup.join(dir), &target)?;
    }
    if backup.join(CONFIG_FILE).exists() {
        bytes += fs::copy(backup.join(CONFIG_FILE), data_dir.join(CONFIG_FILE))?;
    }
    Ok(Copied { bytes })
}
