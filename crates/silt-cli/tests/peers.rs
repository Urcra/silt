//! Two agents in one clone each record their own work (ADR 0086), and know about each other (ADR 0087).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Output, Stdio};

use common::{Machine, Server, eventually, origin};

struct World {
    laptop: Machine,
    _server: Server,
    root: tempfile::TempDir,
}

fn world(daemon: bool) -> World {
    let root = common::temp_root("silt-peers");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    if daemon {
        laptop.silt(&["daemon", "start"]);
    } else {
        laptop.silt(&["daemon", "stop"]);
    }
    World { laptop, _server: server, root }
}

fn in_tool(machine: &Machine, args: &[&str], marker: &Path, envs: &[(&str, &str)]) -> Child {
    let marker_env = marker.to_string_lossy().into_owned();
    let mut envs = envs.to_vec();
    envs.push(("FAKE_IN_TOOL", &marker_env));
    let child = machine.command(args, &envs).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().expect("starts");
    assert!(eventually(|| marker.exists()), "the agent reaches its tool");
    child
}

fn let_go(marker: &Path) {
    fs::write(PathBuf::from(format!("{}.go", marker.display())), "").expect("let the agent go");
}

fn finished(child: Child) -> Output {
    let output = child.wait_with_output().expect("waits");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("could not"), "everything was recorded: {stderr}");
    output
}

fn thread_named(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let line = stderr.lines().find(|line| line.contains("thread ")).unwrap_or_else(|| panic!("names its thread: {stderr}"));
    line.split("thread ").nth(1).and_then(|rest| rest.split([' ', ',']).next()).expect("thread id").to_owned()
}

/// Agent A is held in its tool, B starts in the same clone and is held in
/// its own, then A and B each finish their edit in turn.
fn two_agents_in_one_clone_are_both_recorded(daemon: bool) {
    let world = world(daemon);
    let laptop = &world.laptop;
    let envs: &[(&str, &str)] = if daemon { &[] } else { &[("SILT_DAEMONLESS_CAPTURE", "1")] };
    let (marker_a, marker_b) = (world.root.path().join("in-tool-a"), world.root.path().join("in-tool-b"));
    let a = in_tool(laptop, &["claude"], &marker_a, envs);
    let b = in_tool(laptop, &["claude"], &marker_b, envs);
    if daemon {
        // While both are in a tool call, an edit nobody's hook names could
        // be either's.
        fs::write(laptop.repo.join("NOTES.md"), "by hand or by an agent\n").expect("writes");
        let both_took_it = || {
            let log = laptop.stdout(&["log"]);
            let threads: Vec<&str> = log.lines().filter_map(|line| line.split_whitespace().next()).collect();
            threads.len() == 2 && threads.iter().all(|thread| laptop.stdout(&["show", thread]).contains("NOTES.md"))
        };
        assert!(eventually(both_took_it), "both threads take the edit in");
    }
    let_go(&marker_a);
    let a = finished(a);
    let_go(&marker_b);
    let b = finished(b);
    let (thread_a, thread_b) = (thread_named(&a.stderr), thread_named(&b.stderr));
    assert_ne!(thread_a, thread_b);
    let (show_a, show_b) = (laptop.stdout(&["show", &thread_a]), laptop.stdout(&["show", &thread_b]));
    for show in [&show_a, &show_b] {
        assert!(show.contains("Done: farewell added."), "{show}");
        assert!(show.contains("    ✎ src/main.rs +17 -0"), "each agent's own edit is under its tool call: {show}");
    }
    assert!(!show_a.contains("by another agent"), "B's edit came after A was done: {show_a}");
    let others = show_b.lines().find(|line| line.contains("by another agent")).unwrap_or_else(|| panic!("{show_b}"));
    assert!(others.starts_with("  ✎ by another agent (claude-code ") && others.ends_with("): src/main.rs +17 -0"), "{others}");
    if daemon {
        for show in [&show_a, &show_b] {
            let unclear = show.lines().find(|line| line.contains("by one of these agents")).unwrap_or_else(|| panic!("{show}"));
            assert!(unclear.ends_with("NOTES.md +23 -0"), "{unclear}");
        }
    }
    let farewells = |diff: &str| diff.lines().filter(|line| *line == "+fn farewell() {}").count();
    let diff_b = laptop.stdout(&["diff", &thread_b]);
    assert_eq!(farewells(&diff_b), 2, "B's tree has both edits: {diff_b}");
    let without_others = laptop.stdout(&["diff", &thread_b, "--hide-others"]);
    assert_eq!(farewells(&without_others), 1, "A's edit is left out: {without_others}");
    assert_eq!(without_others.contains("NOTES.md"), daemon, "an unclear edit may be B's own, so it stays");
    let main = fs::read_to_string(laptop.repo.join("src/main.rs")).expect("reads");
    assert_eq!(main.matches("fn farewell() {}").count(), 2, "{main}");
}

#[test]
fn two_agents_in_one_clone_are_both_recorded_with_a_daemon() {
    two_agents_in_one_clone_are_both_recorded(true);
}

#[test]
fn two_agents_in_one_clone_are_both_recorded_without_a_daemon() {
    two_agents_in_one_clone_are_both_recorded(false);
}
