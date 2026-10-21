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

    // What belongs to no thread comes from both, each saying where from.
    let servers = |path: &str| -> HashSet<String> {
        laptop
            .web("GET", path, None)
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["server"].as_str().map(str::to_owned))
            .collect()
    };
    let both: HashSet<String> = [world.work.url.clone(), world.oss.url.clone()].into();
    assert!(eventually(|| servers("/api/projects") == both), "the project as each server has it: {:?}", servers("/api/projects"));
    let asked = format!("/api/projects?server={}", world.oss.url);
    assert_eq!(servers(&asked), [world.oss.url.clone()].into(), "one server, when named");
}

#[test]
fn a_project_kept_on_this_machine_syncs_nowhere_until_moved_to_a_server() {
    let world = world();
    let laptop = &world.laptop;
    let said = laptop.stdout(&["project", "home", "local"]);
    assert!(said.contains("stay on this machine only"), "{said}");
    let kept = record(laptop);
    let line = laptop.log_line(&kept).expect("listed here");
    assert!(line.contains("this machine only"), "{line}");

    std::thread::sleep(Duration::from_secs(4));
    assert!(!threads(&world.work_peer).contains(&kept), "nothing reached the default server");
    assert!(!threads(&world.oss_peer).contains(&kept), "nothing reached the other server");
    let refused = laptop.run(&["thread", "share", &kept, "someone"]);
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success() && said.contains("stays on this machine only"), "a thread kept here isn't shared: {said}");

    let said = laptop.stdout(&["project", "home", &world.oss.url]);
    assert!(said.contains("1 threads kept on this machine now sync there"), "{said}");
    assert!(eventually(|| threads(&world.oss_peer).contains(&kept)), "the moved thread reaches the server chosen");
    assert!(!laptop.log_line(&kept).expect("listed here").contains("this machine only"));
    std::thread::sleep(Duration::from_secs(4));
    assert!(!threads(&world.work_peer).contains(&kept), "and only that server");
}
