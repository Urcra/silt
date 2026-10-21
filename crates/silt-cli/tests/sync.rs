//! Two machines and a Silt server, all real processes: a thread recorded on
//! a laptop continues on a home machine with its conversation and files.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::{Machine, Server, checkout_from, eventually, git, origin, thread_of};

/// Fields drop in order: the machines stop their daemons before the
/// directories holding their sockets go away.
struct World {
    laptop: Machine,
    home: Machine,
    server: Server,
    root: tempfile::TempDir,
}

/// An origin with one pushed commit, and a laptop clone with one more that
/// only the laptop has.
fn world() -> World {
    let root = tempfile::Builder::new().prefix("silt-sync").tempdir_in("/tmp").expect("temp dir");
    let origin = origin(root.path());

    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let home = Machine::new(root.path(), "home", &origin, &server);
    fs::write(laptop.repo.join("src/lib.rs"), "pub fn wip() {}\n").expect("write");
    git(&laptop.repo, &["add", "."]);
    git(&laptop.repo, &["commit", "-q", "-m", "wip, not pushed"]);
    World { root, server, laptop, home }
}

#[test]
fn a_thread_continues_on_another_machine() {
    let world = world();
    let (laptop, home) = (&world.laptop, &world.home);
    laptop.silt(&["claude"]);
    let thread = thread_of(laptop);
    assert!(fs::read_to_string(laptop.repo.join("src/main.rs")).expect("read").contains("fn farewell"));

    assert!(
        eventually(|| home.log_line(&thread).is_some_and(|line| line.contains("remote") && line.contains("2 msgs"))),
        "the home machine sees the laptop's thread: {}",
        home.stdout(&["log"])
    );
    let resumed = home.silt(&["resume", &thread]);
    let checkout = checkout_from(&resumed.stderr);
    assert!(checkout.starts_with(&home.repo));
    let main = fs::read_to_string(checkout.join("src/main.rs")).expect("read");
    assert_eq!(main, "fn main() {}\nfn farewell() {}\nfn home() {}\n");
    assert_eq!(
        fs::read_to_string(checkout.join("src/lib.rs")).expect("the unpushed commit came in a bundle"),
        "pub fn wip() {}\n"
    );
    assert_eq!(fs::read_to_string(home.repo.join("src/main.rs")).expect("read"), "fn main() {}\n");
    let wip = git(&laptop.repo, &["rev-parse", "HEAD"]).trim().to_owned();
    let mirror = fs::read_dir(world.server.data.join("git"))
        .expect("the server keeps a git mirror")
        .flatten()
        .next()
        .expect("one repository per family")
        .path();
    let mirrored = git(&mirror, &["for-each-ref", "--format=%(objectname)", "refs/silt/threads/"]);
    assert!(mirrored.contains(&wip), "the unpushed commit is on the server's mirror: {mirrored}");
    let fetched = git(&home.repo, &["for-each-ref", "--format=%(objectname)", "refs/silt/mirror/"]);
    assert!(fetched.contains(&wip), "the home machine fetched it from the mirror: {fetched}");

    assert!(eventually(|| home.stdout(&["diff", &thread]).contains("+fn home() {}")));
    let show = home.stdout(&["show", &thread]);
    assert!(show.contains("> Please add a farewell line to main"), "{show}");
    assert!(show.contains("> Continuing at home: add a home line"), "{show}");
    let token = "ghp_abcdefghijklmnopqrstuvwxyz0123";
    assert!(
        show.contains(&format!("I used {} to fetch it", silt_redact::placeholder(token))),
        "the home machine doesn't know the token: {show}"
    );
    assert!(laptop.stdout(&["show", &thread]).contains("ghp_abcdefghijklmnopqrstuvwxyz0123"));
    assert!(!dir_contains(&world.server.data, b"ghp_abcdefghijklmnopqrstuvwxyz0123"), "secrets stay off the server");
    assert!(home.log_line(&thread).is_some_and(|line| line.contains("here")));

    assert!(
        eventually(|| laptop.stdout(&["show", &thread]).contains("Continuing at home")),
        "the laptop sees what happened at home"
    );
    assert!(laptop.log_line(&thread).is_some_and(|line| line.contains("moved")), "{}", laptop.stdout(&["log"]));
    let log = laptop.stdout(&["log"]);
    assert!(
        log.lines().any(|line| line.contains(&format!("fork of {thread}")) && line.contains("here")),
        "the laptop's checkout continues in a fork: {log}"
    );
    assert!(!fs::read_to_string(laptop.repo.join("src/main.rs")).expect("read").contains("fn home"));

    // A machine that knows the token from its secrets file sees it again.
    let desk_home = world.root.path().join("desk").join("silt");
    fs::create_dir_all(&desk_home).expect("mkdir");
    fs::write(desk_home.join("secrets"), format!("# deploy token\n{token}\n")).expect("write");
    let desk = Machine::new(world.root.path(), "desk", &world.root.path().join("origin.git"), &world.server);
    assert!(
        eventually(|| desk.try_stdout(&["show", &thread]).contains(&format!("I used {token} to fetch it"))),
        "{}",
        desk.try_stdout(&["show", &thread])
    );
    assert!(!dir_contains(&world.server.data, token.as_bytes()), "secrets stay off the server");

    // Someone else who happens to know the token gets nothing filled in: a
    // placeholder from another person may be planted (ADR 0016).
    let eve_home = world.root.path().join("eve").join("silt");
    fs::create_dir_all(&eve_home).expect("mkdir");
    fs::write(eve_home.join("secrets"), format!("{token}\n")).expect("write");
    let eve = Machine::new_for(world.root.path(), "eve", "eve", &world.root.path().join("origin.git"), &world.server);
    assert!(
        eventually(|| eve.try_stdout(&["show", &thread]).contains(&silt_redact::placeholder(token))),
        "{}",
        eve.try_stdout(&["show", &thread])
    );
    assert!(!eve.stdout(&["show", &thread]).contains(token));
}

