//! Local models through a real daemon, with `silt-fake-llama` in place of
//! llama.cpp's server (ADR 0066).

mod common;

use std::path::{Path, PathBuf};
use std::process::Output;

use common::{fake_agent, silt_command, temp_root};

struct Here {
    root: tempfile::TempDir,
}

impl Here {
    fn new() -> Self {
        Self { root: temp_root("silt-models") }
    }

    fn dir(&self) -> &Path {
        self.root.path()
    }

    fn silt(&self, args: &[&str]) -> Output {
        silt_command()
            .args(args)
            .current_dir(self.dir())
            .env("SILT_HOME", self.dir().join("silt"))
            .env("SILT_LLAMA_SERVER", fake_agent("llama"))
            .env("SILT_HUGGINGFACE_URL", "http://127.0.0.1:9")
            .output()
            .expect("silt runs")
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
    bytes.extend(2u64.to_le_bytes());
    for (key, value) in [("general.architecture", "llama"), ("general.name", name)] {
        bytes.extend((key.len() as u64).to_le_bytes());
        bytes.extend(key.as_bytes());
        bytes.extend(8u32.to_le_bytes());
        bytes.extend((value.len() as u64).to_le_bytes());
        bytes.extend(value.as_bytes());
    }
    bytes
}

#[test]
fn a_gguf_file_here_is_listed_used_where_it_is_and_removed_without_touching_it() {
    let here = Here::new();
    let listed = here.stdout(&["model", "list"]);
    assert!(listed.contains("FAKE0: Fake GPU (25.8 GB)"), "{listed}");
    assert!(listed.contains("No models here yet."), "{listed}");
    assert!(listed.contains("gpt-oss-20b  gpt-oss 20B (MXFP4)"), "{listed}");

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
