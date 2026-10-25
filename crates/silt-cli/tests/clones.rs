//! The daemon remembers the clones it worked in across restarts, so the web
//! UI can start agents in new threads of them.

mod common;

use std::time::Duration;

use common::{Machine, Server, origin};
use silt_daemon::client;
use silt_daemon::protocol::{AgentChoices, Request, Response};

fn known_clones(machine: &Machine) -> Vec<std::path::PathBuf> {
    match client::send(
        &silt_daemon::paths::socket_path_in(&machine.silt_home),
        &Request::AcpAgents,
        Some(Duration::from_secs(10)),
    )
    .expect("answers")
    {
        Response::Ok { data } => serde_json::from_value::<AgentChoices>(data)
            .expect("agent choices")
            .clones
            .into_iter()
            .map(|clone| clone.path)
            .collect(),
        Response::Error { message } => panic!("{message}"),
    }
}

#[test]
fn clones_are_remembered_across_restarts() {
    let root = common::temp_root("silt-clones");
    let origin = origin(root.path());
    let _server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &_server);
    laptop.silt(&["claude"]);
    assert_eq!(known_clones(&laptop), vec![laptop.repo.clone()]);

    laptop.silt(&["daemon", "stop"]);
    laptop.silt(&["log"]);
    assert_eq!(known_clones(&laptop), vec![laptop.repo.clone()], "a restarted daemon still knows the clone");
}

#[test]
fn only_one_daemon_runs_per_data_directory() {
    let root = common::temp_root("silt-one-daemon");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    laptop.silt(&["log"]);
    let second = laptop.run(&["daemon", "run"]);
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("another Silt daemon already runs"));
    assert!(laptop.stdout(&["daemon", "status"]).starts_with("running"), "the first one is untouched");
}
