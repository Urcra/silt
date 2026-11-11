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
fn a_made_checkout_is_kept_and_an_interrupted_one_made_again() {
    let dir = source_repo();
    let repo = Repo::discover(dir.path()).expect("discovers");
    let base = repo.head_commit().expect("head").expect("commit");
    let checkout = repo.create_managed_checkout("t6", base, None).expect("creates");
    assert!(repo.managed_checkout_ready("t6"));
    fs::write(checkout.path.join("work.txt"), "someone's work\n").expect("write");
    assert_eq!(repo.create_managed_checkout("t6", base, None).expect("again"), checkout);
    assert!(checkout.path.join("work.txt").exists(), "making it again keeps what is there");

    let interrupted = repo.create_managed_checkout("t7", base, None).expect("creates");
    git(&interrupted.git_dir, &["config", "--unset", "silt.ready"]);
    for entry in fs::read_dir(&interrupted.path).expect("lists").flatten() {
        if entry.file_name() != ".git" {
            fs::remove_file(entry.path()).expect("remove");
        }
    }
    assert!(!repo.managed_checkout_ready("t7"));
    let remade = repo.create_managed_checkout("t7", base, None).expect("made again");
    assert_eq!(fs::read_to_string(remade.path.join("a.txt")).expect("materialized"), "one\n");
    assert!(repo.managed_checkout_ready("t7"));
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

#[test]
fn bundles_carry_commits_origin_does_not_have() {
    let origin = source_repo();
    let clone_dir = tempfile::tempdir().expect("temp dir");
    let clone = clone_dir.path().join("a");
    let other = clone_dir.path().join("b");
    let status = std::process::Command::new("git").args(["clone", "-q"]).arg(origin.path()).arg(&clone).status().expect("clone");
    assert!(status.success());
    std::process::Command::new("git").args(["clone", "-q"]).arg(origin.path()).arg(&other).status().expect("clone");

    let repo = Repo::discover(&clone).expect("discovers");
    let head = repo.head_commit().expect("head").expect("commit");
    assert_eq!(repo.bundle(head, &[]).expect("bundle"), None, "origin already has HEAD");

    fs::write(clone.join("local.txt"), "only here\n").expect("write");
    git(&clone, &["add", "."]);
    git(&clone, &["commit", "-q", "-m", "local only"]);
    let local = repo.head_commit().expect("head").expect("commit");
    let bundle = repo.bundle(local, &[]).expect("bundle").expect("not empty");

    let other = Repo::discover(&other).expect("discovers");
    assert!(!other.has_commit(local));
    other.fetch_bundle(&bundle, "t9").expect("fetches");
    assert!(other.has_commit(local));
    assert_eq!(stdout(git(other.root(), &["rev-parse", "refs/silt/threads/t9"])), local.to_string());

    assert_eq!(repo.bundle(local, &[local]).expect("bundle"), None, "a known commit isn't bundled again");
    fs::write(clone.join("local.txt"), "and more\n").expect("write");
    git(&clone, &["commit", "-q", "-am", "more"]);
    let next = repo.head_commit().expect("head").expect("commit");
    let increment = repo.bundle(next, &[local]).expect("bundle").expect("not empty");
    assert!(increment.len() < bundle.len() + 200, "only what came since: {} vs {}", increment.len(), bundle.len());
    other.fetch_bundle(&increment, "t9").expect("fetches on top of the first");
    assert!(other.has_commit(next));
}

#[test]
fn linked_paths_are_shared_and_prepare_runs_in_the_checkout() {
    use silt_git::conventions::{LinkOutcome, PrepareOutcome, link_shared_paths, run_prepare};
    let dir = source_repo();
    let root = dir.path();
    fs::create_dir_all(root.join(".agents")).expect("mkdir");
    fs::write(root.join(".gitignore"), "node_modules/\n.env\n").expect("write");
    fs::write(root.join(".agents/linked"), "node_modules\n.env\ncache\na.txt\n").expect("write");
    // Where it writes shows where it ran; `pwd` would name the checkout in
    // Git for Windows' own form there.
    fs::write(root.join(".agents/prepare"), "#!/bin/sh\necho \"ran $SILT_THREAD\" > prepared.txt\n").expect("write");
    silt_os::make_executable(&root.join(".agents/prepare")).expect("chmod");
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "conventions"]);
    fs::create_dir_all(root.join("node_modules/left-pad")).expect("mkdir");
    fs::write(root.join("node_modules/left-pad/index.js"), "pad").expect("write");
    let repo = Repo::discover(root).expect("discovers");
    let base = repo.head_commit().expect("head").expect("commit");
    let checkout = repo.create_managed_checkout("t5", base, None).expect("creates");
    let checkout_repo = Repo::discover(&checkout.path).expect("discovers the checkout");
    let shared = tempfile::tempdir().expect("temp dir");

    let outcomes = link_shared_paths(&checkout_repo, root, shared.path()).expect("links");
    assert_eq!(
        outcomes,
        vec![
            LinkOutcome::Linked { path: "node_modules".into(), seeded: true },
            LinkOutcome::Linked { path: ".env".into(), seeded: false },
            LinkOutcome::NotIgnored { path: "cache".into() },
            LinkOutcome::NotIgnored { path: "a.txt".into() },
        ]
    );
    assert_eq!(fs::read_to_string(checkout.path.join("node_modules/left-pad/index.js")).expect("seeded"), "pad");
    assert_eq!(stdout(git(&checkout.path, &["status", "--porcelain"])), "", "links are ignored");
    let again = link_shared_paths(&checkout_repo, root, shared.path()).expect("links again");
    assert_eq!(again[0], LinkOutcome::AlreadyLinked { path: "node_modules".into() });

    let outcome = run_prepare(&checkout.path, &[("SILT_THREAD", "t5")]).expect("runs");
    assert!(matches!(outcome, Some(PrepareOutcome::Ran { ref status, .. }) if status.success()), "{outcome:?}");
    assert_eq!(fs::read_to_string(checkout.path.join("prepared.txt")).expect("prepared in the checkout"), "ran t5\n");
}

