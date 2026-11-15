//! llama.cpp's own build of `llama-server`, for a machine without one
//! (ADR 0072): its latest release from GitHub, fetched only when the person
//! asks, checked against the SHA-256 GitHub publishes for it, and unpacked
//! whole, as it ships its libraries beside the program.

use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::LlmError;
use crate::download::{self, Wanted};

/// Where llama.cpp publishes its releases.
pub const RELEASES: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest";
const PROGRAM: &str = "llama-server";
/// How deep in an archive the program may be.
const MOST_DEPTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub tag: String,
    pub assets: Vec<Asset>,
}

fn plain_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 64
        && tag.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// Reads GitHub's answer for a release; an asset without a SHA-256 is left
/// out, as nothing could check it.
pub fn parse_release(answer: &Value) -> Result<Release, LlmError> {
    let tag = answer["tag_name"]
        .as_str()
        .filter(|tag| plain_tag(tag))
        .ok_or_else(|| LlmError::Fetch("GitHub's release has no tag".into()))?;
    let assets = answer["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let sha256 = asset["digest"].as_str()?.strip_prefix("sha256:")?;
            (sha256.len() == 64 && sha256.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| Asset {
                name: asset["name"].as_str().unwrap_or_default().to_owned(),
                url: asset["browser_download_url"].as_str().unwrap_or_default().to_owned(),
                size: asset["size"].as_u64().unwrap_or(0),
                sha256: sha256.to_ascii_lowercase(),
            })
        })
        // Only over HTTPS, but for a stand-in on loopback, which tests serve.
        .filter(|asset| (asset.url.starts_with("https://") || asset.url.starts_with("http://127.0.0.1:")) && asset.size > 0)
        .collect();
    Ok(Release { tag: tag.to_owned(), assets })
}

/// The build for this machine: Metal on Apple silicon, Vulkan on Linux,
/// which serves NVIDIA and AMD alike. `None` where llama.cpp has none Silt
/// takes, such as Windows, whose builds need their own runtime.
pub fn choose_asset<'a>(release: &'a Release, os: &str, arch: &str) -> Option<&'a Asset> {
    let wanted: &[&str] = match (os, arch) {
        ("macos", "aarch64") => &["bin-macos-arm64"],
        ("macos", "x86_64") => &["bin-macos-x64"],
        ("linux", "x86_64") => &["bin-ubuntu-vulkan-x64", "bin-ubuntu-x64"],
        ("linux", "aarch64") => &["bin-ubuntu-vulkan-arm64", "bin-ubuntu-arm64"],
        _ => return None,
    };
    wanted.iter().find_map(|kind| {
        [".tar.gz", ".zip"].iter().find_map(|extension| {
            let name = format!("llama-{}-{kind}{extension}", release.tag);
            release.assets.iter().find(|asset| asset.name == name)
        })
    })
}

/// A relative path of plain names, so it stays where it's unpacked.
fn stays_inside(path: &Path) -> bool {
    path.components().all(|part| matches!(part, Component::Normal(_) | Component::CurDir)) && path.components().next().is_some()
}

fn unpack_tar(archive: &Path, into: &Path) -> Result<(), LlmError> {
    let reader = flate2::read::GzDecoder::new(File::open(archive).map_err(LlmError::Disk)?);
    let mut tar = tar::Archive::new(reader);
    for entry in tar.entries().map_err(LlmError::Disk)? {
        let mut entry = entry.map_err(LlmError::Disk)?;
        let path = entry.path().map_err(LlmError::Disk)?.into_owned();
        let link_ok = entry.link_name().map_err(LlmError::Disk)?.is_none_or(|target| stays_inside(&target));
        if !stays_inside(&path) || !link_ok || entry.header().entry_type().is_hard_link() {
            return Err(LlmError::Corrupt(format!("{} names {} outside itself", archive.display(), path.display())));
        }
        entry.unpack_in(into).map_err(LlmError::Disk)?;
    }
    Ok(())
}

