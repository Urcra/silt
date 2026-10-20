//! The daemon remembers the clones it worked in across restarts, so the web
//! UI can start agents in new threads of them.

mod common;

use std::time::Duration;

use common::{Machine, Server, origin};
use silt_daemon::client;
use silt_daemon::protocol::{AgentChoices, Request, Response};

fn known_clones(machine: &Machine) -> Vec<std::path::PathBuf> {
    match client::send(&machine.silt_home.join("daemon.sock"), &Request::AcpAgents, Duration::from_secs(10)).expect("answers") {
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
    let root = tempfile::Builder::new().prefix("silt-clones").tempdir_in("/tmp").expect("temp dir");
    let origin = origin(root.path());
    let _server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &_server);
    laptop.silt(&["claude"]);
    assert_eq!(known_clones(&laptop), vec![laptop.repo.clone()]);

    laptop.silt(&["daemon", "stop"]);
    laptop.silt(&["log"]);
    assert_eq!(known_clones(&laptop), vec![laptop.repo.clone()], "a restarted daemon still knows the clone");
}