#[test]
fn a_repository_without_a_remote_is_the_project_of_its_folder_and_its_clones() {
    let dir = source_repo();
    git(dir.path(), &["remote", "remove", "origin"]);
    let repo = Repo::discover(dir.path()).expect("discovers");
    let folder = repo.root().to_string_lossy().into_owned();
    assert_eq!(repo.project().expect("project"), silt_git::normalize_remote(&folder));
    let base = repo.head_commit().expect("head").expect("commit");
    let checkout = repo.create_managed_checkout("t1", base, None).expect("creates");
    assert_eq!(Repo::discover(&checkout.path).expect("discovers").project().expect("project"), repo.project().expect("project"));
    let elsewhere = tempfile::tempdir().expect("temp dir");
    let clone = elsewhere.path().join("clone");
    git(elsewhere.path(), &["clone", "-q", &folder, "clone"]);
    assert_eq!(Repo::discover(&clone).expect("discovers").project().expect("project"), repo.project().expect("project"));
    git(dir.path(), &["remote", "add", "origin", "git@github.com:acme/app.git"]);
    assert_eq!(Repo::discover(dir.path()).expect("discovers").project().expect("project"), "github.com/acme/app");
}

#[test]
fn init_makes_the_folder_and_an_empty_repository() {
    let dir = tempfile::tempdir().expect("temp dir");
    let repo = Repo::init(&dir.path().join("new/app")).expect("inits");
    assert!(repo.root().ends_with("new/app"));
    assert_eq!(repo.head_commit().expect("head"), None);
    assert!(repo.project().expect("project").ends_with("/new/app"));
}

#[test]
fn a_chat_is_its_own_repository_with_the_same_first_commit_everywhere() {
    let (one, two) = (tempfile::tempdir().expect("temp dir"), tempfile::tempdir().expect("temp dir"));
    let first = Repo::init_chat(&one.path().join("2026-11-03-which-sensor-1a2b3c")).expect("inits");
    let second = Repo::init_chat(&two.path().join("elsewhere")).expect("inits");
    assert!(first.is_chat());
    assert_eq!(first.project().expect("project"), silt_types::CHATS);
    let root = first.head_commit().expect("head").expect("a first commit");
    assert_eq!(second.head_commit().expect("head"), Some(root), "the same commit on every machine");
    assert!(first.files_at(root).expect("files").is_empty());
    assert_eq!(first.head_branch().expect("branch").as_deref(), Some("main"));

    first.leave_chats().expect("leaves");
    let promoted = Repo::discover(first.root()).expect("discovers");
    assert!(!promoted.is_chat());
    assert!(promoted.project().expect("project").ends_with("/2026-11-03-which-sensor-1a2b3c"), "a project of its folder");
    assert_eq!(promoted.head_commit().expect("head"), Some(root));
}