fn unpack_zip(archive: &Path, into: &Path) -> Result<(), LlmError> {
    let mut zip = zip::ZipArchive::new(File::open(archive).map_err(LlmError::Disk)?)
        .map_err(|error| LlmError::Corrupt(error.to_string()))?;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(|error| LlmError::Corrupt(error.to_string()))?;
        let Some(path) = file.enclosed_name() else {
            return Err(LlmError::Corrupt(format!("{} names a file outside itself", archive.display())));
        };
        let to = into.join(path);
        if file.is_dir() {
            fs::create_dir_all(&to).map_err(LlmError::Disk)?;
            continue;
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(LlmError::Disk)?;
        }
        let mut out = File::create(&to).map_err(LlmError::Disk)?;
        io::copy(&mut file, &mut out).map_err(LlmError::Disk)?;
        if file.unix_mode().is_some_and(|mode| mode & 0o111 != 0) {
            silt_os::make_executable(&to).map_err(LlmError::Disk)?;
        }
    }
    Ok(())
}

/// `llama-server` under `dir`, a few folders down at most.
fn find_program(dir: &Path, depth: usize) -> Option<PathBuf> {
    let name = format!("{PROGRAM}{}", std::env::consts::EXE_SUFFIX);
    let entries = fs::read_dir(dir).ok()?;
    let mut folders = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = entry.file_type().ok()?;
        if kind.is_file() && entry.file_name() == name.as_str() {
            return Some(path);
        }
        if kind.is_dir() {
            folders.push(path);
        }
    }
    folders.sort();
    (depth > 0).then(|| folders.iter().find_map(|folder| find_program(folder, depth - 1))).flatten()
}

/// The builds of llama.cpp Silt fetched, one at a time, under
/// `runtimes/llama.cpp/<tag>/`.
#[derive(Debug, Clone)]
pub struct Runtimes {
    dir: PathBuf,
}

