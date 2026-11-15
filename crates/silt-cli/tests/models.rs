//! Local models through a real daemon, with `silt-fake-llama` in place of
//! llama.cpp's server (ADR 0066).

mod common;

use std::path::{Path, PathBuf};
use std::process::Output;

use axum::Router;
use axum::extract::Path as UrlPath;
use axum::routing::get;
use common::{fake_agent, silt_command, temp_root};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

struct Here {
    root: tempfile::TempDir,
    /// Hugging Face's and GitHub's stand-in, when the test has one.
    web: String,
    /// Whether `SILT_LLAMA_SERVER` names the fake server.
    server: bool,
}

impl Here {
    fn new() -> Self {
        Self { root: temp_root("silt-models"), web: "http://127.0.0.1:9".into(), server: true }
    }

    fn with_web(web: String, server: bool) -> Self {
        Self { root: temp_root("silt-models"), web, server }
    }

    fn dir(&self) -> &Path {
        self.root.path()
    }

    fn silt(&self, args: &[&str]) -> Output {
        let mut command = silt_command();
        command
            .args(args)
            .current_dir(self.dir())
            .env("SILT_HOME", self.dir().join("silt"))
            .env("SILT_HUGGINGFACE_URL", &self.web)
            .env("SILT_LLAMA_RELEASES_URL", format!("{}/releases/latest", self.web));
        if self.server {
            command.env("SILT_LLAMA_SERVER", fake_agent("llama"));
        } else {
            command.env_remove("SILT_LLAMA_SERVER").env("PATH", std::env::var_os("PATH").unwrap_or_default());
        }
        command.output().expect("silt runs")
    }

    fn stdout(&self, args: &[&str]) -> String {
        let output = self.silt(args);
        assert!(output.status.success(), "silt {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

impl Drop for Here {
    fn drop(&mut self) {
        let _ = silt_command().args(["daemon", "stop"]).env("SILT_HOME", self.dir().join("silt")).output();
    }
}

/// A GGUF header, all a model needs to be listed.
fn gguf(name: &str) -> Vec<u8> {
    let mut bytes = b"GGUF".to_vec();
    bytes.extend(3u32.to_le_bytes());
    bytes.extend(0u64.to_le_bytes());
    bytes.extend(3u64.to_le_bytes());
    for (key, value) in [("general.architecture", "llama"), ("general.name", name)] {
        bytes.extend((key.len() as u64).to_le_bytes());
        bytes.extend(key.as_bytes());
        bytes.extend(8u32.to_le_bytes());
        bytes.extend((value.len() as u64).to_le_bytes());
        bytes.extend(value.as_bytes());
    }
    let key = "llama.context_length";
    bytes.extend((key.len() as u64).to_le_bytes());
    bytes.extend(key.as_bytes());
    bytes.extend(4u32.to_le_bytes());
    bytes.extend(16_384u32.to_le_bytes());
    bytes
}

#[test]
fn a_gguf_file_here_is_listed_used_where_it_is_and_removed_without_touching_it() {
    let here = Here::new();
    let listed = here.stdout(&["model", "list"]);
    assert!(listed.contains("FAKE0: Fake GPU (25.8 GB)"), "{listed}");
    assert!(listed.contains("No models here yet."), "{listed}");
    assert!(listed.contains("silt model search"), "{listed}");

    let file: PathBuf = here.dir().join("Tiny Model.gguf");
    std::fs::write(&file, gguf("Tiny")).expect("write");
    let pulled = here.stdout(&["model", "pull", "Tiny Model.gguf"]);
    assert!(pulled.starts_with("tiny-model is ready: Tiny"), "{pulled}");
    let listed = here.stdout(&["model", "list"]);
    assert!(listed.contains("tiny-model  Tiny"), "{listed}");
    assert!(listed.contains(&format!("from {}", common::canonical(&file).display())), "{listed}");

    let removed = here.stdout(&["model", "rm", "tiny-model"]);
    assert!(removed.starts_with("removed tiny-model;"), "{removed}");
    assert!(file.exists(), "the person's file stays");
    assert!(here.stdout(&["model", "list"]).contains("No models here yet."));
}

#[test]
fn what_names_no_model_is_refused_and_a_failed_download_says_why() {
    let here = Here::new();
    let refused = here.silt(&["model", "pull", "nonsense"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("isn't a Hugging Face repository"));

    let unreachable = here.silt(&["model", "pull", "owner/repo:model.gguf"]);
    assert!(!unreachable.status.success());
    let said = String::from_utf8_lossy(&unreachable.stderr);
    assert!(said.contains("couldn't download model"), "{said}");
    assert!(here.stdout(&["model", "list"]).contains("model failed to download"));
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Serves `app` on loopback from a thread of its own, for the test's life.
fn serve(app: Router) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
            axum::serve(listener, app).await.expect("serves");
        });
    });
    url
}

