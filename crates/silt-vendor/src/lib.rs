//! The CDN files artifacts load, fetched once and kept on disk, for a frame
//! that loads them through Silt rather than from the CDNs (ADR 0071). Only
//! pinned files of the allowlist are fetched: `silt_design::frame::vendorable`.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The largest file kept.
const FILE_BYTES: usize = 20 << 20;
/// What the cache may hold in all.
const CACHE_BYTES: u64 = 512 << 20;
/// Google Fonts serves woff2 only to browsers that say they take it.
const BROWSER: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

#[derive(Debug, thiserror::Error)]
pub enum VendorError {
    #[error("{0}")]
    Refused(String),
    #[error("the CDN answered {0}")]
    Status(u16),
    #[error("could not reach the CDN: {0}")]
    Fetch(#[from] reqwest::Error),
    #[error("could not keep the file: {0}")]
    Disk(#[from] std::io::Error),
}

/// A file as the vendor route serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

pub struct Vendor {
    dir: PathBuf,
    client: reqwest::Client,
}

impl Vendor {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.to_owned(), client: reqwest::Client::new() }
    }

    /// The file at `host` and `path`, from the cache or else the CDN.
    pub async fn get(&self, host: &str, path: &str, query: Option<&str>) -> Result<Fetched, VendorError> {
        silt_design::frame::vendorable(host, path, query).map_err(VendorError::Refused)?;
        let url = match query {
            Some(query) => format!("https://{host}{path}?{query}"),
            None => format!("https://{host}{path}"),
        };
        let key: String = Sha256::digest(url.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
        let (body, kind) = (self.dir.join(format!("{key}.bin")), self.dir.join(format!("{key}.type")));
        if let (Ok(bytes), Ok(content_type)) = (tokio::fs::read(&body).await, tokio::fs::read_to_string(&kind).await) {
            return Ok(Fetched { content_type, bytes });
        }
        let response = self.client.get(&url).header(reqwest::header::USER_AGENT, BROWSER).send().await?;
        if !response.status().is_success() {
            return Err(VendorError::Status(response.status().as_u16()));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.bytes().all(|b| b.is_ascii_graphic() || b == b' '))
            .unwrap_or("application/octet-stream")
            .to_owned();
        let bytes = response.bytes().await?;
        if bytes.len() > FILE_BYTES {
            return Err(VendorError::Refused(format!("{url} is larger than {} MiB", FILE_BYTES >> 20)));
        }
        let bytes = if host == "fonts.googleapis.com" {
            silt_design::frame::fonts_css_through_vendor(&String::from_utf8_lossy(&bytes)).into_bytes()
        } else {
            bytes.to_vec()
        };
        if self.held().await + bytes.len() as u64 <= CACHE_BYTES {
            tokio::fs::create_dir_all(&self.dir).await?;
            let staging = self.dir.join(format!("{key}.tmp"));
            tokio::fs::write(&staging, &bytes).await?;
            tokio::fs::write(&kind, &content_type).await?;
            tokio::fs::rename(&staging, &body).await?;
        }
        Ok(Fetched { content_type, bytes })
    }

    async fn held(&self) -> u64 {
        let Ok(mut entries) = tokio::fs::read_dir(&self.dir).await else {
            return 0;
        };
        let mut total = 0;
        while let Ok(Some(entry)) = entries.next_entry().await {
            total += entry.metadata().await.map_or(0, |meta| meta.len());
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_kept_file_is_served_without_the_cdn_and_others_are_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let vendor = Vendor::new(dir.path());
        let url = "https://cdnjs.cloudflare.com/ajax/libs/d3/7.9.0/d3.min.js";
        let key: String = Sha256::digest(url.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
        std::fs::write(dir.path().join(format!("{key}.bin")), b"window.d3={}").expect("write");
        std::fs::write(dir.path().join(format!("{key}.type")), "text/javascript").expect("write");
        let kept = vendor.get("cdnjs.cloudflare.com", "/ajax/libs/d3/7.9.0/d3.min.js", None).await.expect("kept");
        assert_eq!((kept.content_type.as_str(), kept.bytes.as_slice()), ("text/javascript", b"window.d3={}".as_slice()));
        assert!(matches!(vendor.get("esm.sh", "/react@18.3.1", None).await, Err(VendorError::Refused(_))));
        assert!(matches!(vendor.get("unpkg.com", "/react@latest/index.js", None).await, Err(VendorError::Refused(_))));
    }
}
