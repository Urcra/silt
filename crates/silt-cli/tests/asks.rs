//! Agents asking other agents (ADR 0120): one fake agent asks another with
//! `silt_ask`, which works in a thread of its own and reports back.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{Machine, Server, eventually, origin};

/// Fields drop in order: the machine stops its daemon before the
/// directories holding its socket go away.
struct World {
    laptop: Machine,
    _server: Server,
    _root: tempfile::TempDir,
}

fn world() -> World {
    let root = common::temp_root("silt-asks");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let fake = laptop.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([
        {"id": "v1", "name": "Fake v1", "command": fake},
        {"id": "other", "name": "Fake other", "command": fake}
    ]);
    fs::write(laptop.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    World { laptop, _server: server, _root: root }
}

fn start(machine: &Machine, prompt: &str) -> String {
    let started = machine.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": machine.repo, "agent": "v1", "prompt": prompt})),
    );
    started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned())
}

fn history(machine: &Machine, thread: &str) -> Vec<serde_json::Value> {
    let (status, body) = machine.web_answer("GET", &format!("/api/threads/{thread}/history"), None);
    if status != "200" {
        return Vec::new();
    }
    serde_json::from_str::<serde_json::Value>(&body).unwrap_or_default()["entries"].as_array().cloned().unwrap_or_default()
}

fn asks(machine: &Machine) -> Vec<serde_json::Value> {
    machine.web("GET", "/api/asks", None).as_array().cloned().unwrap_or_default()
}

fn idle(machine: &Machine, thread: &str) -> bool {
    machine.web("GET", &format!("/api/threads/{thread}/live"), None)["agent"]["state"] == "idle"
}

/// A thread's own checkout, as Silt makes one for an agent it starts.
fn checkout_of(machine: &Machine, thread: &str) -> PathBuf {
    machine.repo.join(".silt/checkouts").join(thread)
}

#[test]
fn an_agent_asks_another_in_its_working_tree_and_gets_its_report() {
    let world = world();
    let laptop = &world.laptop;
    let caller = start(laptop, "Ask agent other: Review src/main.rs and report what you find.");

    let mut said = Vec::new();
    assert!(
        eventually(|| {
            said = history(laptop, &caller);
            said.iter().any(|entry| entry["text"].as_str().is_some_and(|text| text.contains("reported back on ask")))
                && idle(laptop, &caller)
        }),
        "the caller's call answers with the report: {said:#?}\n{}",
        laptop.daemon_log()
    );

    let asked = asks(laptop);
    assert_eq!(asked.len(), 1, "{asked:#?}");
    let ask = &asked[0];
    assert_eq!(ask["caller"], caller.as_str());
    assert_eq!(ask["agent"], "other");
    assert_eq!(ask["agent_name"], "Fake other");
    assert_eq!(ask["caller_agent"], "Fake v1");
    assert_eq!(ask["state"], "done");
    assert_eq!(ask["checkout"], "here");
    assert_eq!(ask["mode"], "review");
    assert_eq!(ask["title"], "Review the change");
    let report = ask["report"].as_str().expect("a report");
    assert!(report.starts_with("Done: Fake v1, the agent in Silt thread"), "its last message: {report}");

    let thread = ask["asked"].as_str().expect("its thread");
    let its = history(laptop, thread);
    // A fork's history starts with its source's; its own prompt is the last.
    let last = its.iter().rev().find(|entry| entry["kind"] == "user").expect("its prompt");
    let prompt = last["text"].as_str().unwrap_or_default();
    assert!(prompt.contains("Review src/main.rs and report what you find."), "{prompt}");
    assert!(prompt.contains("not your user"), "it's told who asks: {prompt}");
    assert!(
        !prompt.contains("Ask agent other"),
        "it starts with only what it was asked, not its caller's conversation: {prompt}"
    );

    // Beside its caller, in the same working tree: both agents' edits are there, and it has no checkout of its own.
    let main = fs::read_to_string(checkout_of(laptop, &caller).join("src/main.rs")).expect("the caller's file");
    assert_eq!(main.matches("fn acp() {}").count(), 2, "{main}");
    assert!(!checkout_of(laptop, thread).exists());
}