/// Hugging Face, with one repository: a model split in two, and a
/// projector beside it that isn't one.
fn hub() -> String {
    let first = gguf("Split Model");
    let second = vec![7u8; 4096];
    let file = |path: &str, bytes: &[u8]| json!({"rfilename": path, "size": bytes.len(), "lfs": {"sha256": sha256(bytes), "size": bytes.len()}});
    let model = json!({
        "id": "owner/split-GGUF",
        "gguf": {"architecture": "llama", "context_length": 16384},
        "siblings": [
            {"rfilename": "README.md", "size": 3},
            file("Q4_K_M/Split-Q4_K_M-00001-of-00002.gguf", &first),
            file("Q4_K_M/Split-Q4_K_M-00002-of-00002.gguf", &second),
            file("mmproj-F16.gguf", b"x"),
        ],
    });
    let search = json!([{"id": "owner/split-GGUF", "downloads": 1234, "lastModified": "2026-11-01T00:00:00.000Z"}]);
    serve(
        Router::new()
            .route("/api/models", get(move || async move { axum::Json(search) }))
            .route("/api/models/{owner}/{name}", get(move || async move { axum::Json(model) }))
            .route(
                "/{owner}/{name}/resolve/main/{*file}",
                get(move |UrlPath((_, _, file)): UrlPath<(String, String, String)>| async move {
                    if file.ends_with("00001-of-00002.gguf") { first } else { second }
                }),
            ),
    )
}

#[test]
fn a_model_found_on_hugging_face_is_chosen_by_its_quantization_and_pulled_whole() {
    let here = Here::with_web(hub(), true);
    let found = here.stdout(&["model", "search"]);
    assert!(found.starts_with("owner/split-GGUF  1234 downloads"), "{found}");
    let files = here.stdout(&["model", "files", "owner/split-GGUF"]);
    assert!(files.contains("owner/split-GGUF (llama, trained for 16384 tokens of context)"), "{files}");
    assert!(files.contains("Q4_K_M"), "{files}");
    assert!(files.contains("2 parts"), "{files}");
    assert!(!files.contains("mmproj"), "a projector isn't a model: {files}");
    assert!(files.contains("silt model pull owner/split-GGUF:Q4_K_M/Split-Q4_K_M-00001-of-00002.gguf"), "{files}");

    let pulled = here.stdout(&["model", "pull", "owner/split-GGUF:Q4_K_M/Split-Q4_K_M-00001-of-00002.gguf"]);
    assert!(pulled.starts_with("split-q4_k_m is ready: Split Model"), "{pulled}");
    let kept = here.dir().join("silt/models/llm/split-q4_k_m/Q4_K_M");
    assert!(kept.join("Split-Q4_K_M-00002-of-00002.gguf").is_file(), "every part came");
    let listed = here.stdout(&["model", "list"]);
    assert!(listed.contains("16384 context"), "sized to what it was trained for, as it fits: {listed}");
}

/// GitHub's latest llama.cpp release, whose builds hold `llama-server`:
/// here, the fake one.
#[cfg(not(windows))]
fn releases() -> String {
    let program = std::fs::read(fake_agent("llama")).expect("the fake server");
    let mut archive = Vec::new();
    {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(&mut archive, flate2::Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_size(program.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "build/bin/llama-server", program.as_slice()).expect("append");
        tar.into_inner().expect("finish").finish().expect("gzip");
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    drop(listener);
    let assets: Vec<Value> = ["macos-arm64", "ubuntu-vulkan-x64", "ubuntu-vulkan-arm64"]
        .iter()
        .map(|kind| {
            json!({"name": format!("llama-b9999-bin-{kind}.tar.gz"), "browser_download_url": format!("{url}/download/build.tar.gz"),
                "size": archive.len(), "digest": format!("sha256:{}", sha256(&archive))})
        })
        .collect();
    let release = json!({"tag_name": "b9999", "assets": assets});
    let app = Router::new()
        .route("/releases/latest", get(move || async move { axum::Json(release) }))
        .route("/download/build.tar.gz", get(move || async move { archive }));
    let address = url.trim_start_matches("http://").to_owned();
    let listener = std::net::TcpListener::bind(&address).expect("rebind");
    listener.set_nonblocking(true).expect("nonblocking");
    std::thread::spawn(move || {
        tokio::runtime::Runtime::new().expect("runtime").block_on(async move {
            axum::serve(tokio::net::TcpListener::from_std(listener).expect("listener"), app).await.expect("serves");
        });
    });
    url
}

#[cfg(not(windows))]
#[test]
fn llama_cpp_is_fetched_when_asked_and_runs_models() {
    let here = Here::with_web(releases(), false);
    let before = here.stdout(&["model", "list"]);
    assert!(before.contains("llama-server: not found; `silt model runtime`"), "{before}");
    let fetched = here.stdout(&["model", "runtime"]);
    assert!(fetched.contains("llama.cpp is ready:"), "{fetched}");
    assert!(fetched.contains("runtimes/llama.cpp/b9999/build/bin/llama-server"), "{fetched}");
    let after = here.stdout(&["model", "list"]);
    assert!(after.contains("FAKE0: Fake GPU"), "the fetched server lists the devices: {after}");
}
