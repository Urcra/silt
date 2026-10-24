//! Two users on one Silt server, all real processes: Bob forks Alice's
//! in-progress thread with its conversation, pulls her newer work, and
//! offers his back with merge requests that Alice approves or declines.
// The agents these tests drive are shell-script stand-ins.
#![cfg(unix)]

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
    server: Server,
    _root: tempfile::TempDir,
}

fn team() -> Team {
    let root = tempfile::Builder::new().prefix("silt-forks").tempdir_in("/tmp").expect("temp dir");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let alice = Machine::new_for(root.path(), "alice-laptop", "alice", &origin, &server);
    let bob = Machine::new_for(root.path(), "bob-desktop", "bob", &origin, &server);
    Team { alice, bob, server, _root: root }
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
        &[("FAKE_EXPECT", "Please-add-a-farewell"), ("FAKE_SAY", "Bob here: adding bob"), ("FAKE_LINE", "fn bob() {}")],
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
    assert!(eventually(|| bob.try_stdout(&["diff", &thread]).contains("+// alice was here")));
    assert!(eventually(|| bob.stdout(&["show", &thread]).contains("alice: continuing upstream")));

    let pulled = bob.run_with(&["pull", &fork], &[]);
    let pulled_text = String::from_utf8_lossy(&pulled.stdout);
    assert!(pulled.status.success(), "{}", String::from_utf8_lossy(&pulled.stderr));
    assert!(pulled_text.contains("pulled 1 messages and 1 changed files"), "{pulled_text}");
    assert!(eventually(
        || read(&fork_checkout.join("src/main.rs")) == "// alice was here\nfn main() {}\nfn farewell() {}\nfn bob() {}\n"
    ));
    let show = bob.stdout(&["show", &fork]);
    assert!(show.contains("upstream progress") && show.contains("alice: continuing upstream"), "{show}");
    let own = bob.stdout(&["diff", &fork, "--base", "merge-base"]);
    assert!(own.contains("+fn bob() {}") && !own.contains("+// alice was here"), "only the fork's own work: {own}");
    let whole = bob.stdout(&["diff", &fork]);
    assert!(whole.contains("+fn bob() {}") && whole.contains("+// alice was here"), "since the fork point: {whole}");
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
    alice.silt(&["daemon", "stop"]);
    alice.stdout(&["log"]);
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
    let Team { alice, bob, server, .. } = &team();
    server.admin(&["user", "admin", "alice"]);
    let set = alice.stdout(&["visibility", "private"]);
    assert!(set.contains("is now private"), "{set}");
    alice.silt(&["claude"]);
    let thread = thread_of(alice);
    assert!(eventually(|| alice.stdout(&["auth", "status"]).contains("syncing")));
    // A later thread Bob may see shows his machine has listed what came
    // before it.
    alice.silt(&["claude"]);
    let later = alice
        .stdout(&["log"])
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .find(|id| *id != thread)
        .expect("a second thread")
        .to_owned();
    alice.silt(&["visibility", "org", "--thread", &later]);
    assert!(eventually(|| bob.log_line(&later).is_some()), "Bob's machine has listed since");
    assert!(bob.log_line(&thread).is_none(), "{}", bob.stdout(&["log"]));
    let refused = bob.run(&["fork", &thread]);
    assert!(!refused.status.success());

    alice.silt(&["visibility", "org", "--thread", &thread]);
    assert!(eventually(|| bob.log_line(&thread).is_some()), "the thread's own setting opens it up");
}

