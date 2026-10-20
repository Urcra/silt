//! Agents other than Claude Code (ADR 0012), with scripted stand-ins that
//! write each agent's own session format: sessions are found and recorded,
//! continue natively on another machine, and move between agents.

mod common;

use std::fs;

use common::{Machine, Server, checkout_from, eventually, origin, thread_of};

struct World {
    laptop: Machine,
    home: Machine,
    server: Server,
    root: tempfile::TempDir,
}

fn world() -> World {
    let root = tempfile::Builder::new().prefix("silt-agents").tempdir_in("/tmp").expect("temp dir");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let home = Machine::new(root.path(), "home", &origin, &server);
    World { laptop, home, server, root }
}

/// Records a first session with `agent` on the laptop and waits until the
/// home machine has all of it.
fn recorded(world: &World, agent: &str, said: &str) -> String {
    world.laptop.silt(&[agent]);
    let thread = thread_of(&world.laptop);
    let show = world.laptop.stdout(&["show", &thread]);
    assert!(show.contains("> Please add a farewell line to main"), "{show}");
    assert!(show.contains(said), "{show}");
    // OpenCode's store is read after the fact, so its edits are only known
    // to be the agent's, not which tool call's.
    let credited = if agent == "opencode" { "src/main.rs +17" } else { "    ✎ src/main.rs" };
    assert!(show.contains(credited), "the edit is credited to the agent: {show}");
    assert!(
        eventually(|| world.home.try_stdout(&["show", &thread]).contains(said)),
        "the home machine gets the thread"
    );
    assert!(world.root.path().exists() && world.server.data.exists());
    thread
}

fn resume_on_home(world: &World, args: &[&str], envs: &[(&str, &str)]) -> std::path::PathBuf {
    let mut command = vec!["resume"];
    command.extend_from_slice(args);
    let resumed = world.home.silt_with(&command, envs);
    checkout_from(&resumed.stderr)
}

#[test]
fn codex_sessions_are_found_and_continue_natively_elsewhere() {
    let world = world();
    let thread = recorded(&world, "codex", "Codex says done.");
    let config = fs::read_to_string(world.laptop.agents.join("codex/last-config")).expect("codex got config");
    assert!(config.contains("mcp_servers.silt.command="), "{config}");

    let checkout = resume_on_home(&world, &[&thread], &[("FAKE_EXPECT", "Please-add-a-farewell-line")]);
    assert_eq!(
        fs::read_to_string(checkout.join("src/main.rs")).expect("read"),
        "fn main() {}\nfn farewell() {}\nfn codex() {}\n"
    );
    let id = fs::read_to_string(world.home.agents.join("codex/last-session")).expect("resumed");
    assert!(
        world.home.stdout(&["show", &thread]).contains("> Continuing in codex"),
        "the continued session is recorded"
    );
    assert!(!id.is_empty());
}

#[test]
fn gemini_and_copilot_sessions_continue_natively_elsewhere() {
    for (agent, said) in [("gemini", "Gemini says done."), ("copilot", "Copilot says done.")] {
        let world = world();
        let thread = recorded(&world, agent, said);
        let checkout = resume_on_home(&world, &[&thread], &[("FAKE_EXPECT", "Please-add-a-farewell-line")]);
        let main = fs::read_to_string(checkout.join("src/main.rs")).expect("read");
        assert!(main.ends_with(&format!("fn {agent}() {{}}\n")), "{main}");
        assert!(
            world.home.stdout(&["show", &thread]).contains(&format!("> Continuing in {agent}")),
            "{agent}'s continued session is recorded"
        );
    }
}

#[test]
fn opencode_sessions_are_read_from_its_store_and_imported_elsewhere() {
    let world = world();
    let thread = recorded(&world, "opencode", "OpenCode says done.");
    let config = fs::read_to_string(world.laptop.agents.join("opencode/last-config")).expect("config");
    assert!(config.contains("\"silt\""), "OpenCode gets Silt's MCP server: {config}");

    let checkout = resume_on_home(&world, &[&thread], &[("FAKE_EXPECT", "Please-add-a-farewell-line")]);
    let main = fs::read_to_string(checkout.join("src/main.rs")).expect("read");
    assert!(main.ends_with("fn opencode() {}\n"), "{main}");
    assert!(
        eventually(|| world.home.stdout(&["show", &thread]).contains("> Continuing in opencode")),
        "the continued session is read back from the store"
    );
}

#[test]
fn a_thread_moves_from_claude_to_codex_and_back() {
    let world = world();
    world.laptop.silt(&["claude"]);
    let thread = thread_of(&world.laptop);
    assert!(eventually(|| world.home.try_stdout(&["show", &thread]).contains("Done: farewell added")));

    let checkout = resume_on_home(
        &world,
        &[&thread, "--agent", "codex"],
        &[("FAKE_EXPECT", "Please-add-a-farewell-line Done:-farewell-added")],
    );
    assert!(fs::read_to_string(checkout.join("src/main.rs")).expect("read").ends_with("fn codex() {}\n"));
    let rollouts: String = walk(&world.home.agents.join("codex/sessions"))
        .iter()
        .map(|path| fs::read_to_string(path).unwrap_or_default())
        .collect();
    let saved = rollouts
        .split("saved to ")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("the translation mentions the saved tool output")
        .to_owned();
    assert_eq!(
        fs::read_to_string(&saved).unwrap_or_else(|_| panic!("the tool output the translation points to exists here: {saved}")),
        "big output\n"
    );
    let show = world.home.stdout(&["show", &thread]);
    assert!(show.contains("> Continuing in codex") && show.contains("Codex says done."), "{show}");

    assert!(eventually(|| world.laptop.try_stdout(&["show", &thread]).contains("Codex says done.")));
    world.laptop.silt_with(
        &["resume", &thread, "--agent", "claude"],
        &[
            ("FAKE_TRANSLATED", "1"),
            ("FAKE_EXPECT", "Please-add-a-farewell-line Continuing-in-codex Codex-says-done"),
            ("FAKE_SAY", "Back in claude"),
            ("FAKE_LINE", "fn back() {}"),
        ],
    );
    let show = world.laptop.stdout(&["show", &thread]);
    assert!(show.contains("> Back in claude"), "{show}");

    world.home.silt_with(
        &["resume", &thread],
        &[("FAKE_TRANSLATED", "1"), ("FAKE_EXPECT", "Back-in-claude Codex-says-done Please-add-a-farewell-line")],
    );
}

#[test]
fn other_agents_start_primed_with_the_conversation() {
    let world = world();
    world.laptop.silt(&["claude"]);
    let thread = thread_of(&world.laptop);
    assert!(eventually(|| world.home.try_stdout(&["show", &thread]).contains("Done: farewell added")));

    let resumed = world.home.silt(&["resume", &thread, "--agent", "opencode"]);
    assert!(String::from_utf8_lossy(&resumed.stderr).contains("with the conversation so far"));
    let prompt = fs::read_to_string(world.home.agents.join("opencode/last-prompt")).expect("primed");
    assert!(prompt.contains("Please add a farewell line to main"), "{prompt}");
    assert!(prompt.contains("Done: farewell added") && prompt.contains("Claude Code"), "{prompt}");
    assert!(
        eventually(|| world.home.stdout(&["show", &thread]).contains("OpenCode says done.")),
        "the primed session is recorded in the same thread"
    );
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}
