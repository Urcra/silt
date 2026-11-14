//! Fetching a model's files. Every byte is checked against a pinned hash and
//! size, a broken download resumes where it stopped, and a file appears
//! under its own name only once it's whole.

use std::path::{Path, PathBuf};

use reqwest::StatusCode;
use reqwest::header::RANGE;
use sha2::{Digest, Sha256};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::LlmError;

/// A file to fetch, as pinned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

/// The running check of a file's bytes against its pin.
#[derive(Debug, Clone)]
pub struct Check {
    hash: Sha256,
    seen: u64,
    size: u64,
}

impl Check {
    pub fn new(size: u64) -> Self {
        Self { hash: Sha256::new(), seen: 0, size }
    }

    pub fn seen(&self) -> u64 {
        self.seen
    }

    /// Takes the next bytes. False once past the pinned size, which a
    /// download stops at.
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        self.seen += bytes.len() as u64;
        if self.seen > self.size {
            return false;
        }
        self.hash.update(bytes);
        true
    }

    pub fn matches(self, sha256: &str) -> bool {
        let found: String = self.hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
        self.seen == self.size && found.eq_ignore_ascii_case(sha256)
    }
}

/// Where a file is kept while it's fetched.
pub fn part_path(to: &Path) -> PathBuf {
    let mut name = to.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    to.with_file_name(name)
}