#[test]
fn search_and_in_flight_span_the_team() {
    let Team { alice, bob, .. } = &team();
    alice.silt(&["claude"]);
    let thread = thread_of(alice);
    let short = &thread[thread.len() - 12..];
    assert!(eventually(|| bob.try_stdout(&["search", "farewell", "--author", "alice"]).contains(short)));
    let found = bob.stdout(&["search", "farewell", "--file", "src", "--since", "1d", "--status", "active"]);
    assert!(found.contains(short) && found.contains("alice, active"), "{found}");
    assert!(bob.stdout(&["search", "farewell", "--author", "bob"]).contains("no matches"));
    assert!(bob.stdout(&["search", "farewell", "--file", "docs"]).contains("no matches"));

    let flight = bob.stdout(&["in-flight", "src/main.rs"]);
    assert!(flight.contains(short) && flight.contains("alice"), "{flight}");
    assert!(bob.stdout(&["in-flight", "README.md"]).contains("no other thread"));
    assert!(
        alice.stdout(&["in-flight", "src"]).contains("no other thread"),
        "a thread doesn't list itself where it is checked out"
    );
}

#[test]
fn comments_follow_their_lines_and_sync_between_users() {
    let Team { alice, bob, .. } = &team();
    alice.silt(&["claude"]);
    let thread = thread_of(alice);
    let added = alice.stdout(&["comment", &thread, "--file", "src/main.rs", "--lines", "2", "farewell needs a doc comment"]);
    assert!(added.contains("at src/main.rs:2"), "{added}");
    let id = added.split_whitespace().nth(2).expect("comment id").to_owned();

    let main = alice.repo.join("src/main.rs");
    fs::write(&main, format!("// one\n// two\n{}", read(&main))).expect("edit above the comment");
    assert!(eventually(|| alice.stdout(&["comments", &thread]).contains("src/main.rs:4")));

    assert!(
        eventually(|| bob.try_stdout(&["history", &thread]).contains("[0] user: Please add a farewell line")),
        "{}",
        bob.try_stdout(&["history", &thread])
    );
    assert!(eventually(|| bob.stdout(&["comments", &thread]).contains("farewell needs a doc comment")));
    let from_bob = bob.stdout(&["comment", &thread, "--entry", "0", "good prompt"]);
    assert!(from_bob.contains("at entry 0"), "{from_bob}");
    let bobs = from_bob.split_whitespace().nth(2).expect("comment id").to_owned();
    assert!(eventually(|| alice.stdout(&["comments", &thread]).contains("bob  entry 0")));

    fs::write(&main, "// one\n// two\nfn main() {}\n").expect("delete the commented line");
    assert!(eventually(|| alice.stdout(&["comments", &thread]).contains("outdated")));
    alice.silt(&["comments", &thread, "--resolve", &bobs]);
    assert!(eventually(|| bob
        .stdout(&["comments", &thread])
        .lines()
        .any(|line| line.starts_with(&bobs) && line.contains("[resolved]"))));
    assert!(alice.stdout(&["comments", &thread]).contains(&id));
}

#[test]
fn a_private_fork_is_shared_only_through_its_merge_request() {
    let Team { alice, bob, .. } = &team();
    alice.silt(&["claude"]);
    let thread = thread_of(alice);
    assert!(eventually(|| bob.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))));
    let forked = bob.silt_with(
        &["fork", "--private", &thread],
        &[("FAKE_SAY", "Bob tries something quietly"), ("FAKE_LINE", "fn quiet() {}")],
    );
    let fork_checkout = checkout_from(&forked.stderr);
    let fork = bob
        .stdout(&["log"])
        .lines()
        .find(|line| line.contains(&format!("fork of {thread}")))
        .and_then(|line| line.split_whitespace().next())
        .expect("the fork is listed")
        .to_owned();
    let fork_main = fork_checkout.join("src/main.rs");
    fs::write(&fork_main, read(&fork_main) + "fn quieter() {}\n").expect("Bob keeps working");
    assert!(eventually(|| bob.stdout(&["diff", &fork]).contains("+fn quieter() {}")));

    // Pushed after the fork's work, so once Alice has it she has had the
    // chance to see the fork too.
    bob.silt(&["comment", &thread, "--entry", "0", "seen after the fork"]);
    assert!(eventually(|| alice.stdout(&["comments", &thread]).contains("seen after the fork")));
    assert!(alice.log_line(&fork).is_none(), "{}", alice.stdout(&["log"]));
    assert!(alice.stdout(&["search", "quietly"]).contains("no matches"));

    let opened = bob.silt(&["mr", "create", "--thread", &fork, "--title", "Quiet work"]);
    let mr = mr_id(&opened.stdout);
    assert!(eventually(|| alice.try_stdout(&["mr", "list"]).contains(&mr)), "the merge request shares the fork");
    let details = alice.stdout(&["mr", "show", &mr]);
    assert!(details.contains("+fn quieter() {}") && details.contains("Bob tries something quietly"), "{details}");
    alice.silt(&["mr", "approve", &mr]);
    assert!(eventually(|| read(&alice.repo.join("src/main.rs")).contains("fn quieter() {}")));
    assert!(alice.log_line(&fork).is_none(), "the fork itself stays private");
}

