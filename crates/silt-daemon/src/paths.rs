use std::env;
use std::io;
use std::path::{Path, PathBuf};

/// Where Silt keeps its data: `$SILT_HOME`, else `$XDG_DATA_HOME/silt`, else
/// `~/.local/share/silt`; on Windows `%LOCALAPPDATA%\silt`, which doesn't
/// roam, as the store and blobs are too big to.
pub fn data_dir() -> PathBuf {
    if let Some(home) = env::var_os("SILT_HOME") {
        return PathBuf::from(home);
    }
    if cfg!(windows)
        && let Some(local) = env::var_os("LOCALAPPDATA")
    {
        return PathBuf::from(local).join("silt");
    }
    if let Some(data) = env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(data).join("silt");
    }
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join(".local").join("share").join("silt")
}

/// Unix socket paths must fit in `sun_path`, 108 bytes on Linux.
#[cfg(unix)]
const SOCKET_PATH_LIMIT: usize = 100;

/// The daemon's socket: in the data dir when its path is short enough, else
/// in the runtime dir under a name derived from the data dir. Without a
/// runtime dir it goes in a directory of this user's own under the shared
/// temporary directory, never straight into it.
#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    let data = data_dir();
    let preferred = data.join("daemon.sock");
    if preferred.as_os_str().len() <= SOCKET_PATH_LIMIT {
        return preferred;
    }
    let runtime =
        env::var_os("XDG_RUNTIME_DIR").map_or_else(|| env::temp_dir().join(format!("silt-{}", silt_os::uid())), PathBuf::from);
    runtime.join(format!("silt-{:016x}.sock", fnv1a(data.as_os_str().as_encoded_bytes())))
}

/// The daemon's pipe, named after the data dir, so each data dir, and each
/// test's, has its own.
#[cfg(windows)]
pub fn socket_path() -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\silt-{:016x}", fnv1a(data_dir().as_os_str().as_encoded_bytes())))
}

/// Makes `dir`, or an existing one, reachable by this user alone. One owned
/// by anyone else is refused: in a shared directory it could be a trap.
pub fn make_private(dir: &Path) -> io::Result<()> {
    silt_os::make_private_dir(dir)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

pub fn log_path() -> PathBuf {
    data_dir().join("daemon.log")
}
