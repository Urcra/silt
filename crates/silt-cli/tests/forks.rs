//! Two users on one Silt server, all real processes: Bob forks Alice's
//! in-progress thread with its conversation, pulls her newer work, and
//! offers his back with merge requests that Alice approves or declines.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::{Machine, Server, checkout_from, eventually, origin, thread_of};

/// Fields drop in order: the machines stop their daemons before the
/// directories holding their sockets go away.
struct Team {
    alice: Machine,
    bob: Machine,
    _server: Server,
    _root: tempfile::TempDir,
}

fn team() -> Team {
    let root = tempfile::Builder::new().prefix("silt-forks").tempdir_in("/tmp").expect("temp dir");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let alice = Machine::new_for(root.path(), "alice-laptop", "alice", &origin, &server);
    let bob = Machine::new_for(root.path(), "bob-desktop", "bob", &origin, &server);
    Team {
        alice,
        bob,
        _server: server,
        _root: root,
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read")
}

/// Fires the prompt hook the way Claude Code would for `machine`'s most
/// recently restored session and returns what the hook printed.
fn prompt_hook(machine: &Machine, session: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(["hook", "UserPromptSubmit"])
        .env("SILT_HOME", &machine.silt_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("hook runs");
    let input = format!(r#"{{"session_id":"{session}","hook_event_name":"UserPromptSubmit"}}"#);
    std::io::Write::write_all(&mut child.stdin.take().expect("stdin"), input.as_bytes()).expect("write input");
    String::from_utf8_lossy(&child.wait_with_output().expect("hook finishes").stdout).into_owned()
}

fn last_session(machine: &Machine) -> String {
    read(&machine.claude_dir.join("last-session"))
}

fn first_session(machine: &Machine) -> String {
    let projects = machine.claude_dir.join("projects");
    let project: PathBuf = fs::read_dir(projects).expect("projects").flatten().next().expect("project").path();
    fs::read_dir(project)
        .expect("project dir")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "jsonl"))
        .and_then(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
        .expect("transcript")
}

fn mr_id(output: &[u8]) -> String {
    String::from_utf8_lossy(output)
        .split_whitespace()
        .nth(3)
        .expect("opened merge request <id>:")
        .trim_end_matches(':')
        .to_owned()
}

#[test]
fn a_teammate_forks_pulls_and_offers_work_back() {
    let Team { alice, bob, .. } = &team();
    alice.silt(&["claude"]);
    let thread = thread_of(alice);
    let alice_session = first_session(alice);

    assert!(
        eventually(|| bob.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))),
        "Bob sees Alice's thread: {}",
        bob.stdout(&["log"])
    );
    let refused = bob.run(&["resume", &thread]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("fork it"), "only Alice resumes her thread");

    let forked = bob.silt_with(
        &["fork", &thread],
        &[
            ("FAKE_EXPECT", "Please-add-a-farewell"),
            ("FAKE_SAY", "Bob here: adding bob"),
            ("FAKE_LINE", "fn bob() {}"),
        ],
    );
    let fork_checkout = checkout_from(&forked.stderr);
    assert_eq!(read(&fork_checkout.join("src/main.rs")), "fn main() {}\nfn farewell() {}\nfn bob() {}\n");
    let fork = bob
        .stdout(&["log"])
        .lines()
        .find(|line| line.contains(&format!("fork of {thread}")))
        .and_then(|line| line.split_whitespace().next())
        .expect("the fork is listed")
        .to_owned();
    let show = bob.stdout(&["show", &fork]);
    assert!(show.contains("before this fork") && show.contains("> Please add a farewell line"), "{show}");
    assert!(show.contains("> Bob here: adding bob"), "{show}");
    assert!(!read(&alice.repo.join("src/main.rs")).contains("fn bob"), "a fork never changes its source");

    let main = alice.repo.join("src/main.rs");
    fs::write(&main, format!("// alice was here\n{}", read(&main))).expect("Alice keeps working");
    let transcript = alice
        .claude_dir
        .join("projects")
        .join(alice.repo.to_string_lossy().replace(|ch: char| !ch.is_ascii_alphanumeric(), "-"))
        .join(format!("{alice_session}.jsonl"));
    fs::write(
        &transcript,
        read(&transcript) + "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"alice: continuing upstream\"}}\n",
    )
    .expect("append to Alice's session");
    assert!(eventually(|| bob.stdout(&["diff", &thread]).contains("+// alice was here")));
    assert!(eventually(|| bob.stdout(&["show", &thread]).contains("alice: continuing upstream")));

    let pulled = bob.run_with(&["pull", &fork], &[]);
    let pulled_text = String::from_utf8_lossy(&pulled.stdout);
    assert!(pulled.status.success(), "{}", String::from_utf8_lossy(&pulled.stderr));
    assert!(pulled_text.contains("pulled 1 messages and 1 changed files"), "{pulled_text}");
    assert!(eventually(|| read(&fork_checkout.join("src/main.rs"))
        == "// alice was here\nfn main() {}\nfn farewell() {}\nfn bob() {}\n"));
    let show = bob.stdout(&["show", &fork]);
    assert!(show.contains("upstream progress") && show.contains("alice: continuing upstream"), "{show}");
    let context = prompt_hook(bob, &last_session(bob));
    assert!(context.contains("additionalContext") && context.contains("pulled newer work"), "{context}");
    assert!(!prompt_hook(bob, &last_session(bob)).contains("pulled"), "a notice is handed over once");

    let opened = bob.silt(&["mr", "create", "--thread", &fork, "--title", "Add bob"]);
    let mr = mr_id(&opened.stdout);
    assert!(eventually(|| alice.stdout(&["mr", "list"]).contains(&mr)), "Alice sees the merge request");
    let details = alice.stdout(&["mr", "show", &mr]);
    assert!(details.contains("Add bob") && details.contains("+fn bob() {}"), "{details}");
    assert!(details.contains("Bob here: adding bob"), "the fork's conversation comes with it: {details}");
    assert!(!details.contains("conflict hints"), "{details}");
    let refused = bob.run(&["mr", "approve", &mr]);
    assert!(!refused.status.success(), "only the target's device approves");

    alice.silt(&["mr", "approve", &mr]);
    assert!(eventually(|| read(&main) == "// alice was here\nfn main() {}\nfn farewell() {}\nfn bob() {}\n"));
    let show = alice.stdout(&["show", &thread]);
    assert!(show.contains("merged \"Add bob\" from fork"), "{show}");
    assert!(!show.contains("Bob here: adding bob"), "the fork's conversation stays in the fork: {show}");
    let context = prompt_hook(alice, &alice_session);
    assert!(context.contains("merged the merge request \\\"Add bob\\\""), "{context}");
    assert!(eventually(|| bob.stdout(&["mr", "list"]).contains("approved")));

    let fork_main = fork_checkout.join("src/main.rs");
    fs::write(&fork_main, read(&fork_main) + "fn bob2() {}\n").expect("Bob keeps working");
    assert!(eventually(|| bob.stdout(&["diff", &fork]).contains("+fn bob2() {}")));
    let opened = bob.silt(&["mr", "create", "--thread", &fork, "--title", "More bob"]);
    let second = mr_id(&opened.stdout);
    assert!(eventually(|| alice.stdout(&["mr", "list"]).contains(&second)));
    alice.silt(&["mr", "decline", &second, "--reason", "not now"]);
    assert!(eventually(|| bob.stdout(&["mr", "list"]).contains("declined: not now")));
    assert!(!read(&main).contains("bob2"));
    assert!(read(&fork_main).contains("bob2"), "a declined fork lives on");

    assert!(eventually(|| alice.log_line(&fork).is_some()));
    let reforked = alice.silt_with(
        &["fork", &fork],
        &[
            ("FAKE_EXPECT", "Please-add-a-farewell Bob-here"),
            ("FAKE_SAY", "Alice picks up Bob's fork"),
            ("FAKE_LINE", "fn alice() {}"),
        ],
    );
    let refork_checkout = checkout_from(&reforked.stderr);
    let refork_main = read(&refork_checkout.join("src/main.rs"));
    assert!(refork_main.contains("fn bob2() {}") && refork_main.ends_with("fn alice() {}\n"), "{refork_main}");
}

#[test]
fn a_private_project_stays_invisible_to_teammates() {
    let Team { alice, bob, .. } = &team();
    let set = alice.stdout(&["visibility", "private"]);
    assert!(set.contains("is now private"), "{set}");
    alice.silt(&["claude"]);
    let thread = thread_of(alice);
    assert!(eventually(|| alice.stdout(&["auth", "status"]).contains("syncing")));
    std::thread::sleep(std::time::Duration::from_secs(4));
    assert!(bob.log_line(&thread).is_none(), "{}", bob.stdout(&["log"]));
    let refused = bob.run(&["fork", &thread]);
    assert!(!refused.status.success());

    alice.silt(&["visibility", "team", "--thread", &thread]);
    assert!(eventually(|| bob.log_line(&thread).is_some()), "the thread's own setting opens it up");
}
