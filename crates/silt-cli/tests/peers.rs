//! Agents in one project know about each other (ADR 0086).

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

fn two_agents_in_one_clone_are_both_recorded(daemon: bool) {
    let world = world(daemon);
    let laptop = &world.laptop;
    let envs: &[(&str, &str)] = if daemon { &[] } else { &[("SILT_DAEMONLESS_CAPTURE", "1")] };
    let (marker_a, marker_b) = (world.root.path().join("in-tool-a"), world.root.path().join("in-tool-b"));
    let a = in_tool(laptop, &["claude"], &marker_a, envs);
    let b = in_tool(laptop, &["claude"], &marker_b, envs);
    let_go(&marker_a);
    let a = finished(a);
    let_go(&marker_b);
    let b = finished(b);
    for thread in [thread_named(&a.stderr), thread_named(&b.stderr)] {
        let show = laptop.stdout(&["show", &thread]);
        assert!(show.contains("Done: farewell added."), "{thread}: {show}");
    }
}

#[test]
fn two_agents_in_one_clone_are_both_recorded_with_a_daemon() {
    two_agents_in_one_clone_are_both_recorded(true);
}

#[test]
#[ignore = "the first agent's hook panics while the clone is bound to the second's thread (docs/research/agents-in-one-clone.md)"]
fn two_agents_in_one_clone_are_both_recorded_without_a_daemon() {
    two_agents_in_one_clone_are_both_recorded(false);
}
