use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use silt_types::BlobHash;
use tokio::fs;

/// Content-addressed blobs on disk, named by their BLAKE3 hash.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
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
        file.sync_all().await?;
        drop(file);
        fs::rename(&temporary, &path).await?;
        let dir = dir.to_owned();
        tokio::task::spawn_blocking(move || silt_os::sync_dir(&dir)).await.map_err(io::Error::other)??;
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

    /// Every stored blob and when it was written.
    pub async fn list(&self) -> io::Result<Vec<(BlobHash, SystemTime)>> {
        let mut found = Vec::new();
        let Ok(mut dirs) = fs::read_dir(&self.root).await else {
            return Ok(found);
        };
        while let Some(dir) = dirs.next_entry().await? {
            let prefix = dir.file_name().to_string_lossy().into_owned();
            if prefix.len() != 2 || !dir.file_type().await?.is_dir() {
                continue;
            }
            let mut files = fs::read_dir(dir.path()).await?;
            while let Some(file) = files.next_entry().await? {
                let name = format!("{prefix}{}", file.file_name().to_string_lossy());
                if let Ok(hash) = name.parse::<BlobHash>() {
                    found.push((hash, file.metadata().await?.modified()?));
                }
            }
        }
        Ok(found)
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
