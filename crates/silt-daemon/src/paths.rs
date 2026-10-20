use std::env;
use std::path::PathBuf;

/// Where Silt keeps its data: `$SILT_HOME`, else `$XDG_DATA_HOME/silt`, else
/// `~/.local/share/silt`.
pub fn data_dir() -> PathBuf {
    if let Some(home) = env::var_os("SILT_HOME") {
        return PathBuf::from(home);
    }
    if let Some(data) = env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(data).join("silt");
    }
    let home = env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join(".local").join("share").join("silt")
}

/// Unix socket paths must fit in `sun_path`, 108 bytes on Linux.
const SOCKET_PATH_LIMIT: usize = 100;

/// The daemon's socket: in the data dir when its path is short enough, else
/// in the runtime dir under a name derived from the data dir.
pub fn socket_path() -> PathBuf {
    let data = data_dir();
    let preferred = data.join("daemon.sock");
    if preferred.as_os_str().len() <= SOCKET_PATH_LIMIT {
        return preferred;
    }
    let runtime = env::var_os("XDG_RUNTIME_DIR").map_or_else(env::temp_dir, PathBuf::from);
    runtime.join(format!("silt-{:016x}.sock", fnv1a(data.as_os_str().as_encoded_bytes())))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

pub fn log_path() -> PathBuf {
    data_dir().join("daemon.log")
}