#[test]
fn the_inbox_hears_about_merge_requests_comments_and_mentions() {
    let Team { alice, bob, .. } = &team();
    alice.silt(&["claude"]);
    let thread = thread_of(alice);
    assert!(eventually(|| bob.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))));
    bob.silt_with(&["fork", &thread], &[("FAKE_EXPECT", "Please-add-a-farewell"), ("FAKE_LINE", "fn bob() {}")]);
    let fork = bob
        .stdout(&["log"])
        .lines()
        .find(|line| line.contains(&format!("fork of {thread}")))
        .and_then(|line| line.split_whitespace().next())
        .expect("the fork is listed")
        .to_owned();
    let opened = bob.silt(&["mr", "create", "--thread", &fork, "--title", "Add bob, @alice please look"]);
    let mr = mr_id(&opened.stdout);

    assert!(
        eventually(|| alice.stdout(&["inbox"]).contains("bob asks to merge \"Add bob, @alice please look\"")),
        "{}",
        alice.stdout(&["inbox"])
    );
    let inbox = alice.stdout(&["inbox"]);
    assert!(inbox.contains("bob mentioned you"), "{inbox}");
    // The inbox comes from the server, the agent's notice once Alice's
    // daemon has pulled the merge request, which can be a moment later.
    let session = first_session(alice);
    assert!(
        eventually(|| {
            let context = prompt_hook(alice, &session);
            context.contains("merge request") && context.contains("Add bob")
        }),
        "Alice's agent hears about it"
    );
    assert!(bob.stdout(&["inbox"]).contains("nothing new"), "nobody hears about their own work");

    // Asking for changes keeps it open and tells the fork's owner and agent.
    alice.silt(&["mr", "request-changes", &mr, "please", "add", "a", "test"]);
    assert!(
        eventually(|| bob.stdout(&["inbox"]).contains("alice asks for changes to your merge request")),
        "{}",
        bob.stdout(&["inbox"])
    );
    assert!(eventually(|| bob.stdout(&["mr", "list"]).contains("changes requested: please add a test")));
    let fork_session = first_session(bob);
    assert!(eventually(|| prompt_hook(bob, &fork_session).contains("asked for changes")), "Bob's agent hears what to change");

    alice.silt(&["mr", "approve", &mr]);
    assert!(eventually(|| bob.stdout(&["inbox"]).contains("alice approved and merged your merge request")));
    let praised = alice.stdout(&["comment", &thread, "--entry", "0", "nice work @bob"]);
    let praise = praised.split_whitespace().nth(2).expect("the comment's id").to_owned();
    assert!(eventually(|| bob.stdout(&["inbox"]).contains("alice mentioned you")));
    bob.silt(&["comment", &thread, "--entry", "0", "thanks"]);
    assert!(eventually(|| alice.stdout(&["inbox"]).contains("bob commented on thread")));

    // A reply goes under the comment and tells its author.
    assert!(eventually(|| bob.try_stdout(&["comments", &thread]).contains(&praise)));
    bob.silt(&["comment", &thread, "--reply", &praise, "glad to help"]);
    assert!(eventually(|| alice.stdout(&["inbox"]).contains("bob replied on thread")), "{}", alice.stdout(&["inbox"]));
    assert!(eventually(|| alice.stdout(&["comments", &thread]).contains("↳")));
    let listed = alice.stdout(&["comments", &thread]);
    assert!(listed.contains("bob: glad to help"), "{listed}");

    alice.silt(&["inbox", "read", "--all"]);
    assert!(alice.stdout(&["inbox"]).contains("nothing new"));
    assert!(alice.stdout(&["inbox", "--all"]).contains("bob asks to merge"));
}