#[test]
fn an_ask_in_its_own_checkout_changes_the_caller_only_once_taken() {
    let world = world();
    let laptop = &world.laptop;
    let caller = start(laptop, "Ask agent other: Add a line, in your own checkout.");
    assert!(
        eventually(|| asks(laptop).first().is_some_and(|ask| ask["state"] == "done") && idle(laptop, &caller)),
        "the ask reports: {:#?}\n{}",
        asks(laptop),
        laptop.daemon_log()
    );
    let ask = asks(laptop).remove(0);
    assert_eq!(ask["checkout"], "own");
    assert_eq!(ask["mode"], "work");
    let id = ask["id"].as_str().expect("an id");
    let thread = ask["asked"].as_str().expect("its thread");

    let caller_main = checkout_of(laptop, &caller).join("src/main.rs");
    let asked_main = checkout_of(laptop, thread).join("src/main.rs");
    assert_ne!(caller_main, asked_main, "it works in a checkout of its own");
    let before = fs::read_to_string(&caller_main).expect("the caller's file");
    assert_eq!(before.matches("fn acp() {}").count(), 1, "the caller's own edit only: {before}");
    let theirs = fs::read_to_string(&asked_main).expect("its file");
    assert_eq!(theirs.matches("fn acp() {}").count(), 2, "it started from the caller's edit, and added its own: {theirs}");

    let took = laptop.web("POST", &format!("/api/asks/{id}/take"), None);
    assert!(took.as_str().is_some_and(|said| said.contains("src/main.rs")), "{took}");
    assert!(
        eventually(|| fs::read_to_string(&caller_main).is_ok_and(|main| main.matches("fn acp() {}").count() == 2)),
        "taking brings its edit into the caller's checkout: {}",
        fs::read_to_string(&caller_main).unwrap_or_default()
    );
    assert_eq!(asks(laptop)[0]["taken"], true);
}

#[test]
fn an_unknown_agent_is_refused_with_whom_it_may_ask() {
    let world = world();
    let laptop = &world.laptop;
    let caller = start(laptop, "Ask agent nobody: Review it.");
    let mut said = Vec::new();
    assert!(
        eventually(|| {
            said = history(laptop, &caller);
            said.iter().any(|entry| entry["text"].as_str().is_some_and(|text| text.contains("no agent nobody on this machine")))
        }),
        "the call fails with why: {said:#?}"
    );
    assert!(asks(laptop).is_empty(), "nothing was asked");
}

#[test]
fn a_report_that_comes_after_the_wait_wakes_its_caller() {
    let world = world();
    let laptop = &world.laptop;
    let caller = start(laptop, "Ask agent other: Review src/main.rs, no wait.");
    let mut said = Vec::new();
    assert!(
        eventually(|| {
            said = history(laptop, &caller);
            let answered = said.iter().any(|entry| entry["text"].as_str().is_some_and(|text| text.contains("is still working")));
            // The prompt Silt woke it with carries the report, which the fake repeats.
            let woken = said.iter().any(|entry| {
                entry["kind"] == "wake"
                    && entry["text"] == "The agents it asked reported back, and Silt passed it on to the agent."
            }) && said.iter().any(|entry| {
                entry["kind"] == "assistant"
                    && entry["text"].as_str().is_some_and(|text| text.contains("Fake other reported back on ask"))
            });
            answered && woken && idle(laptop, &caller)
        }),
        "the call answers at once, and the report wakes the caller later: {said:#?}\n{}",
        laptop.daemon_log()
    );
    let count = |kind: &str| said.iter().filter(|entry| entry["kind"] == kind).count();
    assert_eq!((count("user"), count("wake")), (1, 1), "its prompt, then the report, once and not as the person's: {said:#?}");
}

#[test]
fn the_person_stops_an_ask_and_its_caller_is_told() {
    let world = world();
    let laptop = &world.laptop;
    let caller = start(laptop, "Ask agent other: Ponder long over the retry, no wait.");
    assert!(
        eventually(|| asks(laptop).first().is_some_and(|ask| ask["state"] == "running") && idle(laptop, &caller)),
        "the ask runs past its caller's turn: {:#?}",
        asks(laptop)
    );
    let id = asks(laptop)[0]["id"].as_str().expect("an id").to_owned();
    let stopped = laptop.web("POST", &format!("/api/asks/{id}/stop"), None);
    assert_eq!(stopped["state"], "stopped", "{stopped}");
    assert_eq!(stopped["why"], "you stopped it");
    let mut said = Vec::new();
    assert!(
        eventually(|| {
            said = history(laptop, &caller);
            said.iter().any(|entry| {
                entry["kind"] == "assistant"
                    && entry["text"].as_str().is_some_and(|text| text.contains("ended without a report: you stopped it"))
            })
        }),
        "its caller hears how it ended: {said:#?}"
    );
}
