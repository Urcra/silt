use std::fs;
use std::path::Path;
use std::process::Command;

use silt_git::{FileMode, Repo};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.invalid"])
        .args(args)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

fn repo_with_commit() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    fs::create_dir(root.join("src")).expect("mkdir");
    fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("write");
    fs::write(root.join("run.sh"), "#!/bin/sh\n").expect("write");
    fs::write(root.join(".gitignore"), "target/\n*.log\n").expect("write");
    git(root, &["add", "."]);
    git(root, &["update-index", "--chmod=+x", "run.sh"]);
    git(root, &["commit", "-q", "-m", "init"]);
    git(root, &["remote", "add", "origin", "git@github.com:Urcra/silt.git"]);
    dir
}

#[test]
fn reads_head_branch_remote_and_tree() {
    let dir = repo_with_commit();
    let repo = Repo::discover(&dir.path().join("src")).expect("discovers from a subdirectory");
    assert_eq!(repo.root(), dir.path().canonicalize().expect("canonical"));
    assert_eq!(repo.head_branch().expect("branch").as_deref(), Some("main"));
    assert_eq!(
        repo.remote_url("origin").expect("remote").as_deref(),
        Some("git@github.com:Urcra/silt.git")
    );
    assert_eq!(repo.remote_url("upstream").expect("remote"), None);

    let head = repo.head_commit().expect("head").expect("has a commit");
    let mut files = repo.files_at(head).expect("tree");
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let summary: Vec<(&str, FileMode)> = files.iter().map(|f| (f.path.as_str(), f.mode)).collect();
    assert_eq!(
        summary,
        vec![
            (".gitignore", FileMode::Regular),
            ("run.sh", FileMode::Executable),
            ("src/main.rs", FileMode::Regular),
        ]
    );
    let main = files.iter().find(|f| f.path == "src/main.rs").expect("main.rs");
    assert_eq!(repo.blob(main.oid).expect("blob"), b"fn main() {}\n");
}

