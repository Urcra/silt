//! One machine signed in to two servers (ADR 0032): each thread syncs with
//! its own server, and nothing of it reaches the other.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use common::{Machine, Server, eventually, origin};

/// Fields drop in order: the machines stop their daemons before the
/// directories holding their sockets go away.
struct World {
    laptop: Machine,
    work_peer: Machine,
    oss_peer: Machine,
    work: Server,
    oss: Server,
    _root: tempfile::TempDir,
}

fn world() -> World {
    let root = tempfile::Builder::new().prefix("silt-servers").tempdir_in("/tmp").expect("temp dir");
    let origin = origin(root.path());
    let work = Server::start(&root.path().join("work"));
    let oss = Server::start(&root.path().join("oss"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &work);
    let code = oss.pairing_code();
    laptop.silt(&["auth", "login", "--server", &oss.url, "--code", &code, "--name", "laptop"]);
    let work_peer = Machine::new(root.path(), "work-peer", &origin, &work);
    let oss_peer = Machine::new(root.path(), "oss-peer", &origin, &oss);
    World { laptop, work_peer, oss_peer, work, oss, _root: root }
}

fn threads(machine: &Machine) -> HashSet<String> {
    machine.try_stdout(&["log"]).lines().filter_map(|line| line.split_whitespace().next().map(str::to_owned)).collect()
}

/// Records a thread with the fake agent and returns it.
fn record(machine: &Machine) -> String {
    let before = threads(machine);
    machine.silt(&["claude"]);
    threads(machine).difference(&before).next().expect("a new thread").clone()
}

#[test]
fn each_thread_syncs_with_its_own_server_only() {
    let world = world();
    let laptop = &world.laptop;
    let status = laptop.stdout(&["auth", "status"]);
    let (work_at, oss_at) = (status.find(&world.work.url), status.find(&world.oss.url));
    assert!(work_at.is_some_and(|work| oss_at.is_some_and(|oss| work < oss)), "both, the first the default: {status}");

    let at_work = record(laptop);
    assert!(eventually(|| threads(&world.work_peer).contains(&at_work)), "the work server has the thread");

    // With the work server gone, the open-source one is the default, so the
    // next thread goes there. Signing in to work again doesn't move it.
    laptop.silt(&["auth", "logout", "--server", &world.work.url]);
    let in_the_open = record(laptop);
    let code = world.work.pairing_code();
    laptop.silt(&["auth", "login", "--server", &world.work.url, "--code", &code, "--name", "laptop"]);
    assert!(eventually(|| threads(&world.oss_peer).contains(&in_the_open)), "the open-source server has the second thread");

    std::thread::sleep(Duration::from_secs(4));
    assert!(!threads(&world.oss_peer).contains(&at_work), "the work thread never reached the open-source server");
    assert!(!threads(&world.work_peer).contains(&in_the_open), "the open-source thread never reached work");
}
