//! Asking Hugging Face what a model's files are: their sizes, and the
//! SHA-256 it keeps for each, which pins the download.

use serde::Deserialize;

use crate::{LlmError, Remote, checked_file, split_part};

/// A file of a model, pinned.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct PinnedFile {
    /// Relative to the repository and to the model's directory.
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// One entry of the Hub's file listing.
#[derive(Debug, Clone, Deserialize)]
pub struct Listed {
    pub path: String,
    #[serde(default)]
    pub lfs: Option<Lfs>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Lfs {
    pub oid: String,
    pub size: u64,
}

/// The files `file` needs: itself, or every part of a split model in order.
pub fn select(listing: &[Listed], file: &str) -> Result<Vec<PinnedFile>, LlmError> {
    let missing = || LlmError::NotOnHub(file.to_owned());
    let (dir, name) = file.rsplit_once('/').map_or(("", file), |(dir, name)| (dir, name));
    let stem = name.strip_suffix(".gguf").ok_or_else(missing)?;
    let wanted: Vec<String> = match split_part(stem) {
        Some(part) => (1..=part.count)
            .map(|index| {
                let name = format!("{}-{index:05}-of-{:05}.gguf", part.base, part.count);
                if dir.is_empty() { name } else { format!("{dir}/{name}") }
            })
            .collect(),
        None => vec![file.to_owned()],
    };
    wanted
        .iter()
        .map(|path| {
            let listed = listing.iter().find(|listed| listed.path == *path).ok_or_else(missing)?;
            let lfs = listed.lfs.as_ref().ok_or_else(|| LlmError::NotPinned(path.clone()))?;
            let plain = lfs.oid.len() == 64 && lfs.oid.bytes().all(|byte| byte.is_ascii_hexdigit());
            if !plain {
                return Err(LlmError::NotPinned(path.clone()));
            }
            Ok(PinnedFile { path: checked_file(path)?.to_owned(), sha256: lfs.oid.to_ascii_lowercase(), size: lfs.size })
        })
        .collect()
}

/// Hugging Face, or what stands in for it (`SILT_HUGGINGFACE_URL`, in tests).
#[derive(Debug, Clone)]
pub struct Hub {
    base: String,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new("https://huggingface.co")
    }
}

impl Hub {
    pub fn new(base: impl Into<String>) -> Self {
        Self { base: base.into().trim_end_matches('/').to_owned() }
    }

    pub fn file_url(&self, remote: &Remote, path: &str) -> String {
        format!("{}/{}/resolve/main/{path}", self.base, remote.repo)
    }

    /// The pinned files `remote` needs.
    pub async fn files(&self, client: &reqwest::Client, remote: &Remote) -> Result<Vec<PinnedFile>, LlmError> {
        let url = match remote.file.rsplit_once('/') {
            Some((dir, _)) => format!("{}/api/models/{}/tree/main/{dir}", self.base, remote.repo),
            None => format!("{}/api/models/{}/tree/main", self.base, remote.repo),
        };
        let response = client.get(&url).send().await.map_err(|error| LlmError::Fetch(error.to_string()))?;
        if !response.status().is_success() {
            return Err(LlmError::Fetch(format!("Hugging Face answered {} for {}", response.status(), remote.repo)));
        }
        let listing: Vec<Listed> = response.json().await.map_err(|error| LlmError::Fetch(error.to_string()))?;
        select(&listing, &remote.file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(path: &str, size: u64, oid: Option<&str>) -> Listed {
        Listed { path: path.to_owned(), lfs: oid.map(|oid| Lfs { oid: oid.to_owned(), size }) }
    }

    const OID: &str = "fadc3e5f8d42bf7e894a785b05082e47daee4df26680389817e2093056f088ad";

    #[test]
    fn a_single_file_is_pinned_by_its_lfs_hash() {
        let listing = [listed("README.md", 10, None), listed("m-Q4.gguf", 1234, Some(OID))];
        let files = select(&listing, "m-Q4.gguf").expect("selected");
        assert_eq!(files, [PinnedFile { path: "m-Q4.gguf".into(), sha256: OID.into(), size: 1234 }]);
    }

    #[test]
    fn a_split_model_brings_every_part_in_order() {
        let listing = [listed("BF16/m-00002-of-00002.gguf", 20, Some(OID)), listed("BF16/m-00001-of-00002.gguf", 10, Some(OID))];
        let files = select(&listing, "BF16/m-00002-of-00002.gguf").expect("selected");
        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["BF16/m-00001-of-00002.gguf", "BF16/m-00002-of-00002.gguf"]);
        assert!(matches!(select(&listing[..1], "BF16/m-00001-of-00002.gguf"), Err(LlmError::NotOnHub(_))));
    }

    #[test]
    fn a_file_without_an_lfs_hash_isnt_downloaded() {
        let listing = [listed("m.gguf", 10, None), listed("bad.gguf", 10, Some("not-a-hash"))];
        assert!(matches!(select(&listing, "m.gguf"), Err(LlmError::NotPinned(_))));
        assert!(matches!(select(&listing, "bad.gguf"), Err(LlmError::NotPinned(_))));
        assert!(matches!(select(&listing, "gone.gguf"), Err(LlmError::NotOnHub(_))));
    }
}
