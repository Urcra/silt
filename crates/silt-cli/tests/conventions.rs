//! Repository conventions on managed checkouts: `.agents/prepare` runs and
//! `.agents/linked` paths are shared on every mount.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use common::{Machine, Server, checkout_from, eventually, git, origin, thread_of};

struct World {
    laptop: Machine,
    home: Machine,
    server: Server,
    root: tempfile::TempDir,
}

const PREPARE: &str = "#!/bin/sh\necho \"prepared $SILT_THREAD\" >> node_modules/prepare.log\necho prepare-ran\n";

fn world() -> World {
    let root = tempfile::Builder::new().prefix("silt-conv").tempdir_in("/tmp").expect("temp dir");
    let origin = origin(root.path());
    let seed = root.path().join("seed");
    fs::create_dir_all(seed.join(".agents")).expect("mkdir");
    fs::write(seed.join(".gitignore"), "node_modules/\n").expect("write");
    fs::write(seed.join(".agents/linked"), "# dependencies\nnode_modules\n").expect("write");
    fs::write(seed.join(".agents/prepare"), PREPARE).expect("write");
    fs::set_permissions(seed.join(".agents/prepare"), fs::Permissions::from_mode(0o755)).expect("chmod");
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "conventions"]);
    git(&seed, &["push", "-q", &origin.to_string_lossy(), "main"]);

    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let home = Machine::new(root.path(), "home", &origin, &server);
    for machine in [&laptop, &home] {
        fs::create_dir_all(machine.repo.join("node_modules/dep")).expect("mkdir");
        fs::write(machine.repo.join("node_modules/dep/index.js"), "installed\n").expect("write");
    }
    World { laptop, home, server, root }
}

#[test]
fn managed_checkouts_link_shared_paths_and_run_prepare() {
    let world = world();
    let started = world.laptop.silt(&["claude", "--isolated"]);
    let stderr = String::from_utf8_lossy(&started.stderr);
    assert!(stderr.contains("silt: linked node_modules"), "{stderr}");
    let thread = thread_of(&world.laptop);
    let checkout = world.laptop.repo.join(".silt/checkouts").join(
        fs::read_dir(world.laptop.repo.join(".silt/checkouts"))
            .expect("checkouts")
            .next()
            .expect("one checkout")
            .expect("entry")
            .file_name(),
    );
    assert!(fs::symlink_metadata(checkout.join("node_modules")).expect("linked").is_symlink());
    assert_eq!(
        fs::read_to_string(checkout.join("node_modules/dep/index.js")).expect("seeded from the clone"),
        "installed\n"
    );
    assert!(String::from_utf8_lossy(&started.stdout).contains("prepare-ran"));
    let log = fs::read_to_string(checkout.join("node_modules/prepare.log")).expect("prepare ran");
    assert!(log.starts_with("prepared "), "{log}");
    assert!(
        !fs::exists(world.laptop.repo.join("node_modules/prepare.log")).expect("stat"),
        "the shared copy is separate from the user's clone"
    );
    let show = world.laptop.stdout(&["show", &thread]);
    assert!(!show.contains("node_modules"), "linked paths are not part of the thread: {show}");

    assert!(eventually(|| world.home.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))));
    let resumed = world.home.silt(&["resume", &thread]);
    let home_checkout = checkout_from(&resumed.stderr);
    assert!(fs::symlink_metadata(home_checkout.join("node_modules")).expect("linked").is_symlink());
    let log = fs::read_to_string(home_checkout.join("node_modules/prepare.log")).expect("prepare ran on resume");
    assert!(log.starts_with("prepared ") && log.contains(&thread), "{log}");
    let status = git_status(&home_checkout);
    assert!(!status.contains("node_modules"), "links stay out of git status: {status}");

    let here = world.home.silt(&["fork", &thread, "--here"]);
    assert!(!String::from_utf8_lossy(&here.stdout).contains("prepare-ran"), "adopted clones are left alone");
    assert!(world.root.path().join("server").exists() && world.server.data.exists());
}

