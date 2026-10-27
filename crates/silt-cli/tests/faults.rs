//! Recording survives the daemon (ADR 0018): killed or stopped while a tool
//! runs, a session still keeps every turn mark and credits each edit to
//! its tool, and sessions of one family left without a daemon record side
//! by side.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Output, Stdio};

use common::{Machine, Server, eventually, origin, thread_of};

struct World {
    laptop: Machine,
    _server: Server,
    root: tempfile::TempDir,
}

fn world() -> World {
    let root = common::temp_root("silt-faults");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    laptop.silt(&["daemon", "start"]);
    World { laptop, _server: server, root }
}

/// Runs `silt args` until the agent is inside its tool, at `marker`.
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

/// The machine's daemon process, found by its command line and `SILT_HOME`.
#[cfg(unix)]
fn daemon_pid(machine: &Machine) -> u32 {
    let home = format!("SILT_HOME={}", machine.silt_home.display());
    fs::read_dir("/proc")
        .expect("lists processes")
        .flatten()
        .find_map(|entry| {
            let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
            let cmdline = fs::read(entry.path().join("cmdline")).ok()?;
            let args: Vec<&[u8]> = cmdline.split(|byte| *byte == 0).collect();
            let daemon = args.windows(2).any(|pair| pair == [b"daemon".as_slice(), b"run".as_slice()]);
            let environ = fs::read(entry.path().join("environ")).ok()?;
            let ours = environ.split(|byte| *byte == 0).any(|variable| variable == home.as_bytes());
            (daemon && ours).then_some(pid)
        })
        .expect("the daemon runs")
}

/// The machine's daemon process: the one serving its pipe.
#[cfg(windows)]
fn daemon_pid(machine: &Machine) -> u32 {
    let status = machine.stdout(&["daemon", "status"]);
    let pipe = status.trim().strip_prefix("running (").and_then(|rest| rest.strip_suffix(')')).expect("the daemon runs");
    silt_os::pipe::server_pid(Path::new(pipe)).expect("the pipe's server")
}

fn kill_daemon(machine: &Machine) {
    silt_os::kill(daemon_pid(machine)).expect("kills");
    assert!(eventually(|| machine.stdout(&["daemon", "status"]).trim() == "not running"));
}

/// Whether anyone holds the capture lock right now.
fn capture_lock_free(machine: &Machine) -> bool {
    let file = fs::OpenOptions::new().write(true).open(machine.silt_home.join("capture.lock")).expect("opens");
    let free = file.try_lock().is_ok();
    let _ = file.unlock();
    free
}

/// One tool call made the session's only edit, and the turn finished.
fn recorded_once(machine: &Machine, thread: &str, line: &str) {
    let show = machine.stdout(&["show", thread]);
    assert_eq!(show.matches("    ✎ src/main.rs +").count(), 1, "the edit is credited to its tool, once: {show}");
    assert!(!show.contains("outside tool calls"), "{show}");
    let turn = machine.stdout(&["diff", thread, "--turn", "1"]);
    assert!(turn.contains(&format!("+{line}")), "{turn}");
}

#[test]
fn a_daemon_killed_while_a_tool_runs_loses_nothing() {
    let world = world();
    let laptop = &world.laptop;
    let marker = world.root.path().join("in-tool");
    let agent = in_tool(laptop, &["claude"], &marker, &[]);
    kill_daemon(laptop);
    let_go(&marker);
    finished(agent);

    let thread = thread_of(laptop);
    recorded_once(laptop, &thread, "fn farewell() {}");
    assert!(laptop.stdout(&["show", &thread]).contains("Done: farewell added."));
}

