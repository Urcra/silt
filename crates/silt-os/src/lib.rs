//! What each operating system does its own way, behind one interface:
//! private files, detached processes, links, finding programs, file
//! systems that need polling, and on Windows the control channel's pipe
//! (ADR 0045).

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

mod wsl;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;
#[cfg(unix)]
pub use unix::{owned_by_me, uid};

#[cfg(windows)]
#[expect(unsafe_code, reason = "Win32 has no safe interface for security descriptors, tokens or a pipe's server")]
mod windows;
#[cfg(windows)]
use windows as imp;
#[cfg(windows)]
pub use windows::pipe;

pub use wsl::{WslPath, in_wsl, is_distribution_name, linux_path, wsl_path};

/// Makes `dir`, or an existing one, reachable by this user alone. One that
/// isn't a plain directory of this user's is refused: in a shared place it
/// could be a trap.
pub fn make_private_dir(dir: &Path) -> io::Result<()> {
    imp::make_private_dir(dir)
}

/// Creates `dir` closed to other users. An existing one is left as it is,
/// since someone else, such as a service's user, may own it on purpose.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    imp::make_private_dir(dir)
}

/// Writes `bytes` to `path`, replacing what was there, for this user's eyes
/// only.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    imp::write_private(path, bytes)
}

/// `path` with links and `..` resolved, as the file system names it. On
/// Windows that's without the `\\?\` prefix `std::fs::canonicalize` adds,
/// which git, agents and shells don't write: a clone found at `C:\src\app`
/// must compare equal to the `C:\src\app` a session records.
pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    Ok(simplified(fs::canonicalize(path)?))
}

/// A verbatim Windows path as plainly as it can be written: `\\?\C:\x` as
/// `C:\x` and `\\?\UNC\host\share` as `\\host\share`. Paths a plain form
/// can't hold, too long or with a reserved name, stay verbatim.
pub fn simplified(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    let plain = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        rest.to_owned()
    } else {
        return path;
    };
    let fits = plain.len() < 260 && plain.split('\\').skip(1).all(|part| part.is_empty() || windows_name(part));
    if fits { PathBuf::from(plain) } else { path }
}

/// A name Windows takes as written outside a verbatim path: no trailing dot
/// or space, no device name.
fn windows_name(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    !part.ends_with(['.', ' ']) && !device
}

/// Makes a rename into `dir` durable, as a directory's fsync does on Unix.
/// Windows can't open a directory as a file, and NTFS journals renames
/// itself, so there it does nothing.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    if cfg!(windows) { Ok(()) } else { fs::File::open(dir)?.sync_all() }
}

/// Whether the file can be run as it is. Windows decides by extension, not
/// by a bit on the file, so nothing on disk says so there.
pub fn is_executable(metadata: &fs::Metadata) -> bool {
    imp::is_executable(metadata)
}

/// Lets the owner run `path`, and everyone who can read it.
pub fn make_executable(path: &Path) -> io::Result<()> {
    imp::make_executable(path)
}

/// Starts `command` so it outlives this process and whatever started it:
/// its own process group, and on Windows out of this console and job. On
/// Windows it inherits no handles, so its stdio is not `command`'s.
pub fn spawn_detached(command: &mut std::process::Command) -> io::Result<()> {
    imp::spawn_detached(command)
}

/// Puts the process `command` starts in its own group, so `kill_tree` stops
/// what it starts too.
pub fn own_group(command: &mut tokio::process::Command) {
    imp::own_group(command);
}

/// Stops the process `pid` started with `own_group`, and everything it
/// started.
pub fn kill_tree(pid: u32) {
    imp::kill_tree(pid);
}

/// Where `program` is, given directly or found on `path`, as the system
/// would run it: on Windows that tries `PATHEXT`'s extensions too.
pub fn find_program(program: &str, path: &OsStr) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    let cwd = std::env::current_dir().ok()?;
    which::which_in(program, Some(path), cwd).ok()
}

/// What's on a `PATH`, each directory listed once, for asking about many
/// programs: [`find_program`] looks in every directory for each one, which
/// is slow where a directory is, such as a Windows drive seen from WSL.
pub struct Programs {
    path: std::ffi::OsString,
    names: std::collections::HashSet<String>,
}

impl Programs {
    pub fn on(path: &OsStr) -> Self {
        let names = std::env::split_paths(path)
            .filter_map(|dir| fs::read_dir(dir).ok())
            .flatten()
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .flat_map(|name| {
                let bare = imp::without_run_extension(&name).map(program_key);
                std::iter::once(program_key(&name)).chain(bare)
            })
            .collect();
        Self { path: path.to_owned(), names }
    }

