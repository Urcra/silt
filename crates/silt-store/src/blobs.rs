use std::io;
use std::path::{Path, PathBuf};

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

    pub async fn put(&self, bytes: &[u8]) -> io::Result<BlobHash> {
        let hash = Self::hash(bytes);
        let path = self.path(hash);
        if fs::try_exists(&path).await? {
            return Ok(hash);
        }
        let dir = path.parent().expect("blob paths have a parent");
        fs::create_dir_all(dir).await?;
        let temporary = dir.join(format!(".{hash}.tmp"));
        fs::write(&temporary, bytes).await?;
        fs::rename(&temporary, &path).await?;
        Ok(hash)
    }

    pub async fn get(&self, hash: BlobHash) -> io::Result<Vec<u8>> {
        fs::read(self.path(hash)).await
    }

    fn path(&self, hash: BlobHash) -> PathBuf {
        let hex = hash.to_string();
        Path::new(&self.root).join(&hex[..2]).join(&hex[2..])
    }
}
