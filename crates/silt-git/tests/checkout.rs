use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use silt_git::Repo;

fn git(dir: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.invalid"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    output
}

fn stdout(output: Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn source_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    fs::write(root.join("a.txt"), "one\n").expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "init"]);
    git(root, &["remote", "add", "origin", "https://example.invalid/repo.git"]);
    dir
}

#[test]
fn managed_checkout_borrows_objects_and_stays_out_of_the_source() {
    let dir = source_repo();
    let repo = Repo::discover(dir.path()).expect("discovers");
    let base = repo.head_commit().expect("head").expect("commit");
    let checkout = repo.create_managed_checkout("t1", base, None).expect("creates");

    assert_eq!(checkout.path, repo.root().join(".silt/checkouts/t1"));
    assert_eq!(fs::read_to_string(checkout.path.join("a.txt")).expect("materialized"), "one\n");
    assert_eq!(stdout(git(&checkout.path, &["status", "--porcelain"])), "");
    assert_eq!(stdout(git(&checkout.path, &["rev-parse", "HEAD"])), base.to_string());
    assert_eq!(stdout(git(&checkout.path, &["remote"])), "local\norigin");
    assert!(stdout(git(&checkout.git_dir, &["count-objects", "-v"])).contains("count: 0"));
    assert_eq!(stdout(git(repo.root(), &["status", "--porcelain"])), "", "source sees nothing new");
}

#[test]
fn checkout_survives_aggressive_gc_in_the_source() {
    let dir = source_repo();
    let root = dir.path();
    git(root, &["switch", "-q", "-c", "feature"]);
    fs::write(root.join("b.txt"), "two\n").expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "feature"]);
    let repo = Repo::discover(root).expect("discovers");
    let base = repo.head_commit().expect("head").expect("commit");
    let checkout = repo.create_managed_checkout("t2", base, None).expect("creates");

    git(root, &["switch", "-q", "main"]);
    git(root, &["branch", "-q", "-D", "feature"]);
    git(root, &["reflog", "expire", "--expire=now", "--expire-unreachable=now", "--all"]);
    git(root, &["gc", "-q", "--prune=now"]);

    git(&checkout.path, &["fsck", "--connectivity-only"]);
    assert_eq!(stdout(git(&checkout.path, &["log", "--format=%s", "-1"])), "feature");
}

#[test]
fn pushing_to_local_updates_a_clean_checked_out_branch() {
    let dir = source_repo();
    let repo = Repo::discover(dir.path()).expect("discovers");
    let base = repo.head_commit().expect("head").expect("commit");
    let checkout = repo.create_managed_checkout("t3", base, None).expect("creates");

    fs::write(checkout.path.join("a.txt"), "changed\n").expect("write");
    git(&checkout.path, &["commit", "-q", "-am", "agent work"]);
    git(&checkout.path, &["push", "-q", "local", "HEAD:main"]);
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).expect("read"), "changed\n");
}

#[test]
fn removing_a_checkout_cleans_up() {
    let dir = source_repo();
    let repo = Repo::discover(dir.path()).expect("discovers");
    let base = repo.head_commit().expect("head").expect("commit");
    let checkout = repo.create_managed_checkout("t4", base, None).expect("creates");
    repo.remove_managed_checkout("t4", &checkout).expect("removes");
    assert!(!checkout.path.exists() && !checkout.git_dir.exists());
    assert_eq!(stdout(git(repo.root(), &["for-each-ref", "refs/silt"])), "");
}
