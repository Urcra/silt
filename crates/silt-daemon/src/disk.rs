use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Files larger than this are stored whole instead of as text.
const TEXT_LIMIT: usize = 4 * 1024 * 1024;

/// A file's bytes and metadata as found on disk.
#[derive(Debug, Clone)]
pub struct RawFile {
    pub bytes: Vec<u8>,
    pub hash: [u8; 32],
    pub mtime_ns: i64,
    pub executable: bool,
}

impl RawFile {
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Whether the content is text Silt can merge character by character.
    pub fn is_text(&self) -> bool {
        self.bytes.len() <= TEXT_LIMIT && !self.bytes.contains(&0) && std::str::from_utf8(&self.bytes).is_ok()
    }
}

/// Whether `path` (relative to the checkout) is Silt's or git's own data,
/// or a path Silt never writes.
pub fn is_internal(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or(path);
    first == ".silt" || !silt_checkout::is_writable(path)
}

/// The full path of `path` in the checkout at `root`, refusing paths Silt
/// must not touch and paths that pass through a symlinked directory, which
/// could lead outside the checkout. With `create`, missing directories are
/// made on the way.
fn inside(root: &Path, path: &str, create: bool) -> io::Result<PathBuf> {
    if !silt_checkout::is_writable(path) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{path} is not a path Silt writes")));
    }
    let mut full = root.to_owned();
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        full.push(component);
        if components.peek().is_none() {
            break;
        }
        match fs::symlink_metadata(&full) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{path} goes through {}, which is not a directory", full.display()),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => fs::create_dir(&full)?,
            Err(error) => return Err(error),
        }
    }
    Ok(full)
}

/// Reads a regular file. Missing files, symlinks and paths Silt doesn't
/// write give `None`.
pub fn read(root: &Path, path: &str) -> io::Result<Option<RawFile>> {
    let full = match inside(root, path, false) {
        Ok(full) => full,
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied) => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = match fs::symlink_metadata(&full) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    let bytes = match fs::read(&full) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(RawFile {
        hash: *blake3::hash(&bytes).as_bytes(),
        mtime_ns: mtime_ns(&metadata),
        executable: metadata.permissions().mode() & 0o111 != 0,
        bytes,
    }))
}

