use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use silt_types::BlobHash;
use tokio::fs;

/// Content-addressed blobs on disk, named by their BLAKE3 hash.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
    /// Whether a blob is synced before it's renamed into place.
    durable: bool,
}

impl BlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), durable: true }
    }

    /// Blobs whose loss a reader recovers from, kept without syncing each:
    /// a family's files' states, whose family is replayed when one is
    /// missing or damaged (ADR 0116). A snapshot writes thousands at once.
    pub fn recoverable(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), durable: false }
    }

    pub fn hash(bytes: &[u8]) -> BlobHash {
        BlobHash(*blake3::hash(bytes).as_bytes())
    }

    /// Stores `bytes` durably: written to a file of this call's own, synced,
    /// then renamed into place, so concurrent stores of one blob and crashes
    /// never leave a partial blob behind.
    pub async fn put(&self, bytes: &[u8]) -> io::Result<BlobHash> {
        let hash = Self::hash(bytes);
        let path = self.path(hash);
        if fs::try_exists(&path).await? {
            // Fresh again: garbage collection spares recently stored blobs,
            // and a delta referring to this one may be on its way.
            let touched = path.clone();
            // Windows changes a file's times only through a handle that may write.
            tokio::task::spawn_blocking(move || {
                std::fs::File::options().write(true).open(touched)?.set_modified(SystemTime::now())
            })
            .await
            .map_err(io::Error::other)??;
            return Ok(hash);
        }
        let dir = path.parent().expect("blob paths have a parent");
        fs::create_dir_all(dir).await?;
        let unique = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_nanos());
        let temporary = dir.join(format!(".{hash}.{}-{unique}.tmp", std::process::id()));
        let mut file = fs::File::create(&temporary).await?;
        tokio::io::AsyncWriteExt::write_all(&mut file, bytes).await?;
        if self.durable {
            file.sync_all().await?;
        }
        drop(file);
        fs::rename(&temporary, &path).await?;
        if self.durable {
            let dir = dir.to_owned();
            tokio::task::spawn_blocking(move || silt_os::sync_dir(&dir)).await.map_err(io::Error::other)??;
        }
        Ok(hash)
    }

    /// The blob's bytes. One damaged on disk counts as missing, so it gets
    /// stored again rather than handed out.
    pub async fn get(&self, hash: BlobHash) -> io::Result<Vec<u8>> {
        let bytes = fs::read(self.path(hash)).await?;
        if Self::hash(&bytes) == hash {
            Ok(bytes)
        } else {
            let _ = fs::remove_file(self.path(hash)).await;
            Err(io::Error::new(io::ErrorKind::NotFound, format!("blob {hash} was damaged")))
        }
    }

    /// [`Self::get`] for a caller that can't wait on the runtime, such as a
    /// family reading a file's state when first used (ADR 0116).
    pub fn read(&self, hash: BlobHash) -> io::Result<Vec<u8>> {
        let bytes = std::fs::read(self.path(hash))?;
        if Self::hash(&bytes) == hash {
            Ok(bytes)
        } else {
            let _ = std::fs::remove_file(self.path(hash));
            Err(io::Error::new(io::ErrorKind::NotFound, format!("blob {hash} was damaged")))
        }
    }

    pub fn contains(&self, hash: BlobHash) -> bool {
        self.path(hash).is_file()
    }

    /// Where a stored blob is, for a reader that streams it or a range of
    /// it rather than holding it whole; `None` when it isn't stored.
    pub fn file(&self, hash: BlobHash) -> Option<PathBuf> {
        let path = self.path(hash);
        path.is_file().then_some(path)
    }

    /// Stores the file at `source` without holding it in memory, hashing it
    /// as it's read. `take` moves the file in where it can, as for an upload
    /// already written to a file of its own.
    pub async fn put_file(&self, source: &Path, take: bool) -> io::Result<(BlobHash, u64)> {
        let (source, root, durable) = (source.to_owned(), self.root.clone(), self.durable);
        tokio::task::spawn_blocking(move || put_file(&root, durable, &source, take)).await.map_err(io::Error::other)?
    }

    /// How much of blob `hash` an upload by `owner` has sent so far.
    pub async fn received(&self, owner: &str, hash: BlobHash) -> io::Result<u64> {
        let path = self.partial(owner, hash)?;
        match fs::metadata(&path).await {
            Ok(meta) => Ok(meta.len()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(error),
        }
    }

    /// The most of blob `hash` any upload of it has sent, to say how much of
    /// it has arrived.
    pub async fn received_any(&self, hash: BlobHash) -> io::Result<u64> {
        let (root, name) = (self.root.join(PARTIAL), hash.to_string());
        tokio::task::spawn_blocking(move || {
            let Ok(owners) = std::fs::read_dir(root) else {
                return Ok(0);
            };
            let mut most = 0;
            for owner in owners.flatten() {
                if let Ok(meta) = std::fs::metadata(owner.path().join(&name)) {
                    most = most.max(meta.len());
                }
            }
            Ok(most)
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Adds a chunk of blob `hash`, `size` bytes in all, to `owner`'s upload
    /// of it at `offset`, and once it's whole and its hash checks out,
    /// stores it (ADR 0120). Chunks arrive in order: one at any other
    /// offset is refused with what has been received.
    pub async fn append(&self, owner: &str, hash: BlobHash, offset: u64, chunk: &[u8], size: u64) -> Result<Chunked, ChunkError> {
        if self.contains(hash) {
            return Ok(Chunked::Stored);
        }
        let end = offset.checked_add(chunk.len() as u64).ok_or(ChunkError::Exceeds)?;
        if end > size {
            return Err(ChunkError::Exceeds);
        }
        let path = self.partial(owner, hash)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).await?;
        }
        let received = self.received(owner, hash).await?;
        if received != offset {
            return Err(ChunkError::Offset { received });
        }
        let mut file = fs::OpenOptions::new().create(true).append(true).open(&path).await?;
        tokio::io::AsyncWriteExt::write_all(&mut file, chunk).await?;
        if self.durable {
            file.sync_data().await?;
        }
        drop(file);
        if end < size {
            return Ok(Chunked::Received(end));
        }
        let stored = self.put_file(&path, true).await;
        let _ = fs::remove_file(&path).await;
        match stored {
            Ok((stored, _)) if stored == hash => Ok(Chunked::Stored),
            Ok((stored, _)) => {
                self.remove(stored).await?;
                Err(ChunkError::Damaged)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Drops uploads left unfinished since before `before`.
    pub async fn drop_stale_uploads(&self, before: SystemTime) -> io::Result<usize> {
        let root = self.root.join(PARTIAL);
        tokio::task::spawn_blocking(move || {
            let mut dropped = 0;
            let Ok(owners) = std::fs::read_dir(&root) else {
                return Ok(0);
            };
            for owner in owners.flatten() {
                for upload in std::fs::read_dir(owner.path())?.flatten() {
                    if upload.metadata()?.modified()? < before {
                        std::fs::remove_file(upload.path())?;
                        dropped += 1;
                    }
                }
            }
            Ok(dropped)
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Notes that blob `hash` waits to be uploaded for `family`, after the
    /// deltas naming it (ADR 0120); kept on disk, so a restart goes on.
    pub async fn defer_upload(&self, hash: BlobHash, family: &str) -> io::Result<()> {
        let dir = self.root.join(DEFERRED);
        fs::create_dir_all(&dir).await?;
        fs::write(dir.join(hash.to_string()), family).await
    }

    /// The blobs waiting to be uploaded, each with its family; a few small
    /// files, read where a sync loop can't wait.
    pub fn deferred_uploads(&self) -> io::Result<Vec<(BlobHash, String)>> {
        let Ok(entries) = std::fs::read_dir(self.root.join(DEFERRED)) else {
            return Ok(Vec::new());
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            if let Ok(hash) = entry.file_name().to_string_lossy().parse::<BlobHash>() {
                found.push((hash, std::fs::read_to_string(entry.path())?.trim().to_owned()));
            }
        }
        Ok(found)
    }

    pub async fn uploaded(&self, hash: BlobHash) -> io::Result<()> {
        match fs::remove_file(self.root.join(DEFERRED).join(hash.to_string())).await {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    fn partial(&self, owner: &str, hash: BlobHash) -> io::Result<PathBuf> {
        if owner.is_empty() || !owner.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "an upload's owner is letters, digits, - and _"));
        }
        Ok(self.root.join(PARTIAL).join(owner).join(hash.to_string()))
    }

    /// Every stored blob and when it was written, listed on one blocking
    /// thread rather than one for each blob.
    pub async fn list(&self) -> io::Result<Vec<(BlobHash, SystemTime)>> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || list_blobs(&root)).await.map_err(io::Error::other)?
    }

    pub async fn remove(&self, hash: BlobHash) -> io::Result<()> {
        match fs::remove_file(self.path(hash)).await {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    fn path(&self, hash: BlobHash) -> PathBuf {
        let hex = hash.to_string();
        Path::new(&self.root).join(&hex[..2]).join(&hex[2..])
    }
}

/// Uploads under way, by owner; named so [`list_blobs`] passes them by.
const PARTIAL: &str = ".partial";
/// Blobs waiting to be uploaded after their deltas.
const DEFERRED: &str = ".deferred";

/// How a chunk of an upload went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chunked {
    /// So much of the blob has arrived.
    Received(u64),
    /// The blob is whole and stored.
    Stored,
}

#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("the upload has {received} bytes so far")]
    Offset { received: u64 },
    #[error("the chunk runs past the blob's size")]
    Exceeds,
    #[error("the blob's content doesn't match its hash")]
    Damaged,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The file's hash and length, read a block at a time.
pub fn hash_file(path: &Path) -> io::Result<(BlobHash, u64)> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(std::fs::File::open(path)?)?;
    let size = std::fs::metadata(path)?.len();
    Ok((BlobHash(*hasher.finalize().as_bytes()), size))
}

fn put_file(root: &Path, durable: bool, source: &Path, take: bool) -> io::Result<(BlobHash, u64)> {
    let (hash, size) = hash_file(source)?;
    let hex = hash.to_string();
    let dir = root.join(&hex[..2]);
    let path = dir.join(&hex[2..]);
    if path.is_file() {
        std::fs::File::options().write(true).open(&path)?.set_modified(SystemTime::now())?;
        if take {
            std::fs::remove_file(source)?;
        }
        return Ok((hash, size));
    }
    std::fs::create_dir_all(&dir)?;
    let unique = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_nanos());
    let temporary = dir.join(format!(".{hash}.{}-{unique}.tmp", std::process::id()));
    // A move keeps a large file from being written twice; across file
    // systems it can't, so it's copied.
    let moved = take && std::fs::rename(source, &temporary).is_ok();
    if !moved {
        std::fs::copy(source, &temporary)?;
    }
    if durable {
        std::fs::File::open(&temporary)?.sync_all()?;
    }
    // A copy of a file changed while it was read: what's kept must be what was hashed.
    if !moved && hash_file(&temporary)?.0 != hash {
        std::fs::remove_file(&temporary)?;
        return Err(io::Error::other("the file changed while it was stored"));
    }
    std::fs::rename(&temporary, &path)?;
    if take && !moved {
        std::fs::remove_file(source)?;
    }
    if durable {
        silt_os::sync_dir(&dir)?;
    }
    Ok((hash, size))
}

fn list_blobs(root: &Path) -> io::Result<Vec<(BlobHash, SystemTime)>> {
    let mut found = Vec::new();
    let Ok(dirs) = std::fs::read_dir(root) else {
        return Ok(found);
    };
    for dir in dirs {
        let dir = dir?;
        let prefix = dir.file_name().to_string_lossy().into_owned();
        if prefix.len() != 2 || !dir.file_type()?.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(dir.path())? {
            let file = file?;
            let name = format!("{prefix}{}", file.file_name().to_string_lossy());
            if let Ok(hash) = name.parse::<BlobHash>() {
                found.push((hash, file.metadata()?.modified()?));
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_file_is_stored_by_its_hash_and_taken_when_asked() {
        let dir = tempfile::tempdir().expect("dir");
        let blobs = BlobStore::new(dir.path().join("blobs"));
        let source = dir.path().join("clip.webm");
        std::fs::write(&source, b"\x1a\x45\xdf\xa3 a clip").expect("write");
        let (hash, size) = blobs.put_file(&source, false).await.expect("put");
        assert_eq!((hash, size), (BlobStore::hash(b"\x1a\x45\xdf\xa3 a clip"), 11));
        assert!(source.exists());
        assert_eq!(blobs.put_file(&source, true).await.expect("again").0, hash);
        assert!(!source.exists());
        assert_eq!(blobs.get(hash).await.expect("get"), b"\x1a\x45\xdf\xa3 a clip");
        assert!(blobs.file(hash).is_some());
    }

    #[tokio::test]
    async fn an_upload_in_chunks_resumes_where_it_stopped_and_is_checked_whole() {
        let dir = tempfile::tempdir().expect("dir");
        let blobs = BlobStore::new(dir.path());
        let bytes: Vec<u8> = (0..10_000u32).map(|n| (n % 251) as u8).collect();
        let hash = BlobStore::hash(&bytes);
        let size = bytes.len() as u64;
        assert_eq!(blobs.append("7", hash, 0, &bytes[..4000], size).await.expect("first"), Chunked::Received(4000));
        assert!(matches!(blobs.append("7", hash, 0, &bytes[..4000], size).await, Err(ChunkError::Offset { received: 4000 })));
        assert_eq!(blobs.received("7", hash).await.expect("received"), 4000);
        assert_eq!(blobs.received("8", hash).await.expect("another's"), 0);
        assert_eq!(blobs.received_any(hash).await.expect("any"), 4000);
        assert!(matches!(blobs.append("7", hash, 4000, &bytes[4000..], size - 1).await, Err(ChunkError::Exceeds)));
        assert_eq!(blobs.append("7", hash, 4000, &bytes[4000..], size).await.expect("last"), Chunked::Stored);
        assert_eq!(blobs.get(hash).await.expect("whole"), bytes);
        assert_eq!(blobs.received("7", hash).await.expect("done"), 0);
        assert_eq!(blobs.list().await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn an_upload_that_doesnt_match_its_hash_is_dropped() {
        let dir = tempfile::tempdir().expect("dir");
        let blobs = BlobStore::new(dir.path());
        let hash = BlobStore::hash(b"what was promised");
        assert!(matches!(blobs.append("7", hash, 0, b"something else", 14).await, Err(ChunkError::Damaged)));
        assert!(!blobs.contains(hash));
        assert!(blobs.list().await.expect("list").is_empty());
        assert!(blobs.append("../x", hash, 0, b"a", 1).await.is_err());
    }

    #[tokio::test]
    async fn deferred_uploads_are_kept_until_done() {
        let dir = tempfile::tempdir().expect("dir");
        let blobs = BlobStore::new(dir.path());
        let hash = blobs.put(b"a long video").await.expect("put");
        blobs.defer_upload(hash, "family-1").await.expect("defer");
        assert_eq!(blobs.deferred_uploads().expect("list"), vec![(hash, "family-1".to_owned())]);
        assert_eq!(blobs.list().await.expect("blobs").len(), 1);
        blobs.uploaded(hash).await.expect("done");
        assert!(blobs.deferred_uploads().expect("list").is_empty());
    }
}