#[test]
fn sync_recovers_from_server_restarts_and_rollbacks() {
    let mut world = world();
    let backup = world.root.path().join("server-backup");
    world.laptop.silt(&["claude"]);
    let thread = thread_of(&world.laptop);
    assert!(eventually(|| world.home.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))));

    world.server.stop();
    copy_dir(&world.server.data, &backup);
    world.server.run();
    let status = world.laptop.stdout(&["auth", "status"]);
    assert!(status.contains(&world.server.url), "{status}");

    world.home.silt(&["resume", &thread]);
    assert!(eventually(|| world.laptop.stdout(&["show", &thread]).contains("Continuing at home")));

    world.server.stop();
    fs::remove_dir_all(&world.server.data).expect("remove server data");
    copy_dir(&backup, &world.server.data);
    world.server.run();
    let machine = Machine {
        silt_home: world.root.path().join("fresh"),
        claude_dir: world.root.path().join("fresh-claude"),
        repo: world.home.repo.clone(),
        fake: world.home.fake.clone(),
        agents: world.home.agents.clone(),
    };
    fs::create_dir_all(&machine.claude_dir).expect("mkdir");
    let code = world.server.pairing_code();
    machine.silt(&["auth", "login", "--server", &world.server.url, "--code", &code, "--name", "fresh"]);
    assert!(
        eventually(|| machine.log_line(&thread).is_some_and(|line| line.contains("3 msgs"))),
        "after a rollback the clients push what the server lost: {}",
        machine.stdout(&["log"])
    );
}

fn dir_contains(dir: &Path, needle: &[u8]) -> bool {
    fs::read_dir(dir).expect("read dir").flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            dir_contains(&path, needle)
        } else {
            fs::read(&path).is_ok_and(|bytes| bytes.windows(needle.len()).any(|window| window == needle))
        }
    })
}