impl Runtimes {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The `llama-server` of the build fetched, if one was.
    pub fn server(&self) -> Option<PathBuf> {
        let mut builds: Vec<PathBuf> = fs::read_dir(&self.dir)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && path.extension().is_none_or(|extension| extension != "partial"))
            .collect();
        builds.sort();
        builds.iter().rev().find_map(|build| find_program(build, MOST_DEPTH))
    }

    /// Fetches the latest release's build for this machine from `releases`
    /// (GitHub's API), replacing any fetched before, and reports how much
    /// of it is there as it comes. Answers with its `llama-server`.
    pub async fn pull(
        &self,
        client: &reqwest::Client,
        releases: &str,
        on_done: &mut (impl FnMut(u64, u64) + Send),
    ) -> Result<PathBuf, LlmError> {
        let response = client
            .get(releases)
            .header(reqwest::header::USER_AGENT, "silt")
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| LlmError::Fetch(error.to_string()))?;
        if !response.status().is_success() {
            return Err(LlmError::Fetch(format!("GitHub answered {} for llama.cpp's latest release", response.status())));
        }
        let answer: Value = response.json().await.map_err(|error| LlmError::Fetch(error.to_string()))?;
        let release = parse_release(&answer)?;
        let asset = choose_asset(&release, std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
            LlmError::Fetch(format!(
                "llama.cpp's release {} has no build Silt takes for this machine; install llama-server and put it on the PATH",
                release.tag
            ))
        })?;
        tokio::fs::create_dir_all(&self.dir).await.map_err(LlmError::Disk)?;
        let archive = self.dir.join(&asset.name);
        let wanted = Wanted { url: asset.url.clone(), sha256: asset.sha256.clone(), size: asset.size };
        let total = asset.size;
        download::fetch(client, &wanted, &archive, &mut |done| on_done(done, total)).await?;
        let partial = self.dir.join(format!("{}.partial", release.tag));
        let build = self.dir.join(&release.tag);
        let (archive_path, partial_dir) = (archive.clone(), partial.clone());
        tokio::task::spawn_blocking(move || {
            let _ = fs::remove_dir_all(&partial_dir);
            fs::create_dir_all(&partial_dir).map_err(LlmError::Disk)?;
            if archive_path.to_string_lossy().ends_with(".zip") {
                unpack_zip(&archive_path, &partial_dir)
            } else {
                unpack_tar(&archive_path, &partial_dir)
            }
        })
        .await
        .map_err(|error| LlmError::Fetch(error.to_string()))??;
        let _ = tokio::fs::remove_file(&archive).await;
        if find_program(&partial, MOST_DEPTH).is_none() {
            let _ = tokio::fs::remove_dir_all(&partial).await;
            return Err(LlmError::Corrupt(format!("{} has no {PROGRAM}", asset.name)));
        }
        let _ = tokio::fs::remove_dir_all(&build).await;
        tokio::fs::rename(&partial, &build).await.map_err(LlmError::Disk)?;
        let mut older = tokio::fs::read_dir(&self.dir).await.map_err(LlmError::Disk)?;
        while let Ok(Some(entry)) = older.next_entry().await {
            if entry.path() != build && entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
                let _ = tokio::fs::remove_dir_all(entry.path()).await;
            }
        }
        find_program(&build, MOST_DEPTH).ok_or_else(|| LlmError::Corrupt(format!("{} has no {PROGRAM}", asset.name)))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SHA: &str = "2fdd20997c4d88ee25f70f500c61f8b999378d92ab055f9d450fc70d617158d3";

    fn release() -> Release {
        let asset = |name: &str| {
            json!({"name": name, "browser_download_url": format!("https://github.com/ggml-org/llama.cpp/releases/download/b9700/{name}"),
                "size": 100, "digest": format!("sha256:{SHA}")})
        };
        parse_release(&json!({
            "tag_name": "b9700",
            "assets": [
                asset("llama-b9700-bin-macos-arm64.tar.gz"),
                asset("llama-b9700-bin-ubuntu-x64.tar.gz"),
                asset("llama-b9700-bin-ubuntu-vulkan-x64.tar.gz"),
                asset("llama-b9700-bin-win-cuda-12.4-x64.zip"),
                {"name": "llama-b9700-bin-ubuntu-arm64.tar.gz", "browser_download_url": "https://x/y", "size": 1},
            ],
        }))
        .expect("parses")
    }

    #[test]
    fn each_machine_gets_its_build() {
        let release = release();
        let chosen = |os, arch| choose_asset(&release, os, arch).map(|asset| asset.name.as_str());
        assert_eq!(chosen("macos", "aarch64"), Some("llama-b9700-bin-macos-arm64.tar.gz"));
        assert_eq!(
            chosen("linux", "x86_64"),
            Some("llama-b9700-bin-ubuntu-vulkan-x64.tar.gz"),
            "Vulkan, for NVIDIA and AMD alike"
        );
        assert_eq!(chosen("linux", "aarch64"), None, "an asset without a SHA-256 is never taken");
        assert_eq!(chosen("windows", "x86_64"), None);
    }

    #[test]
    fn a_release_needs_a_plain_tag() {
        assert!(parse_release(&json!({"tag_name": "../x", "assets": []})).is_err());
        assert!(parse_release(&json!({"assets": []})).is_err());
    }

    #[test]
    fn an_archive_naming_a_path_outside_itself_is_refused() {
        assert!(stays_inside(Path::new("build/bin/llama-server")));
        assert!(!stays_inside(Path::new("../evil")));
        assert!(!stays_inside(Path::new("/etc/passwd")));
        assert!(!stays_inside(Path::new("")));
    }

    #[test]
    fn a_build_unpacks_and_its_server_is_found() {
        let dir = tempfile::tempdir().expect("temp");
        let archive = dir.path().join("build.tar.gz");
        {
            let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
                File::create(&archive).expect("create"),
                flate2::Compression::fast(),
            ));
            for (path, mode) in [("build/bin/llama-server", 0o755), ("build/bin/libllama.so", 0o644)] {
                let mut header = tar::Header::new_gnu();
                header.set_size(4);
                header.set_mode(mode);
                header.set_cksum();
                tar.append_data(&mut header, path, &b"elf\n"[..]).expect("append");
            }
            tar.into_inner().expect("finish").finish().expect("gzip");
        }
        let into = dir.path().join("out");
        fs::create_dir_all(&into).expect("mkdir");
        unpack_tar(&archive, &into).expect("unpacks");
        assert_eq!(find_program(&into, MOST_DEPTH), Some(into.join("build/bin/llama-server")));
        assert!(into.join("build/bin/libllama.so").is_file(), "its libraries come along");
    }
}
