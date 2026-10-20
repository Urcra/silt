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

pub fn socket_path() -> PathBuf {
    data_dir().join("daemon.sock")
}

pub fn log_path() -> PathBuf {
    data_dir().join("daemon.log")
}
