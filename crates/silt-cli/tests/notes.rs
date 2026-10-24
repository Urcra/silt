//! Git notes on a project's commits (ADR 0042): written once the project
//! asks for them, pushed to origin with `push`, and read by `silt blame` in
//! another clone.

mod common;

use std::process::Command;

use common::{Machine, Server, eventually, git, origin};

fn rev(dir: &std::path::Path, rev: &str) -> String {
    let output = Command::new("git").args(["rev-parse", rev]).current_dir(dir).output().expect("git runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The commits `refs/notes/silt` has notes for, in a bare repository.
fn noted(git_dir: &std::path::Path) -> Vec<String> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(["ls-tree", "-r", "--name-only", "refs/notes/silt"])
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout).lines().map(|path| path.replace('/', "")).collect()
}

#[test]
fn notes_reach_origin_once_the_project_says_push_and_blame_reads_them_elsewhere() {
    let root = common::temp_root("silt-notes");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let home = Machine::new(root.path(), "home", &origin, &server);
    laptop.silt(&["claude"]);
    assert!(eventually(|| laptop.try_stdout(&["diff", &common::thread_of(&laptop)]).contains("+fn farewell() {}")));

    std::fs::write(laptop.repo.join("README"), "hello\n").expect("write");
    git(&laptop.repo, &["add", "README"]);
    git(&laptop.repo, &["commit", "-qm", "before notes"]);
    let unnoted = rev(&laptop.repo, "HEAD");
    assert!(eventually(|| laptop.try_stdout(&["log", "--commit", &unnoted[..10]]).contains("turn 1")));

    assert!(eventually(|| laptop.run(&["project", "set", "--git-notes", "push"]).status.success()));
    // A daemon reads its servers' settings as it starts.
    laptop.silt(&["daemon", "stop"]);
    laptop.silt(&["log"]);
    std::fs::write(laptop.repo.join("src/main.rs"), "fn main() {}\nfn farewell() {}\nfn by_hand() {}\n").expect("write");
    git(&laptop.repo, &["commit", "-qam", "with notes"]);
    let commit = rev(&laptop.repo, "HEAD");
    assert!(eventually(|| noted(&origin).contains(&commit)), "the note reached origin");
    assert!(!noted(&origin).contains(&unnoted), "the commit made before the setting has none");

    git(&laptop.repo, &["push", "-q", "origin", "main"]);
    git(&home.repo, &["pull", "-q", "origin", "main"]);
    git(&home.repo, &["fetch", "-q", "origin", "refs/notes/silt:refs/notes/silt"]);
    let blame = home.stdout(&["blame", "--notes", "src/main.rs"]);
    let lines: Vec<&str> = blame.lines().collect();
    assert_eq!(lines.len(), 3, "{blame}");
    assert!(lines[1].contains(" t1 ") && lines[1].contains("claude-code"), "{blame}");
    assert!(lines[1].contains("Please add a farewell"), "the turn's prompt: {blame}");
    assert!(lines[2].contains(" person ") && lines[2].ends_with("| fn by_hand() {}"), "{blame}");
    assert!(!lines[0].contains(" t1 "), "git's own line: {blame}");
}