/// Writes the file atomically and durably (a synced temporary file, renamed
/// into place), creating directories, unless, checked right before the new
/// content takes its place, `unchanged` says someone changed it meanwhile
/// (it gets the current content's hash, `None` if there is no file).
/// Returns `None` when it left the file alone. A file keeps its permission
/// bits; only whether it is executable follows the thread.
pub fn replace(
    root: &Path,
    path: &str,
    bytes: &[u8],
    executable: bool,
    unchanged: impl FnOnce(Option<[u8; 32]>) -> bool,
) -> io::Result<Option<RawFile>> {
    let full = inside(root, path, true)?;
    let dir = full.parent().unwrap_or(root);
    let file_name = full.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = dir.join(format!(".{file_name}.silt-tmp"));
    let mut file = fs::File::create(&temporary)?;
    io::Write::write_all(&mut file, bytes)?;
    let mode = match fs::metadata(&full) {
        Ok(metadata) => metadata.permissions().mode(),
        Err(_) => file.metadata()?.permissions().mode(),
    };
    // Executable where readable, as git checks out an executable file.
    let mode = if executable { mode | ((mode & 0o444) >> 2) } else { mode & !0o111 };
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.sync_all()?;
    drop(file);
    let current = match fs::read(&full) {
        Ok(current) => Some(*blake3::hash(&current).as_bytes()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if !unchanged(current) {
        fs::remove_file(&temporary)?;
        return Ok(None);
    }
    fs::rename(&temporary, &full)?;
    fs::File::open(dir)?.sync_all()?;
    let metadata = fs::metadata(&full)?;
    Ok(Some(RawFile {
        hash: *blake3::hash(bytes).as_bytes(),
        mtime_ns: mtime_ns(&metadata),
        executable,
        bytes: bytes.to_vec(),
    }))
}

pub fn remove(root: &Path, path: &str) -> io::Result<()> {
    let full = match inside(root, path, false) {
        Ok(full) => full,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    match fs::remove_file(full) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn mtime_ns(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, path: &str, bytes: &[u8], executable: bool) -> io::Result<RawFile> {
        Ok(replace(root, path, bytes, executable, |_| true)?.expect("an unconditional write happens"))
    }

    #[test]
    fn a_rewritten_file_keeps_its_permissions() {
        let dir = tempfile::tempdir().expect("temp dir");
        write(dir.path(), "secret.env", b"a\n", false).expect("writes");
        let path = dir.path().join("secret.env");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("chmod");
        write(dir.path(), "secret.env", b"b\n", false).expect("rewrites");
        assert_eq!(fs::metadata(&path).expect("stat").permissions().mode() & 0o777, 0o600);
        write(dir.path(), "secret.env", b"c\n", true).expect("makes executable");
        assert_eq!(fs::metadata(&path).expect("stat").permissions().mode() & 0o777, 0o700, "executable only where readable");
    }

    #[test]
    fn a_file_changed_meanwhile_is_left_alone() {
        let dir = tempfile::tempdir().expect("temp dir");
        let first = write(dir.path(), "a.txt", b"recorded\n", false).expect("writes");
        std::fs::write(dir.path().join("a.txt"), "edited meanwhile\n").expect("edits");
        let replaced = replace(dir.path(), "a.txt", b"from the thread\n", false, |current| current == Some(first.hash)).expect("checks");
        assert!(replaced.is_none());
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).expect("reads"), "edited meanwhile\n");
        assert!(!dir.path().join(".a.txt.silt-tmp").exists(), "the new content is thrown away");
    }

    #[test]
    fn write_then_read_round_trips_with_mode() {
        let dir = tempfile::tempdir().expect("temp dir");
        let written = write(dir.path(), "a/b/run.sh", b"#!/bin/sh\n", true).expect("writes");
        let read_back = read(dir.path(), "a/b/run.sh").expect("reads").expect("exists");
        assert_eq!(read_back.hash, written.hash);
        assert!(read_back.executable && read_back.is_text());
        assert!(read(dir.path(), "missing").expect("reads").is_none());
        remove(dir.path(), "a/b/run.sh").expect("removes");
        remove(dir.path(), "a/b/run.sh").expect("removing twice is fine");
    }

    #[test]
    fn binary_and_internal_files_are_recognised() {
        let dir = tempfile::tempdir().expect("temp dir");
        write(dir.path(), "img.bin", &[1, 0, 2], false).expect("writes");
        assert!(!read(dir.path(), "img.bin").expect("reads").expect("exists").is_text());
        assert!(is_internal(".git/HEAD") && is_internal(".silt/checkouts/x/a") && !is_internal("src/.gitignore"));
        assert!(is_internal("vendor/.GIT/config"));
    }

    #[test]
    fn never_writes_into_git_data_or_through_symlinks() {
        let dir = tempfile::tempdir().expect("temp dir");
        let outside = tempfile::tempdir().expect("temp dir");
        assert!(write(dir.path(), ".git/hooks/pre-commit", b"x", true).is_err());
        assert!(write(dir.path(), "a/../../escape", b"x", false).is_err());
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).expect("symlink");
        assert!(write(dir.path(), "linked/planted", b"x", false).is_err());
        assert!(!outside.path().join("planted").exists());
        std::fs::write(outside.path().join("secret"), "s").expect("writes");
        assert!(read(dir.path(), "linked/secret").expect("reads").is_none());
        remove(dir.path(), "linked/secret").expect_err("refuses");
        assert!(outside.path().join("secret").exists());
    }
}
