use std::fs;
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};

pub fn make_private_dir(dir: &Path) -> io::Result<()> {
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

pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    // `mode` only applies to a new file.
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)
}

pub fn is_executable(metadata: &fs::Metadata) -> bool {
    metadata.permissions().mode() & 0o111 != 0
}

pub fn without_run_extension(_name: &str) -> Option<&str> {
    None
}

pub fn configured_path_dirs() -> Vec<PathBuf> {
    Vec::new()
}

pub fn make_executable(path: &Path) -> io::Result<()> {
    let mode = fs::metadata(path)?.permissions().mode();
    fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o700 | ((mode & 0o044) >> 2)))
}

pub fn spawn_detached(command: &mut std::process::Command) -> io::Result<()> {
    command.process_group(0).spawn().map(drop)
}

pub fn own_group(command: &mut tokio::process::Command) {
    command.process_group(0);
}

pub fn own_group_std(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

#[expect(unsafe_code, reason = "starting a session in the child needs `pre_exec`")]
pub fn own_session_std(command: &mut std::process::Command) {
    // SAFETY: `setsid` is async-signal-safe and touches no memory, so it is
    // sound between fork and exec.
    unsafe {
        command.pre_exec(|| rustix::process::setsid().map(drop).map_err(io::Error::from));
    }
}

pub fn kill_tree(pid: u32) {
    if let Some(group) = rustix::process::Pid::from_raw(pid.cast_signed()) {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
}

fn signal(pid: u32, signal: rustix::process::Signal) -> io::Result<()> {
    let pid = rustix::process::Pid::from_raw(pid.cast_signed()).ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    rustix::process::kill_process(pid, signal).map_err(io::Error::from)
}

pub fn kill(pid: u32) -> io::Result<()> {
    signal(pid, rustix::process::Signal::KILL)
}

pub fn suspend(pid: u32) -> io::Result<()> {
    signal(pid, rustix::process::Signal::STOP)
}

pub fn resume(pid: u32) -> io::Result<()> {
    signal(pid, rustix::process::Signal::CONT)
}

pub fn link(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

pub fn unlink(link: &Path) -> io::Result<()> {
    fs::remove_file(link)
}

pub fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}

pub fn needs_polling(root: &Path) -> bool {
    const V9FS: i64 = 0x0102_1997;
    const FUSE: i64 = 0x6573_5546;
    const CIFS: i64 = 0xFF53_4D42;
    const SMB2: i64 = 0xFE53_4D42;
    const NFS: i64 = 0x6969;
    rustix::fs::statfs(root).is_ok_and(|stats| {
        #[allow(
            clippy::unnecessary_cast,
            reason = "`f_type` is i64 here but a different integer type on other platforms, so #[expect] would not hold everywhere"
        )]
        let kind = stats.f_type as i64;
        [V9FS, FUSE, CIFS, SMB2, NFS].contains(&kind)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_directories_are_closed_to_others_and_traps_refused() {
        let root = tempfile::tempdir().expect("temp dir");
        let made = root.path().join("silt");
        make_private_dir(&made).expect("makes");
        assert_eq!(fs::metadata(&made).expect("stat").permissions().mode() & 0o777, 0o700);

        let open = root.path().join("open");
        fs::create_dir(&open).expect("mkdir");
        fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).expect("chmod");
        make_private_dir(&open).expect("tightens");
        assert_eq!(fs::metadata(&open).expect("stat").permissions().mode() & 0o777, 0o700);

        let elsewhere = root.path().join("elsewhere");
        fs::create_dir(&elsewhere).expect("mkdir");
        std::os::unix::fs::symlink(&elsewhere, root.path().join("link")).expect("symlink");
        assert!(make_private_dir(&root.path().join("link")).is_err(), "a link is not a directory of one's own");
    }

    #[test]
    fn private_files_are_closed_to_others_even_when_they_were_not() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("token");
        fs::write(&path, "old").expect("write");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmod");
        write_private(&path, b"new").expect("writes");
        assert_eq!(fs::metadata(&path).expect("stat").permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn executable_where_readable() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("hook");
        fs::write(&path, "#!/bin/sh\n").expect("write");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmod");
        make_executable(&path).expect("chmod +x");
        assert_eq!(fs::metadata(&path).expect("stat").permissions().mode() & 0o777, 0o755);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("chmod");
        make_executable(&path).expect("chmod +x");
        assert_eq!(fs::metadata(&path).expect("stat").permissions().mode() & 0o777, 0o700);
    }
}