#[test]
fn a_prepare_script_changed_by_someone_else_needs_approval() {
    let world = world();
    let changed = "#!/bin/sh\necho changed-prepare-ran\n";
    fs::write(world.laptop.repo.join(".agents/prepare"), changed).expect("the laptop's work changes it");
    world.laptop.silt(&["claude"]);
    let thread = thread_of(&world.laptop);
    assert!(eventually(|| world.home.log_line(&thread).is_some_and(|line| line.contains("2 msgs"))));
    let resumed = world.home.silt(&["resume", &thread]);
    let checkout = checkout_from(&resumed.stderr);
    assert_eq!(fs::read_to_string(checkout.join(".agents/prepare")).expect("synced"), changed);
    assert!(!String::from_utf8_lossy(&resumed.stdout).contains("changed-prepare-ran"), "it ran without approval");
    assert!(String::from_utf8_lossy(&resumed.stderr).contains("not running .agents/prepare"));
}

fn git_status(dir: &std::path::Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain"])
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn skills_and_rules_reach_every_agent() {
    let root = tempfile::Builder::new().prefix("silt-skills").tempdir_in("/tmp").expect("temp dir");
    let origin = origin(root.path());
    let seed = root.path().join("seed");
    fs::create_dir_all(seed.join(".agents/skills/review")).expect("mkdir");
    fs::write(
        seed.join(".agents/skills/review/SKILL.md"),
        "---\nname: review\ndescription: Review a change before it lands\n---\n\nRead the diff, then run the gate.\n",
    )
    .expect("write");
    fs::write(seed.join("AGENTS.md"), "Always run the gate before committing.\n").expect("write");
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-q", "-m", "skills"]);
    git(&seed, &["push", "-q", &origin.to_string_lossy(), "main"]);
    let server = Server::start(&root.path().join("server"));
    let machine = Machine::new(root.path(), "laptop", &origin, &server);

    machine.silt(&["claude"]);
    let logs = machine.agents.join("logs");
    let instructions = fs::read_to_string(logs.join("claude-instructions")).expect("Claude got instructions");
    assert!(instructions.contains("Always run the gate before committing."), "{instructions}");
    assert!(!instructions.contains("silt_use_skill"), "Claude loads the skills itself: {instructions}");
    assert_eq!(fs::read_to_string(logs.join("claude-skills")).expect("skills").trim(), "review");

    machine.silt(&["codex"]);
    let config = fs::read_to_string(machine.agents.join("codex/last-config")).expect("config");
    assert!(
        !config.contains("developer_instructions"),
        "Codex reads AGENTS.md and .agents/skills itself: {config}"
    );

    let thread = thread_of(&machine);
    let mut mcp = std::process::Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(["mcp", "--thread", &thread, "--checkout"])
        .arg(&machine.repo)
        .env("SILT_HOME", &machine.silt_home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("silt mcp runs");
    {
        use std::io::Write;
        let mut stdin = mcp.stdin.take().expect("stdin");
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"silt_skills","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"silt_use_skill","arguments":{"name":"review"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"prompts/list"}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"prompts/get","params":{"name":"review"}}"#,
        ] {
            writeln!(stdin, "{line}").expect("write");
        }
    }
    let output = mcp.wait_with_output().expect("mcp exits");
    let replies: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("json"))
        .collect();
    let text = |index: usize| replies[index]["result"]["content"][0]["text"].as_str().unwrap_or_default().to_owned();
    assert!(text(0).contains("review: Review a change before it lands"), "{}", text(0));
    assert!(text(1).contains("Read the diff, then run the gate."), "{}", text(1));
    assert_eq!(replies[2]["result"]["prompts"][0]["name"], "review");
    assert!(
        replies[3]["result"]["messages"][0]["content"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("Read the diff"))
    );
}