    /// As [`find_program`], with a name on no listed directory refused at once.
    pub fn find(&self, program: &str) -> Option<PathBuf> {
        let named = !program.contains(['/', std::path::MAIN_SEPARATOR]);
        if named && !self.names.contains(&program_key(program)) {
            return None;
        }
        find_program(program, &self.path)
    }
}

/// A program's name as the system compares it: Windows ignores case.
fn program_key(name: &str) -> String {
    if cfg!(windows) { name.to_ascii_lowercase() } else { name.to_owned() }
}

/// Ends the process `pid` at once, as `kill -KILL` does.
pub fn kill(pid: u32) -> io::Result<()> {
    imp::kill(pid)
}

/// Stops every thread of the process `pid` until [`resume`], as `kill -STOP`
/// does. On Windows the calling thread holds the process as its debugger,
/// so `resume` must come from the same thread.
pub fn suspend(pid: u32) -> io::Result<()> {
    imp::suspend(pid)
}

pub fn resume(pid: u32) -> io::Result<()> {
    imp::resume(pid)
}

/// Makes `link` point at `target`. Where Windows refuses a symlink without
/// Developer Mode, a directory gets a junction and a file a copy.
pub fn link(target: &Path, link: &Path) -> io::Result<()> {
    imp::link(target, link)
}

/// The user's home: `$HOME`, else on Windows `%USERPROFILE%`, where the
/// agents keep `.claude`, `.codex` and the rest.
pub fn home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty());
    let home = if cfg!(windows) { home.or_else(|| std::env::var_os("USERPROFILE")) } else { home };
    home.filter(|home| !home.is_empty()).map(PathBuf::from)
}

/// Whether this process runs as the superuser.
pub fn is_root() -> bool {
    imp::is_root()
}

/// Whether file events can't be relied on under `root`, so it must be
/// polled: network shares everywhere, and on Linux 9p (WSL's `/mnt/c`) and
/// FUSE too (docs/research/probes.md).
pub fn needs_polling(root: &Path) -> bool {
    imp::needs_polling(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_found_on_the_path_given_and_directly() {
        let bin = tempfile::tempdir().expect("tempdir");
        let program = bin.path().join(if cfg!(windows) { "tool.exe" } else { "tool" });
        fs::write(&program, "#!/bin/sh\n").expect("write");
        make_executable(&program).expect("chmod");
        let found = find_program("tool", bin.path().as_os_str()).expect("found on the path");
        assert_eq!(found.file_stem(), Some(OsStr::new("tool")));
        assert!(find_program(&program.to_string_lossy(), OsStr::new("")).is_some(), "by its path");
        assert!(find_program("not-here", bin.path().as_os_str()).is_none());
        assert!(find_program("", bin.path().as_os_str()).is_none());
    }

    #[test]
    fn verbatim_paths_are_written_plainly_when_they_can_be() {
        let plain = |text: &str| simplified(PathBuf::from(text)).to_string_lossy().into_owned();
        assert_eq!(plain(r"\\?\C:\Users\me\repo"), r"C:\Users\me\repo");
        assert_eq!(plain(r"\\?\UNC\wsl.localhost\Ubuntu\home"), r"\\wsl.localhost\Ubuntu\home");
        assert_eq!(plain(r"\\?\C:\a\aux"), r"\\?\C:\a\aux", "a device name needs the verbatim form");
        assert_eq!(plain(r"\\?\C:\a\trailing."), r"\\?\C:\a\trailing.");
        assert_eq!(plain(&format!(r"\\?\C:\{}", "x".repeat(300))), format!(r"\\?\C:\{}", "x".repeat(300)));
        assert_eq!(plain("/home/me/repo"), "/home/me/repo");
        assert_eq!(plain(r"C:\already"), r"C:\already");
    }

    #[test]
    fn private_files_replace_what_was_there() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("token");
        write_private(&path, b"first, and longer").expect("writes");
        write_private(&path, b"second").expect("replaces");
        assert_eq!(fs::read(&path).expect("reads"), b"second");
    }

    #[test]
    fn links_reach_their_targets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("skills");
        fs::create_dir(&target).expect("mkdir");
        fs::write(target.join("SKILL.md"), "hi").expect("write");
        link(&target, &dir.path().join("linked")).expect("links");
        assert_eq!(fs::read_to_string(dir.path().join("linked").join("SKILL.md")).expect("reads"), "hi");
        link(&target.join("SKILL.md"), &dir.path().join("file")).expect("links a file");
        assert_eq!(fs::read_to_string(dir.path().join("file")).expect("reads"), "hi");
    }

    #[test]
    fn private_directories_are_made_and_traps_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let made = root.path().join("silt");
        make_private_dir(&made).expect("makes");
        assert!(made.is_dir());
        make_private_dir(&made).expect("again");
        let file = root.path().join("file");
        fs::write(&file, "").expect("write");
        assert!(make_private_dir(&file).is_err(), "a file is not a directory");
    }
}