/// Fetches `wanted` to `to`, resuming a `.part` left by an earlier try, and
/// reports how many of the file's bytes are there as they come.
pub async fn fetch(
    client: &reqwest::Client,
    wanted: &Wanted,
    to: &Path,
    on_done: &mut (impl FnMut(u64) + Send),
) -> Result<(), LlmError> {
    let name = to.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let corrupt = || LlmError::Corrupt(name.clone());
    let part = part_path(to);
    let mut check = Check::new(wanted.size);
    let had = fs::metadata(&part).await.map(|metadata| metadata.len()).unwrap_or(0);
    if had > wanted.size {
        fs::remove_file(&part).await.map_err(LlmError::Disk)?;
    } else if had > 0 {
        let mut file = File::open(&part).await.map_err(LlmError::Disk)?;
        let mut buffer = vec![0; 1 << 20];
        loop {
            let read = file.read(&mut buffer).await.map_err(LlmError::Disk)?;
            if read == 0 {
                break;
            }
            if !check.feed(&buffer[..read]) {
                return Err(corrupt());
            }
        }
        on_done(check.seen());
    }
    if check.seen() < wanted.size {
        let mut request = client.get(&wanted.url);
        if check.seen() > 0 {
            request = request.header(RANGE, format!("bytes={}-", check.seen()));
        }
        let mut response = request.send().await.map_err(|error| LlmError::Fetch(error.to_string()))?;
        let mut file = match response.status() {
            StatusCode::PARTIAL_CONTENT if check.seen() > 0 => {
                OpenOptions::new().append(true).open(&part).await.map_err(LlmError::Disk)?
            }
            status if status.is_success() => {
                check = Check::new(wanted.size);
                on_done(0);
                File::create(&part).await.map_err(LlmError::Disk)?
            }
            status => return Err(LlmError::Fetch(format!("{} answered {status}", wanted.url))),
        };
        while let Some(chunk) = response.chunk().await.map_err(|error| LlmError::Fetch(error.to_string()))? {
            if !check.feed(&chunk) {
                drop(file);
                let _ = fs::remove_file(&part).await;
                return Err(corrupt());
            }
            file.write_all(&chunk).await.map_err(LlmError::Disk)?;
            on_done(check.seen());
        }
        file.sync_all().await.map_err(LlmError::Disk)?;
    }
    if !check.matches(&wanted.sha256) {
        let _ = fs::remove_file(&part).await;
        return Err(corrupt());
    }
    fs::rename(&part, to).await.map_err(LlmError::Disk)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::http::{HeaderMap, StatusCode as HttpStatus};
    use axum::response::IntoResponse;
    use axum::routing::get;

    use super::*;

    fn sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Serves `body` at `/file`, honouring `Range` unless `ranges` is off,
    /// and records the ranges asked for.
    async fn serve(body: Vec<u8>, ranges: bool) -> (String, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = asked.clone();
        let app = Router::new().route(
            "/file",
            get(move |headers: HeaderMap| {
                let body = body.clone();
                let seen = seen.clone();
                async move {
                    let range = headers.get("range").and_then(|value| value.to_str().ok()).map(str::to_owned);
                    seen.lock().expect("lock").push(range.clone().unwrap_or_default());
                    let start = range
                        .filter(|_| ranges)
                        .and_then(|range| range.strip_prefix("bytes=")?.strip_suffix('-')?.parse::<usize>().ok());
                    match start {
                        Some(start) => (HttpStatus::PARTIAL_CONTENT, body[start..].to_vec()).into_response(),
                        None => (HttpStatus::OK, body).into_response(),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}/file", listener.local_addr().expect("address"));
        tokio::spawn(async move { axum::serve(listener, app).await });
        (url, asked)
    }

    #[tokio::test]
    async fn a_file_appears_only_once_whole_and_checked() {
        let body = vec![7u8; 300_000];
        let (url, _) = serve(body.clone(), true).await;
        let dir = tempfile::tempdir().expect("temp");
        let to = dir.path().join("model.gguf");
        let wanted = Wanted { url, sha256: sha256(&body), size: body.len() as u64 };
        let mut done = 0;
        fetch(&reqwest::Client::new(), &wanted, &to, &mut |bytes| done = bytes).await.expect("fetched");
        assert_eq!(std::fs::read(&to).expect("read"), body);
        assert_eq!(done, body.len() as u64);
        assert!(!part_path(&to).exists());
    }

    #[tokio::test]
    async fn a_broken_download_resumes_from_its_part() {
        let body: Vec<u8> = (0..200_000u32).map(|n| n as u8).collect();
        let (url, asked) = serve(body.clone(), true).await;
        let dir = tempfile::tempdir().expect("temp");
        let to = dir.path().join("model.gguf");
        std::fs::write(part_path(&to), &body[..50_000]).expect("part");
        let wanted = Wanted { url, sha256: sha256(&body), size: body.len() as u64 };
        let mut done = 0;
        fetch(&reqwest::Client::new(), &wanted, &to, &mut |bytes| done = bytes).await.expect("fetched");
        assert_eq!(std::fs::read(&to).expect("read"), body);
        assert_eq!(asked.lock().expect("lock").as_slice(), ["bytes=50000-"]);
        assert_eq!(done, body.len() as u64);
    }

    #[tokio::test]
    async fn a_server_ignoring_ranges_starts_the_file_again() {
        let body = vec![3u8; 100_000];
        let (url, _) = serve(body.clone(), false).await;
        let dir = tempfile::tempdir().expect("temp");
        let to = dir.path().join("model.gguf");
        std::fs::write(part_path(&to), &body[..10_000]).expect("part");
        let wanted = Wanted { url, sha256: sha256(&body), size: body.len() as u64 };
        let mut done = 0;
        fetch(&reqwest::Client::new(), &wanted, &to, &mut |bytes| done = bytes).await.expect("fetched");
        assert_eq!(std::fs::read(&to).expect("read"), body);
        assert_eq!(done, body.len() as u64);
    }

    #[tokio::test]
    async fn bytes_not_matching_the_pin_never_become_the_file() {
        let body = vec![1u8; 1000];
        let (url, _) = serve(body.clone(), true).await;
        let dir = tempfile::tempdir().expect("temp");
        let to = dir.path().join("model.gguf");
        let client = reqwest::Client::new();
        let wrong = Wanted { url: url.clone(), sha256: sha256(b"other"), size: 1000 };
        assert!(matches!(fetch(&client, &wrong, &to, &mut |_| ()).await, Err(LlmError::Corrupt(_))));
        let short = Wanted { url, sha256: sha256(&body), size: 999 };
        assert!(matches!(fetch(&client, &short, &to, &mut |_| ()).await, Err(LlmError::Corrupt(_))));
        assert!(!to.exists());
        assert!(!part_path(&to).exists());
    }
}