#[test]
fn files_whose_names_are_not_utf8_are_left_out() {
    use std::os::unix::ffi::OsStrExt as _;
    let dir = repo_with_commit();
    let root = dir.path();
    fs::write(root.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt")), "latin-1\n").expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "latin-1 name"]);
    let repo = Repo::discover(root).expect("discovers");
    let head = repo.head_commit().expect("head").expect("has a commit");
    let paths: Vec<String> = repo.files_at(head).expect("tree").into_iter().map(|file| file.path).collect();
    assert_eq!(paths.len(), 3, "{paths:?}");
    assert_eq!(repo.worktree_files().expect("files"), vec![".gitignore", "run.sh", "src/main.rs"]);
}

#[test]
fn file_mode_and_case_follow_git_config() {
    let dir = repo_with_commit();
    let repo = Repo::discover(dir.path()).expect("discovers");
    assert!(repo.tracks_file_mode());
    git(dir.path(), &["config", "core.fileMode", "false"]);
    git(dir.path(), &["config", "core.ignoreCase", "true"]);
    let repo = Repo::discover(dir.path()).expect("discovers");
    assert!(!repo.tracks_file_mode());
    assert!(repo.ignores_case());
}

#[test]
fn unborn_repository_has_no_head_commit() {
    let dir = tempfile::tempdir().expect("temp dir");
    git(dir.path(), &["init", "-q"]);
    let repo = Repo::discover(dir.path()).expect("discovers");
    assert_eq!(repo.head_commit().expect("head"), None);
}

#[test]
fn worktree_files_include_untracked_but_not_ignored_or_deleted() {
    let dir = repo_with_commit();
    let root = dir.path();
    fs::write(root.join("notes.txt"), "untracked\n").expect("write");
    fs::write(root.join("debug.log"), "ignored\n").expect("write");
    fs::create_dir(root.join("target")).expect("mkdir");
    fs::write(root.join("target/out"), "ignored\n").expect("write");
    fs::remove_file(root.join("run.sh")).expect("delete tracked file");
    let repo = Repo::discover(root).expect("discovers");
    assert_eq!(
        repo.worktree_files().expect("files"),
        vec![".gitignore", "notes.txt", "src/main.rs"]
    );
    let ignored = repo
        .ignored(&["debug.log".into(), "target/out".into(), "src/main.rs".into(), "new.rs".into()])
        .expect("check-ignore");
    assert_eq!(ignored, ["debug.log".to_owned(), "target/out".to_owned()].into());
    assert!(repo.ignored(&["src/main.rs".into()]).expect("check-ignore").is_empty());
}

#[test]
fn filters_follow_autocrlf_and_attributes() {
    let dir = repo_with_commit();
    let root = dir.path();
    fs::write(root.join(".gitattributes"), "*.bat text eol=crlf\n").expect("write");
    let repo = Repo::discover(root).expect("discovers");
    let mut filters = repo.filters().expect("filters");
    assert_eq!(filters.to_worktree("x.bat", b"a\nb\n").expect("smudge"), b"a\r\nb\r\n");
    assert_eq!(filters.to_git("x.bat", b"a\r\nb\r\n").expect("clean"), b"a\nb\n");
    assert_eq!(filters.to_worktree("x.rs", b"a\nb\n").expect("smudge"), b"a\nb\n");
}

#[test]
fn ident_attribute_falls_back_to_git() {
    let dir = repo_with_commit();
    let root = dir.path();
    fs::write(root.join(".gitattributes"), "*.c ident\n").expect("write");
    let repo = Repo::discover(root).expect("discovers");
    let mut filters = repo.filters().expect("filters");
    let smudged = filters.to_worktree("a.c", b"/* $Id$ */\n").expect("smudge");
    let smudged = String::from_utf8(smudged).expect("utf-8");
    assert!(smudged.starts_with("/* $Id: ") && smudged.ends_with(" $ */\n"), "{smudged}");
    assert_eq!(filters.to_git("a.c", smudged.as_bytes()).expect("clean"), b"/* $Id$ */\n");
}

fn loose_objects(root: &Path) -> usize {
    fs::read_dir(root.join(".git/objects"))
        .expect("objects")
        .flatten()
        .filter(|dir| dir.file_name().len() == 2)
        .map(|dir| fs::read_dir(dir.path()).expect("fan-out").count())
        .sum()
}

#[test]
fn converting_through_git_leaves_no_objects_behind() {
    let dir = repo_with_commit();
    let root = dir.path();
    fs::write(root.join(".gitattributes"), "*.c ident\n").expect("write");
    let before = loose_objects(root);
    let repo = Repo::discover(root).expect("discovers");
    let mut filters = repo.filters().expect("filters");
    let smudged = filters.to_worktree("a.c", b"/* $Id$ */ unsaved draft\n").expect("smudge");
    filters.to_git("a.c", &smudged).expect("clean");
    assert_eq!(loose_objects(root), before);
}

#[test]
fn a_nested_attributes_file_counts_once_it_is_noticed() {
    let dir = repo_with_commit();
    let root = dir.path();
    let repo = Repo::discover(root).expect("discovers");
    let id = |repo: &Repo| repo.filters().expect("filters").to_worktree("src/a.c", b"$Id$\n").expect("smudge");
    assert_eq!(id(&repo), b"$Id$\n");
    fs::write(root.join("src/.gitattributes"), "*.c ident\n").expect("write");
    repo.attributes_changed();
    assert_ne!(id(&repo), b"$Id$\n", "expanded by git");
}

#[test]
fn checks_more_ignored_paths_than_a_pipe_holds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let status = std::process::Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().expect("git");
    assert!(status.success());
    std::fs::write(dir.path().join(".gitignore"), "*.log\n").expect("write");
    let root = dir.path().to_owned();
    let paths: Vec<String> = (0..20_000).map(|n| format!("target/build/output-{n}.log")).collect();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let repo = Repo::discover(&root).expect("repo");
        let _ = sender.send(repo.ignored(&paths).map(|ignored| ignored.len()));
    });
    let ignored = receiver.recv_timeout(std::time::Duration::from_secs(30)).expect("git answered");
    assert_eq!(ignored.expect("checked"), 20_000);
}