#[test]
fn deleted_threads_hide_everywhere_and_purged_ones_are_gone() {
    let Team { alice, bob, .. } = &team();
    alice.silt(&["claude"]);
    let thread = thread_of(alice);
    assert!(eventually(|| bob.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))));
    bob.silt_with(&["fork", &thread], &[("FAKE_EXPECT", "Please-add-a-farewell"), ("FAKE_SAY", "Bob keeps this")]);
    let fork = bob
        .stdout(&["log"])
        .lines()
        .find(|line| line.contains(&format!("fork of {thread}")))
        .and_then(|line| line.split_whitespace().next())
        .expect("the fork is listed")
        .to_owned();

    let refused = bob.run(&["thread", "delete", &thread]);
    assert!(!refused.status.success(), "only the owner deletes a thread");
    assert!(alice.stdout(&["thread", "delete", &thread]).contains("is in the trash"));
    assert!(alice.log_line(&thread).is_none());
    assert!(alice.stdout(&["thread", "trash"]).contains(&thread));
    assert!(eventually(|| bob.log_line(&thread).is_none()), "Bob's machine hides it too");
    assert!(!alice.run(&["thread", "purge", &fork]).status.success(), "purging needs the trash and ownership");

    alice.silt(&["thread", "restore", &thread]);
    assert!(alice.log_line(&thread).is_some());
    assert!(eventually(|| bob.log_line(&thread).is_some()), "and shows it again once restored");

    alice.silt(&["thread", "delete", &thread]);
    assert!(alice.stdout(&["thread", "purge", &thread]).contains("purged for good"));
    assert!(alice.stdout(&["thread", "trash"]).contains("the trash is empty"));
    assert!(eventually(|| bob.log_line(&thread).is_none()), "Bob's machine drops it");
    let show = bob.stdout(&["show", &fork]);
    assert!(
        show.contains("> Please add a farewell line") && show.contains("> Bob keeps this"),
        "the fork keeps what it built on: {show}"
    );

    alice.silt(&["claude"]);
    let second = alice
        .stdout(&["log"])
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .find(|id| *id != fork)
        .expect("a second thread")
        .to_owned();
    assert!(eventually(|| bob.log_line(&second).is_some_and(|line| line.contains("2 msgs"))));
    alice.silt(&["thread", "delete", &second]);
    alice.silt(&["thread", "purge", &second]);
    assert!(eventually(|| bob.log_line(&second).is_none()));
    bob.silt(&["daemon", "stop"]);
    // Once Alice has what Bob's restarted machine sent, it would have sent
    // the purged thread back too.
    bob.silt(&["comment", &fork, "--entry", "0", "back after a restart"]);
    assert!(eventually(|| alice.stdout(&["comments", &fork]).contains("back after a restart")));
    assert!(bob.log_line(&second).is_none(), "a restarted device doesn't bring it back");
    assert!(alice.log_line(&second).is_none());
}