fn copy_dir(from: &Path, to: &Path) {
    let status = Command::new("cp").arg("-a").arg(from).arg(to).status().expect("cp runs");
    assert!(status.success());
}

#[test]
fn resume_here_uses_the_current_clone() {
    let world = world();
    let (laptop, home) = (&world.laptop, &world.home);
    laptop.silt(&["claude"]);
    let thread = thread_of(laptop);
    assert!(eventually(|| home.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))));

    fs::write(home.repo.join("scratch.txt"), "untracked\n").expect("write");
    let refused = home.run(&["resume", "--here", &thread]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("clean working tree"));
    assert!(!laptop.stdout(&["log"]).contains("moved"), "a refused resume leaves the lease alone");
    fs::remove_file(home.repo.join("scratch.txt")).expect("remove");

    let resumed = home.silt(&["resume", "--here", &thread]);
    assert_eq!(checkout_from(&resumed.stderr), home.repo);
    assert_eq!(
        fs::read_to_string(home.repo.join("src/main.rs")).expect("read"),
        "fn main() {}\nfn farewell() {}\nfn home() {}\n"
    );
    assert_eq!(git(&home.repo, &["log", "-1", "--format=%s"]).trim(), "wip, not pushed");
}

#[test]
fn a_client_restored_from_a_backup_recovers_its_work_in_a_fork() {
    let world = world();
    let (laptop, home) = (&world.laptop, &world.home);
    laptop.silt(&["claude"]);
    let thread = thread_of(laptop);
    assert!(eventually(|| home.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))));

    laptop.silt(&["daemon", "stop"]);
    let backup = world.root.path().join("laptop-backup");
    copy_dir(&laptop.silt_home, &backup);
    laptop.silt(&["daemon", "start"]);
    let main = laptop.repo.join("src/main.rs");
    fs::write(&main, fs::read_to_string(&main).expect("read") + "fn synced() {}\n").expect("write");
    assert!(eventually(|| home.try_stdout(&["show", &thread]).contains("farewell")
        && laptop.stdout(&["diff", &thread]).contains("+fn synced() {}")));
    assert!(eventually(|| {
        let output = Command::new(env!("CARGO_BIN_EXE_silt"))
            .args(["auth", "status"])
            .env("SILT_HOME", &laptop.silt_home)
            .output()
            .expect("runs");
        output.status.success()
    }));
    std::thread::sleep(std::time::Duration::from_secs(2));

    laptop.silt(&["daemon", "stop"]);
    fs::remove_dir_all(&laptop.silt_home).expect("remove");
    copy_dir(&backup, &laptop.silt_home);
    laptop.silt(&["daemon", "start"]);
    fs::write(&main, fs::read_to_string(&main).expect("read") + "fn after_restore() {}\n").expect("write");

    assert!(
        eventually(|| laptop.stdout(&["auth", "status"]).contains("recovery thread")),
        "{}",
        laptop.stdout(&["auth", "status"])
    );
    let log = laptop.stdout(&["log"]);
    let recovery = log
        .lines()
        .find(|line| line.contains(&format!("fork of {thread}")))
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or_else(|| panic!("a recovery fork is listed: {log}"))
        .to_owned();
    assert!(
        eventually(|| laptop.stdout(&["diff", &recovery, "--base", "merge-base"]).contains("+fn after_restore() {}")),
        "{}",
        laptop.stdout(&["diff", &recovery, "--base", "merge-base"])
    );
    assert!(!laptop.stdout(&["diff", &thread]).contains("after_restore"), "the original thread keeps the server's version");
    assert!(fs::read_to_string(&main).expect("read").contains("fn after_restore() {}"), "the disk keeps the local work");
    assert!(
        eventually(|| home.try_stdout(&["in-flight", "src/main.rs"]).contains(&recovery)),
        "the recovery's work reaches the server: {}",
        home.try_stdout(&["in-flight", "src/main.rs"])
    );
}