#[test]
fn a_daemon_stopped_while_a_tool_runs_loses_nothing() {
    let world = world();
    let laptop = &world.laptop;
    let marker = world.root.path().join("in-tool");
    let agent = in_tool(laptop, &["claude"], &marker, &[]);
    // Stopped between two of its turns at writing: one stopped while it
    // writes holds everyone up, which no capture survives.
    let pid = daemon_pid(laptop);
    loop {
        silt_os::suspend(pid).expect("stops");
        if capture_lock_free(laptop) {
            break;
        }
        silt_os::resume(pid).expect("goes on");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let_go(&marker);
    let finished = finished(agent);
    silt_os::resume(pid).expect("goes on");
    assert!(!String::from_utf8_lossy(&finished.stderr).contains("error"));

    let thread = thread_of(laptop);
    recorded_once(laptop, &thread, "fn farewell() {}");
    let show = laptop.stdout(&["show", &thread]);
    assert_eq!(show.matches("Done: farewell added.").count(), 1, "{show}");
}

#[test]
fn a_hook_kept_waiting_too_long_is_recorded_by_the_next_one() {
    let world = world();
    let laptop = &world.laptop;
    laptop.silt(&["daemon", "stop"]);
    let marker = world.root.path().join("in-tool");
    let agent = in_tool(laptop, &["claude"], &marker, &[("SILT_DAEMONLESS_CAPTURE", "1")]);
    let holder = fs::OpenOptions::new().write(true).open(laptop.silt_home.join("capture.lock")).expect("opens");
    holder.lock().expect("holds the capture lock, as a stopped writer would");
    let_go(&marker);
    let spooled = laptop.silt_home.join("spooled-hooks");
    assert!(
        eventually(|| fs::read_dir(&spooled).is_ok_and(|mut entries| entries.next().is_some())),
        "the hook that waited too long left itself for later"
    );
    holder.unlock().expect("lets go");
    let finished = agent.wait_with_output().expect("waits");
    let stderr = String::from_utf8_lossy(&finished.stderr);
    assert!(finished.status.success() && stderr.contains("recorded once it lets go"), "{stderr}");

    let thread = thread_of(laptop);
    recorded_once(laptop, &thread, "fn farewell() {}");
    assert!(fs::read_dir(&spooled).expect("lists").next().is_none(), "nothing is left");
}

#[test]
fn two_sessions_of_one_family_record_side_by_side_without_a_daemon() {
    let world = world();
    let laptop = &world.laptop;
    laptop.silt(&["claude"]);
    let thread = thread_of(laptop);
    let forks: Vec<(PathBuf, Child)> = ["one", "two"]
        .iter()
        .map(|name| {
            let marker = world.root.path().join(format!("in-tool-{name}"));
            let (say, line) = (format!("Fork {name} here"), format!("fn {name}() {{}}"));
            let agent = in_tool(laptop, &["fork", &thread], &marker, &[("FAKE_SAY", &say), ("FAKE_LINE", &line)]);
            (marker, agent)
        })
        .collect();
    kill_daemon(laptop);
    for (marker, _) in &forks {
        let_go(marker);
    }
    let forked: Vec<String> = forks
        .into_iter()
        .map(|(_, agent)| {
            let stderr = String::from_utf8_lossy(&finished(agent).stderr).into_owned();
            let line = stderr.lines().find(|line| line.contains(" as thread ")).expect("names the fork").to_owned();
            line.split(" as thread ").nth(1).and_then(|rest| rest.split(',').next()).expect("fork id").to_owned()
        })
        .collect();

    for (fork, (name, other)) in forked.iter().zip([("one", "two"), ("two", "one")]) {
        let show = laptop.stdout(&["show", fork]);
        assert!(show.contains(&format!("> Fork {name} here")), "{show}");
        recorded_once_after_fork(laptop, fork, name, other);
    }
}

/// The fork's own turn made its edit, credited to its tool.
fn recorded_once_after_fork(machine: &Machine, fork: &str, name: &str, other: &str) {
    let show = machine.stdout(&["show", fork]);
    assert!(!show.contains("outside tool calls"), "{show}");
    let own = machine.stdout(&["diff", fork]);
    assert!(own.contains(&format!("+fn {name}() {{}}")) && !own.contains(&format!("fn {other}()")), "{own}");
}

/// `silt` run from `exe`, as `machine` runs it.
fn silt_from(machine: &Machine, exe: &Path, args: &[&str]) -> Output {
    let mut command = std::process::Command::new(exe);
    let like = machine.command(&[], &[]);
    for (key, value) in like.get_envs() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    command.args(args).current_dir(&machine.repo).output().expect("silt runs")
}

#[test]
fn a_daemon_whose_program_was_replaced_is_restarted_on_the_new_one() {
    let world = world();
    let laptop = &world.laptop;
    laptop.silt(&["daemon", "stop"]);
    let bin = world.root.path().join("bin");
    fs::create_dir_all(&bin).expect("mkdir");
    let installed = bin.join(Path::new(env!("CARGO_BIN_EXE_silt")).file_name().expect("a file name"));
    let program = fs::read(env!("CARGO_BIN_EXE_silt")).expect("reads silt");
    fs::write(&installed, &program).expect("installs");
    silt_os::make_executable(&installed).expect("executable");
    assert!(silt_from(laptop, &installed, &["log"]).status.success());
    let before = daemon_pid(laptop);

    // Another build beside it leaves it be.
    let other = laptop.run(&["log"]);
    assert!(String::from_utf8_lossy(&other.stderr).is_empty(), "{}", String::from_utf8_lossy(&other.stderr));

    // Installed again, as the running program can't be written over.
    fs::rename(&installed, bin.join("silt.old")).expect("moves the old one aside");
    fs::write(&installed, &program).expect("installs again");
    silt_os::make_executable(&installed).expect("executable");
    let elsewhere = laptop.run(&["log"]);
    assert!(
        String::from_utf8_lossy(&elsewhere.stderr).contains("`silt daemon stop` restarts it"),
        "another build only says so: {}",
        String::from_utf8_lossy(&elsewhere.stderr)
    );
    assert_eq!(daemon_pid(laptop), before);

    let restarted = silt_from(laptop, &installed, &["log"]);
    assert!(restarted.status.success(), "{}", String::from_utf8_lossy(&restarted.stderr));
    assert!(
        String::from_utf8_lossy(&restarted.stderr).contains("restarted the daemon"),
        "{}",
        String::from_utf8_lossy(&restarted.stderr)
    );
    assert_ne!(daemon_pid(laptop), before, "a new daemon runs");
    let again = silt_from(laptop, &installed, &["log"]);
    assert!(String::from_utf8_lossy(&again.stderr).is_empty(), "once: {}", String::from_utf8_lossy(&again.stderr));
}
