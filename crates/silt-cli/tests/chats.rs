//! Chats (ADR 0059): threads that belong to no repository, each working in
//! a folder of its own, private to their person, continued on another
//! machine and made a project in place.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Machine, Server, eventually, origin};

/// Fields drop in order: the machines stop their daemons before the
/// directories holding their sockets go away.
struct World {
    laptop: Machine,
    home: Machine,
    bob: Machine,
    _server: Server,
    _root: tempfile::TempDir,
}

fn world() -> World {
    let root = common::temp_root("silt-chats");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let home = Machine::new(root.path(), "home", &origin, &server);
    let bob = Machine::new_for(root.path(), "bob", "bob", &origin, &server);
    for machine in [&laptop, &home] {
        let fake = machine.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
        let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
        fs::write(machine.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    }
    World { laptop, home, bob, _server: server, _root: root }
}

fn repo_of(machine: &Machine, thread: &str) -> Option<String> {
    let threads = machine.web("GET", "/api/threads", None);
    let listed = threads.as_array()?.iter().find(|listed| listed["id"] == thread)?;
    listed["repo"].as_str().map(str::to_owned)
}

fn settled(machine: &Machine, thread: &str, last: &str) -> bool {
    eventually(|| {
        let history = machine.web("GET", &format!("/api/threads/{thread}/history"), None);
        let said = history["entries"].as_array().is_some_and(|entries| entries.iter().any(|entry| entry["text"] == last));
        said && machine.web("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["state"] == "idle"
    })
}

fn chat_folder(machine: &Machine) -> PathBuf {
    let chats = machine.silt_home.join("chats");
    let folders: Vec<PathBuf> =
        fs::read_dir(&chats).expect("the chats folder").map(|entry| entry.expect("entry").path()).collect();
    assert_eq!(folders.len(), 1, "{folders:?}");
    folders.into_iter().next().expect("one")
}

#[test]
fn a_chat_works_in_its_own_folder_continues_elsewhere_and_becomes_a_project() {
    let world = world();
    let (laptop, home) = (&world.laptop, &world.home);
    let prompt = "Which motor driver for a stepper?";
    let started =
        laptop.web("POST", "/api/agents/start", Some(serde_json::json!({"agent": "v1", "chat": true, "prompt": prompt})));
    let thread = started.as_str().expect("the thread").to_owned();
    assert!(
        settled(laptop, &thread, &format!("Done: {prompt}")),
        "the chat's agent answers, with no one asked to allow its edit"
    );
    assert_eq!(repo_of(laptop, &thread).as_deref(), Some("silt:chats"));
    assert_eq!(laptop.web("GET", "/api/chats/settings", None)["preset"], "full_access");

    let folder = chat_folder(laptop);
    let name = folder.file_name().expect("named").to_string_lossy().into_owned();
    assert!(name.contains("-which-motor-driver-for-a-"), "{name}");
    assert!(fs::read_to_string(folder.join("src/main.rs")).expect("the agent's file").contains("fn acp() {}"));
    let projects = laptop.web("GET", "/api/projects", None);
    assert!(!projects.to_string().contains("silt:chats"), "chats are no project: {projects}");

    assert!(eventually(|| repo_of(home, &thread).is_some()), "the chat reaches its person's other machine");
    // Someone else in the same org sees the project's threads, but not the chat.
    laptop.silt(&["claude"]);
    let threads = laptop.web("GET", "/api/threads", None);
    let recorded = threads
        .as_array()
        .expect("threads")
        .iter()
        .find(|listed| listed["repo"] != "silt:chats")
        .and_then(|listed| listed["id"].as_str())
        .expect("the project's thread")
        .to_owned();
    assert!(eventually(|| repo_of(&world.bob, &recorded).is_some()), "bob sees the project's thread");
    assert_eq!(repo_of(&world.bob, &thread), None, "the chat is private");

    let resumed = home.web("POST", &format!("/api/threads/{thread}/resume"), Some(serde_json::json!({})));
    let checkout = Path::new(resumed["checkout"].as_str().expect("its checkout")).to_owned();
    assert!(checkout.starts_with(home.silt_home.join("chats")), "in a chat folder there: {}", checkout.display());
    assert!(fs::read_to_string(checkout.join("src/main.rs")).expect("its files came along").contains("fn acp() {}"));
    assert!(silt_git::Repo::discover(&checkout).expect("a repository").is_chat());

    let forked = home.web("POST", &format!("/api/threads/{thread}/fork"), Some(serde_json::json!({})));
    let fork_checkout = Path::new(forked["checkout"].as_str().expect("the fork's checkout")).to_owned();
    assert_ne!(fork_checkout, checkout, "a fork gets a folder of its own");
    assert!(fork_checkout.starts_with(home.silt_home.join("chats")));
    assert!(fs::read_to_string(fork_checkout.join("src/main.rs")).expect("with the chat's files").contains("fn acp() {}"));
    let again = home.web("POST", &format!("/api/threads/{thread}/resume"), Some(serde_json::json!({})));
    assert_eq!(again["checkout"].as_str().map(Path::new), Some(checkout.as_path()), "the chat keeps its folder once forked");

    let start = serde_json::json!({
        "clone": laptop.repo, "agent": "v1", "here": true, "prompt": "Use what we found",
        "continues": thread, "copy_to": "notes/motor-driver",
    });
    let continued = laptop.web("POST", "/api/agents/start", Some(start)).as_str().expect("the new thread").to_owned();
    assert!(fs::read_to_string(laptop.repo.join("notes/motor-driver/src/main.rs")).expect("copied").contains("fn acp() {}"));
    assert!(!laptop.repo.join("notes/motor-driver/.git").exists(), "not the chat's git");
    assert_ne!(repo_of(laptop, &continued).as_deref(), Some("silt:chats"));
    assert!(
        eventually(|| {
            let history = laptop.web("GET", &format!("/api/threads/{continued}/history"), None);
            history.to_string().contains(&format!("This thread continues chat {}", &thread[thread.len() - 12..]))
        }),
        "the agent starts from the chat"
    );
    let escape = serde_json::json!({"clone": laptop.repo, "agent": "v1", "continues": thread, "copy_to": "../out"});
    assert_eq!(laptop.web_answer("POST", "/api/agents/start", Some(escape)).0, "400", "only inside the checkout");

    let project = laptop.web("POST", &format!("/api/threads/{thread}/make-project"), None);
    let project = project.as_str().expect("the project").to_owned();
    assert!(project.ends_with(&name), "{project}");
    assert!(!silt_git::Repo::discover(&folder).expect("a repository").is_chat());
    assert_eq!(repo_of(laptop, &thread).as_deref(), Some(project.as_str()));
    assert!(laptop.web("GET", "/api/projects", None).to_string().contains(&project), "the folder is a project now");
    assert!(eventually(|| repo_of(home, &thread).as_deref() == Some(project.as_str())), "the other machine hears of it");
}

#[test]
fn silt_chat_runs_the_agent_cli_in_a_new_chat() {
    let world = world();
    let laptop = &world.laptop;
    let output = laptop.silt(&["chat", "--about", "Resistor values for a 3.3 V LED"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("silt: chatting in "));
    let folder = chat_folder(laptop);
    assert!(folder.to_string_lossy().contains("-resistor-values-for-a-3-"), "{}", folder.display());
    let threads = laptop.web("GET", "/api/threads", None);
    let chat = threads.as_array().expect("threads").iter().find(|listed| listed["repo"] == "silt:chats");
    assert!(chat.is_some(), "recorded as a chat: {threads}");

    let outside = tempfile::tempdir().expect("temp dir");
    let mut claude = laptop.command(&["claude"], &[]);
    let warned = claude.current_dir(outside.path()).output().expect("silt runs");
    assert!(
        String::from_utf8_lossy(&warned.stderr).contains("`silt chat` starts one"),
        "{}",
        String::from_utf8_lossy(&warned.stderr)
    );
}
