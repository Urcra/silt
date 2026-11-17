use std::env;
use std::io;
use std::path::{Path, PathBuf};

/// Where Silt keeps its data: `$SILT_HOME`, else `$XDG_DATA_HOME/silt`, else
/// `~/.local/share/silt`; on Windows `%LOCALAPPDATA%\silt\data`, which
/// doesn't roam, as the store and blobs are too big to. Not
/// `%LOCALAPPDATA%\silt` itself: the desktop app's installer puts the app
/// there, as Windows doesn't tell `Silt` from `silt`.
pub fn data_dir() -> PathBuf {
    if let Some(home) = env::var_os("SILT_HOME") {
        return PathBuf::from(home);
    }
    if cfg!(windows)
        && let Some(local) = env::var_os("LOCALAPPDATA")
    {
        return PathBuf::from(local).join("silt").join("data");
    }
    if let Some(data) = env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(data).join("silt");
    }
    let home = silt_os::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".local").join("share").join("silt")
}

/// Unix socket paths must fit in `sun_path`, 108 bytes on Linux.
#[cfg(unix)]
const SOCKET_PATH_LIMIT: usize = 100;

pub fn socket_path() -> PathBuf {
    socket_path_in(&data_dir())
}

/// The daemon's socket: in the data dir when its path is short enough, else
/// in the runtime dir under a name derived from the data dir. Without a
/// runtime dir it goes in a directory of this user's own under the shared
/// temporary directory, never straight into it.
#[cfg(unix)]
pub fn socket_path_in(data: &Path) -> PathBuf {
    let preferred = data.join("daemon.sock");
    if preferred.as_os_str().len() <= SOCKET_PATH_LIMIT {
        return preferred;
    }
    let runtime =
        env::var_os("XDG_RUNTIME_DIR").map_or_else(|| env::temp_dir().join(format!("silt-{}", silt_os::uid())), PathBuf::from);
    runtime.join(format!("silt-{:016x}.sock", fnv1a(data.as_os_str().as_encoded_bytes())))
}

/// Inside Silt's sandbox: the socket that answers for this run, and the
/// run's token, which the sandbox's launch sets (ADR 0093).
pub fn run_channel() -> Option<(PathBuf, String)> {
    let socket = env::var_os("SILT_RUN_SOCKET").filter(|socket| !socket.is_empty())?;
    let token = env::var("SILT_RUN_TOKEN").ok().filter(|token| !token.is_empty())?;
    Some((PathBuf::from(socket), token))
}

/// The socket sandboxed agents reach the daemon through (ADR 0093): beside
/// the daemon's, answering only what carries a run's token.
#[cfg(unix)]
pub fn agents_socket_in(data: &Path) -> PathBuf {
    let socket = socket_path_in(data);
    let name = socket.file_stem().map(|stem| stem.to_string_lossy().replace("daemon", "")).unwrap_or_default();
    socket.with_file_name(format!("{}agents.sock", if name.is_empty() { String::new() } else { format!("{name}-") }))
}

/// The daemon's pipe, named after the data dir, so each data dir, and each
/// test's, has its own.
#[cfg(windows)]
pub fn socket_path_in(data: &Path) -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\silt-{:016x}", fnv1a(data.as_os_str().as_encoded_bytes())))
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