#[test]
fn the_owners_agent_hears_teammates_drafts() {
    let Team { alice, bob, .. } = &team();
    alice.silt(&["claude"]);
    let thread = thread_of(alice);
    assert!(eventually(|| bob.log_line(&thread).is_some()));
    let url = bob.stdout(&["open", "--no-browser"]);
    let (base, token) = common::web_ui(&url);
    let full = alice.stdout(&["history", &thread]);
    let full_id = full.split_whitespace().nth(1).expect("the thread's full id").trim_end_matches(':').to_owned();
    let posted = Command::new("curl")
        .args(["-s", "-X", "POST", "-H", "Content-Type: application/json", "-H"])
        .arg(format!("Authorization: Bearer {token}"))
        .arg("--data")
        .arg(r#"{"file":"src/main.rs","draft":"also handle the empty case"}"#)
        .arg(format!("{base}/api/threads/{full_id}/presence"))
        .output()
        .expect("curl runs");
    assert!(posted.status.success(), "{}", String::from_utf8_lossy(&posted.stderr));
    let reply = String::from_utf8_lossy(&posted.stdout);
    assert!(reply.contains("people"), "{reply} for {full_id}");
    let context = prompt_hook(alice, &first_session(alice));
    assert!(context.contains("bob: also handle the empty case"), "Alice's agent hears Bob's draft: {context}");
}

fn mirrored_family(server: &Server) -> Option<std::ffi::OsString> {
    fs::read_dir(server.data.join("git"))
        .ok()?
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().ends_with(".git"))
        .map(|entry| entry.file_name())
}

/// Runs git against the server's mirror of the only family, as `machine`.
fn mirror_git(machine: &Machine, server: &Server, args: &[&str]) -> std::process::Output {
    let sign_ins: serde_json::Value =
        serde_json::from_str(&read(&machine.silt_home.join("servers.json"))).expect("the machine is signed in");
    let config = &sign_ins[0];
    let family = mirrored_family(server).expect("the mirror exists");
    let url = format!("{}/git/{}", server.url, family.to_string_lossy());
    let header = format!("http.extraHeader=Authorization: Bearer {}", config["token"].as_str().expect("token"));
    let protocol = format!("http.extraHeader=silt-protocol: {}", silt_proto::PROTOCOL);
    let mut command = Command::new("git");
    command.args(["-c", &header, "-c", &protocol]).current_dir(&machine.repo);
    for arg in args {
        command.arg(if *arg == "URL" { url.as_str() } else { arg });
    }
    command.output().expect("git runs")
}

#[test]
fn the_git_mirror_shows_and_takes_only_what_each_person_may() {
    let Team { alice, bob, server, .. } = &team();
    alice.silt(&["claude"]);
    let thread = thread_of(alice);
    fs::write(alice.repo.join("notes.txt"), "mirrored\n").expect("write");
    common::git(&alice.repo, &["add", "notes.txt"]);
    common::git(&alice.repo, &["-c", "user.name=a", "-c", "user.email=a@example.com", "commit", "-q", "-m", "notes"]);
    let thread_ref = |listing: &str| listing.lines().any(|line| line.contains("refs/silt/threads/"));
    assert!(eventually(|| mirrored_family(server).is_some()), "the commit reaches the mirror");
    assert!(eventually(|| thread_ref(&String::from_utf8_lossy(&mirror_git(alice, server, &["ls-remote", "URL"]).stdout))));
    assert!(
        eventually(|| thread_ref(&String::from_utf8_lossy(&mirror_git(bob, server, &["ls-remote", "URL"]).stdout))),
        "bob sees alice's thread"
    );
    let listing = String::from_utf8_lossy(&mirror_git(alice, server, &["ls-remote", "URL"]).stdout).into_owned();
    let alice_ref = listing
        .lines()
        .find_map(|line| line.split_whitespace().nth(1).filter(|name| name.starts_with("refs/silt/threads/")))
        .expect("alice's thread ref")
        .to_owned();

    let pushed = mirror_git(bob, server, &["push", "URL", &format!("HEAD:{alice_ref}")]);
    assert!(!pushed.status.success(), "bob may not write alice's thread");
    let deleted = mirror_git(alice, server, &["push", "URL", &format!(":{alice_ref}")]);
    assert!(!deleted.status.success(), "refs can't be deleted");

    alice.silt(&["visibility", "private", "--thread", &thread]);
    assert!(
        eventually(|| !thread_ref(&String::from_utf8_lossy(&mirror_git(bob, server, &["ls-remote", "URL"]).stdout))),
        "a private thread's ref is hidden from others"
    );
    assert!(thread_ref(&String::from_utf8_lossy(&mirror_git(alice, server, &["ls-remote", "URL"]).stdout)));
}
