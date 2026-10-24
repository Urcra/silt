//! Importing the sessions agents already have on disk (ADR 0043): each as
//! a thread kept on this machine, searchable and continued like a recorded
//! one, until `silt send` sends it, redacted.
// The agents these tests drive are shell-script stand-ins.
#![cfg(unix)]

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::{Machine, Server, eventually, origin};

/// Fields drop in order: the machines stop their daemons before the
/// directories holding their sockets go away.
struct World {
    laptop: Machine,
    desktop: Machine,
    server: Server,
    root: tempfile::TempDir,
}

fn world() -> World {
    let root = tempfile::Builder::new().prefix("silt-import").tempdir_in("/tmp").expect("temp dir");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let desktop = Machine::new(root.path(), "desktop", &origin, &server);
    World { laptop, desktop, server, root }
}

/// Runs an agent's fake in `dir` on its own, as before Silt was installed:
/// no hooks, nothing recorded.
fn run_agent_alone(machine: &Machine, bin: &Path, dir: &Path, args: &[&str]) {
    let mut command = Command::new(bin);
    let silt = machine.command(&[], &[]);
    for (key, value) in silt.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
    let output = common::output_once_released(command.args(args).current_dir(dir));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

fn log_lines(machine: &Machine) -> Vec<String> {
    machine.try_stdout(&["log"]).lines().map(str::to_owned).collect()
}

fn line_of(machine: &Machine, thread: &str) -> Option<String> {
    log_lines(machine).into_iter().find(|line| line.starts_with(thread))
}

#[test]
fn sessions_from_before_silt_are_imported_once_kept_here_and_sent_redacted() {
    let world = world();
    let laptop = &world.laptop;
    let session = "6f1c2d3e-0000-4000-8000-000000000001";
    run_agent_alone(laptop, &laptop.fake, &laptop.repo, &["--session-id", session]);
    run_agent_alone(laptop, &laptop.agents.join("codex-bin"), &laptop.repo, &[]);
    let scratch = world.root.path().join("scratch");
    fs::create_dir_all(scratch.join("src")).expect("mkdir");
    run_agent_alone(laptop, &laptop.fake, &scratch, &["--session-id", "6f1c2d3e-0000-4000-8000-000000000002"]);

    let imported = laptop.stdout(&["import"]);
    assert!(imported.contains("imported 3 sessions"), "{imported}");
    assert!(imported.contains("(no project)"), "the scratch session has no project: {imported}");
    let again = laptop.stdout(&["import"]);
    assert!(again.contains("no sessions to import"), "importing twice adds nothing: {again}");
    assert_eq!(log_lines(laptop).len(), 3, "{:?}", log_lines(laptop));
    assert!(log_lines(laptop).iter().all(|line| line.contains("this machine only")), "{:?}", log_lines(laptop));

    let found = laptop.stdout(&["search", "farewell"]);
    let claude = found.split_whitespace().next().expect("a hit").to_owned();
    assert!(found.contains("farewell"), "{found}");
    let shown = laptop.stdout(&["show", &claude]);
    assert!(shown.contains("Please add a farewell line"), "{shown}");

    // A project's new home leaves imported threads where they are.
    laptop.silt(&["project", "home", "local"]);
    laptop.silt(&["project", "home", &world.server.url]);
    assert!(log_lines(laptop).iter().all(|line| line.contains("this machine only")), "{:?}", log_lines(laptop));
    assert!(!laptop.run(&["thread", "share", &claude, "someone"]).status.success(), "a thread kept here can't be shared");

    // A secret no longer in the environment, added to the secrets file just
    // before sending.
    fs::write(laptop.silt_home.join("secrets"), "Done: farewell added\n").expect("writes");
    let sent = laptop.stdout(&["send", &claude]);
    assert!(sent.contains("sending 1 imported threads"), "{sent}");
    assert!(sent.contains("2 secrets"), "the fake's token and the secrets file's: {sent}");
    assert!(!line_of(laptop, &claude).expect("listed").contains("this machine only"));
    assert!(eventually(|| line_of(&world.desktop, &claude).is_some()), "the server has the sent thread");
    assert_eq!(log_lines(&world.desktop).len(), 1, "only the sent thread: {:?}", log_lines(&world.desktop));
    assert!(
        eventually(|| world.desktop.try_stdout(&["show", &claude]).contains("Please add a farewell line")),
        "the sent thread's conversation arrives"
    );
    let there = world.desktop.stdout(&["show", &claude]);
    assert!(!there.contains("ghp_abcdefghijklmnopqrstuvwxyz0123"), "redacted before it left: {there}");
    assert!(!there.contains("Done: farewell added"), "{there}");

    // Continued elsewhere, it gets the tree of the commit it started from.
    let resumed = world.desktop.silt(&["resume", &claude]);
    let checkout = common::checkout_from(&resumed.stderr);
    let main = fs::read_to_string(checkout.join("src/main.rs")).expect("the tree is checked out");
    assert!(main.starts_with("fn main() {}"), "{main}");
    assert!(main.contains("fn home()"), "the resumed agent edited it: {main}");
}

#[test]
fn import_takes_paths_and_an_agent() {
    let world = world();
    let laptop = &world.laptop;
    run_agent_alone(laptop, &laptop.fake, &laptop.repo, &["--session-id", "6f1c2d3e-0000-4000-8000-000000000003"]);
    run_agent_alone(laptop, &laptop.agents.join("codex-bin"), &laptop.repo, &[]);
    let elsewhere = world.root.path().join("elsewhere");
    fs::create_dir_all(elsewhere.join("src")).expect("mkdir");
    run_agent_alone(laptop, &laptop.fake, &elsewhere, &["--session-id", "6f1c2d3e-0000-4000-8000-000000000004"]);

    let codex = laptop.stdout(&["import", "--agent", "codex", "."]);
    assert!(codex.contains("imported 1 sessions"), "{codex}");
    assert!(codex.contains("codex"), "{codex}");
    let codex_thread = codex.split_whitespace().next().expect("the thread").to_owned();
    let forked = laptop.silt(&["fork", &codex_thread]);
    let checkout = common::checkout_from(&forked.stderr);
    let main = fs::read_to_string(checkout.join("src/main.rs")).expect("the fork has the tree");
    assert!(main.starts_with("fn main() {}") && main.contains("fn codex()"), "{main}");
    let here = laptop.stdout(&["import", "."]);
    assert!(here.contains("imported 1 sessions"), "only the session in this clone: {here}");
    let rest = laptop.stdout(&["import"]);
    assert!(rest.contains("imported 1 sessions"), "{rest}");

    // A thread with no project is read here, but can't be continued.
    let found = laptop.stdout(&["search", "farewell"]);
    let unplaced =
        log_lines(laptop).into_iter().map(|line| line.split_whitespace().next().expect("an id").to_owned()).find(|thread| {
            let output = laptop.run(&["resume", thread]);
            String::from_utf8_lossy(&output.stderr).contains("outside any git clone")
        });
    assert!(unplaced.is_some(), "{found}");
    let sent_all = laptop.stdout(&["send", "--all"]);
    assert!(sent_all.contains("sending 3 imported threads"), "{sent_all}");
    assert!(laptop.stdout(&["send", "--all"]).contains("sending 0 imported threads"));
}

#[test]
fn an_imported_thread_keeps_the_times_its_session_ran_at() {
    let world = world();
    let laptop = &world.laptop;
    let slug: String =
        laptop.repo.to_string_lossy().chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' }).collect();
    let dir = laptop.claude_dir.join("projects").join(slug);
    fs::create_dir_all(&dir).expect("mkdir");
    let cwd = laptop.repo.to_string_lossy();
    fs::write(
        dir.join("6f1c2d3e-0000-4000-8000-000000000005.jsonl"),
        format!(
            "{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"timestamp\":\"2025-03-04T05:06:07.000Z\",\"message\":{{\"role\":\"user\",\"content\":\"What is the oldest thing here?\"}}}}\n\
             {{\"type\":\"assistant\",\"timestamp\":\"2025-03-04T05:16:07.000Z\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"The seed commit.\"}}]}}}}\n"
        ),
    )
    .expect("writes");
    laptop.silt(&["import"]);
    let threads = laptop.web("GET", "/api/threads", None);
    let thread = &threads.as_array().expect("a list")[0];
    assert_eq!(thread["created_ms"], 1_741_064_767_000_i64, "{thread}");
    assert_eq!(thread["last_ms"], 1_741_065_367_000_i64, "{thread}");
}

#[test]
fn every_agent_s_sessions_are_imported() {
    let world = world();
    let laptop = &world.laptop;
    let agents = &laptop.agents;
    // The fake checks that Silt's MCP server is configured, as `silt gemini` does.
    fs::create_dir_all(agents.join("gemini/.gemini")).expect("mkdir");
    fs::write(agents.join("gemini/.gemini/settings.json"), r#"{"mcpServers":{"silt":{}}}"#).expect("writes");
    run_agent_alone(laptop, &agents.join("gemini-bin"), &laptop.repo, &["--session-id", "6f1c2d3e-0000-4000-8000-000000000006"]);
    // And that a plugin gives its hooks; these do nothing.
    let plugin = world.root.path().join("copilot-plugin");
    fs::create_dir_all(plugin.join("hooks")).expect("mkdir");
    fs::write(plugin.join("hooks/hooks.json"), r#"{"userPromptSubmitted":[{"bash":"true"}]}"#).expect("writes");
    let plugin = plugin.to_string_lossy();
    run_agent_alone(
        laptop,
        &agents.join("copilot-bin"),
        &laptop.repo,
        &["--session-id", "6f1c2d3e-0000-4000-8000-000000000007", "--plugin-dir", &plugin],
    );
    run_agent_alone(laptop, &agents.join("opencode-bin"), &laptop.repo, &[]);
    let imported = laptop.stdout(&["import"]);
    for agent in ["gemini-cli", "copilot", "opencode"] {
        assert!(imported.contains(agent), "{agent}: {imported}");
    }
    assert!(imported.contains("imported 3 sessions"), "{imported}");
    let found = laptop.stdout(&["search", "farewell"]);
    let threads: std::collections::HashSet<&str> = found.lines().filter_map(|line| line.split_whitespace().next()).collect();
    assert!(threads.len() >= 3, "each agent's thread is searchable: {found}");
}
