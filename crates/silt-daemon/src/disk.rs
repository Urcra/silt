use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
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

/// Whether `path` (relative to the checkout) is Silt's or git's own data.
pub fn is_internal(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or(path);
    first == ".git" || first == ".silt"
}

/// Reads a regular file. Missing files and symlinks give `None`.
pub fn read(root: &Path, path: &str) -> io::Result<Option<RawFile>> {
    let full = root.join(path);
    let metadata = match fs::symlink_metadata(&full) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&full)?;
    Ok(Some(RawFile {
        hash: *blake3::hash(&bytes).as_bytes(),
        mtime_ns: mtime_ns(&metadata),
        executable: metadata.permissions().mode() & 0o111 != 0,
        bytes,
    }))
}

/// Writes atomically (temporary file, then rename), creating directories.
pub fn write(root: &Path, path: &str, bytes: &[u8], executable: bool) -> io::Result<RawFile> {
    let full = root.join(path);
    let dir = full.parent().unwrap_or(root);
    fs::create_dir_all(dir)?;
    let file_name = full.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = dir.join(format!(".{file_name}.silt-tmp"));
    fs::write(&temporary, bytes)?;
    let mut permissions = fs::metadata(&temporary)?.permissions();
    let mode = permissions.mode();
    permissions.set_mode(if executable { mode | 0o111 } else { mode & !0o111 });
    fs::set_permissions(&temporary, permissions)?;
    fs::rename(&temporary, &full)?;
    let metadata = fs::metadata(&full)?;
    Ok(RawFile {
        hash: *blake3::hash(bytes).as_bytes(),
        mtime_ns: mtime_ns(&metadata),
        executable,
        bytes: bytes.to_vec(),
    })
}

pub fn remove(root: &Path, path: &str) -> io::Result<()> {
    match fs::remove_file(root.join(path)) {
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
    }
}
