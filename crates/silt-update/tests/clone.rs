//! The check against a real clone and its upstream.

use std::path::Path;
use std::process::Command;

use silt_update::check::{self, Availability, OffReason};
use silt_update::{BuildInfo, Channel, Commit, Source};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn commit(dir: &Path, file: &str, text: &str, message: &str) -> Commit {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
    std::fs::write(&path, text).expect("write");
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "--quiet", "-m", message]);
    Commit::parse(&git(dir, &["rev-parse", "HEAD"])).expect("a commit")
}

/// An upstream, a clone of it on `main`, and the clone's first commit.
fn clone_of_upstream(root: &Path) -> (std::path::PathBuf, std::path::PathBuf, Commit) {
    let upstream = root.join("upstream");
    std::fs::create_dir_all(&upstream).expect("dir");
    git(&upstream, &["init", "--quiet", "--initial-branch=main"]);
    let first = commit(&upstream, "README.md", "one", "first");
    let clone = root.join("clone");
    git(root, &["clone", "--quiet", upstream.to_str().expect("utf-8"), clone.to_str().expect("utf-8")]);
    (upstream, clone, first)
}

fn built(clone: &Path, commit: &Commit) -> BuildInfo {
    BuildInfo {
        version: "0.0.1".into(),
        channel: Channel::Nightly,
        commit: Some(commit.clone()),
        branch: Some("main".into()),
        dirty: false,
        source: Some(Source::Local(clone.to_owned())),
        debug: false,
    }
}

#[test]
fn main_moving_upstream_is_an_update_that_lists_its_commits() {
    let root = tempfile::tempdir().expect("dir");
    let (upstream, clone, first) = clone_of_upstream(root.path());
    assert_eq!(check::nightly(&built(&clone, &first), true), Availability::UpToDate);

    commit(&upstream, "a.txt", "a", "feat: a");
    let newest = commit(&upstream, "b.txt", "b", "fix: b");
    match check::nightly(&built(&clone, &first), true) {
        Availability::Behind { target, ahead, commits } => {
            assert_eq!(target, newest);
            assert_eq!(ahead, 2);
            let subjects: Vec<_> = commits.iter().map(|commit| commit.subject.as_str()).collect();
            assert_eq!(subjects, ["fix: b", "feat: a"]);
        }
        other => panic!("expected an update, got {other:?}"),
    }
    // Without a fetch it says what it last fetched.
    assert!(matches!(check::nightly(&built(&clone, &first), false), Availability::Behind { .. }));
}

#[test]
fn a_clone_off_main_with_changes_or_gone_is_left_alone() {
    let root = tempfile::tempdir().expect("dir");
    let (_upstream, clone, first) = clone_of_upstream(root.path());
    std::fs::write(clone.join("README.md"), "edited").expect("edit");
    assert_eq!(check::nightly(&built(&clone, &first), true), Availability::Off { reason: OffReason::LocalChanges });
    git(&clone, &["checkout", "--quiet", "--", "README.md"]);
    commit(&clone, "local.txt", "x", "local");
    assert_eq!(check::nightly(&built(&clone, &first), true), Availability::Off { reason: OffReason::LocalCommits { count: 1 } });
    git(&clone, &["switch", "--quiet", "-c", "feature"]);
    assert_eq!(
        check::nightly(&built(&clone, &first), true),
        Availability::Off { reason: OffReason::OffMain { branch: Some("feature".into()) } }
    );
    let gone = root.path().join("gone");
    assert!(matches!(check::nightly(&built(&gone, &first), true), Availability::Off { reason: OffReason::CloneGone { .. } }));
}

#[test]
fn only_a_change_to_the_desktop_crate_needs_a_relaunch() {
    let root = tempfile::tempdir().expect("dir");
    let (upstream, clone, first) = clone_of_upstream(root.path());
    let web_only = commit(&upstream, "web/src/App.tsx", "x", "feat(web): x");
    let desktop = commit(&upstream, "crates/silt-desktop/src/main.rs", "fn main() {}", "feat(desktop): y");
    git(&clone, &["pull", "--quiet", "--ff-only"]);
    let source = Source::Local(clone);
    assert!(!check::desktop_differs(&source, &first, &web_only));
    assert!(check::desktop_differs(&source, &first, &desktop));
    assert!(!check::desktop_differs(&source, &desktop, &desktop));
    let unknown = Commit::parse("0123456789abcdef0123456789abcdef01234567").expect("hex");
    assert!(check::desktop_differs(&source, &unknown, &desktop));
}
