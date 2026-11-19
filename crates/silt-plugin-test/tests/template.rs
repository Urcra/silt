//! `templates/plugin`, the template people start a plugin from, built and
//! tested here against this repository's SDK and harness, so it can't rot.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn repo() -> PathBuf {
    Path::new(::core::env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the repository")
}

/// The template copied into `target/`, its git dependencies on Silt pointed
/// at this checkout and its lock taken from the workspace's.
pub(crate) fn template() -> &'static str {
    static DIR: OnceLock<String> = OnceLock::new();
    DIR.get_or_init(|| {
        let repo = repo();
        let into = repo.join("target/template-plugin");
        let _ = std::fs::remove_dir_all(&into);
        copy(&repo.join("templates/plugin"), &into);
        let manifest = into.join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest).expect("its Cargo.toml");
        let git = r#"git = "https://github.com/Urcra/silt", branch = "main""#;
        assert!(text.contains(git), "the template depends on Silt by git: {text}");
        let mut patched = String::new();
        for line in text.lines() {
            let line = match line.split_once(" = {") {
                Some((name, _)) if line.contains(git) => {
                    format!("{name} = {{ path = \"{}\" }}", repo.join("crates").join(name.trim()).display())
                }
                _ => line.to_owned(),
            };
            patched.push_str(&line);
            patched.push('\n');
        }
        std::fs::write(&manifest, patched).expect("patched");
        std::fs::copy(repo.join("Cargo.lock"), into.join("Cargo.lock")).expect("the lock");
        into.to_string_lossy().into_owned()
    })
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("a folder");
    for entry in std::fs::read_dir(from).expect("the template").flatten() {
        let path = entry.path();
        if entry.file_type().expect("its type").is_dir() {
            if entry.file_name() != "target" {
                copy(&path, &to.join(entry.file_name()));
            }
        } else {
            std::fs::copy(&path, to.join(entry.file_name())).expect("copied");
        }
    }
}

// The template's own tests, finding it where it was copied.
macro_rules! env {
    ("CARGO_MANIFEST_DIR") => {
        $crate::template()
    };
}

#[path = "../../../templates/plugin/tests/plugin.rs"]
mod plugin;

#[test]
fn its_manifest_is_valid_and_names_a_publisher_of_its_own() {
    let text = std::fs::read_to_string(repo().join("templates/plugin/silt-plugin.toml")).expect("its manifest");
    let manifest = silt_plugin::Manifest::parse(&text).expect("valid");
    assert!(manifest.id.starts_with(&format!("{}.", manifest.publisher)));
    assert_ne!(manifest.publisher, "silt", "a template for other people's plugins");
}
