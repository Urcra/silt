use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

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
/// in the runtime dir under a name derived from the data dir. Without a
/// runtime dir it goes in a directory of this user's own under the shared
/// temporary directory, never straight into it.
pub fn socket_path() -> PathBuf {
    let data = data_dir();
    let preferred = data.join("daemon.sock");
    if preferred.as_os_str().len() <= SOCKET_PATH_LIMIT {
        return preferred;
    }
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(|| env::temp_dir().join(format!("silt-{}", uid())), PathBuf::from);
    runtime.join(format!("silt-{:016x}.sock", fnv1a(data.as_os_str().as_encoded_bytes())))
}

/// Makes `dir`, or an existing one, reachable by this user alone. One owned
/// by anyone else is refused: in a shared directory it could be a trap.
pub fn make_private(dir: &Path) -> io::Result<()> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let metadata = fs::symlink_metadata(dir)?;
    if !metadata.is_dir() || metadata.uid() != uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a directory of this user's", dir.display()),
        ));
    }
    if metadata.permissions().mode() & 0o777 != 0o700 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Whether `path` was made by this user, so it is safe to talk to.
pub fn owned_by_me(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.uid() == uid())
}

pub fn uid() -> u32 {
    rustix::process::getuid().as_raw()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

pub fn log_path() -> PathBuf {
    data_dir().join("daemon.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_directories_are_closed_to_others_and_traps_refused() {
        let root = tempfile::tempdir().expect("temp dir");
        let made = root.path().join("silt");
        make_private(&made).expect("makes");
        assert_eq!(fs::metadata(&made).expect("stat").permissions().mode() & 0o777, 0o700);

        let open = root.path().join("open");
        fs::create_dir(&open).expect("mkdir");
        fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).expect("chmod");
        make_private(&open).expect("tightens");
        assert_eq!(fs::metadata(&open).expect("stat").permissions().mode() & 0o777, 0o700);

        let elsewhere = root.path().join("elsewhere");
        fs::create_dir(&elsewhere).expect("mkdir");
        std::os::unix::fs::symlink(&elsewhere, root.path().join("link")).expect("symlink");
        assert!(make_private(&root.path().join("link")).is_err(), "a link is not a directory of one's own");
    }
}
