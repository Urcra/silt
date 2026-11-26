//! Auto for an agent without an auto mode of its own (ADR 0132): a fake
//! agent with no `auto` mode asks to run commands, and another session of
//! it judges them.

mod common;

use std::fs;

use common::{Machine, Server, eventually, origin};

/// Fields drop in order: the machine stops its daemon before the
/// directories holding its socket go away.
struct World {
    laptop: Machine,
    _server: Server,
    _root: tempfile::TempDir,
}

fn world() -> World {
    let root = common::temp_root("silt-auto");
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
        Some(serde_json::json!({"clone": machine.repo, "agent": "v1", "prompt": prompt, "preset": "auto"})),
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

fn agent(machine: &Machine, thread: &str) -> serde_json::Value {
    machine.web("GET", &format!("/api/threads/{thread}/live"), None)["agent"].clone()
}

fn prompt(machine: &Machine, thread: &str, text: &str) {
    machine.web("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": text})));
}

fn said(entries: &[serde_json::Value], words: &str) -> bool {
    entries.iter().any(|entry| entry["kind"] == "assistant" && entry["text"].as_str().is_some_and(|text| text.contains(words)))
}

fn verdicts(entries: &[serde_json::Value]) -> Vec<serde_json::Value> {
    entries.iter().filter(|entry| entry["kind"] == "auto").map(|entry| entry["auto"].clone()).collect()
}

/// Waits for the agent to be idle with `words` said, failing with what it
/// had recorded.
fn wait_for(machine: &Machine, thread: &str, words: &str) -> Vec<serde_json::Value> {
    let mut entries = Vec::new();
    assert!(
        eventually(|| {
            entries = history(machine, thread);
            said(&entries, words) && agent(machine, thread)["state"] == "idle"
        }),
        "waited for {words:?}: {entries:#?}\n{}",
        machine.daemon_log()
    );
    entries
}

#[test]
fn auto_lets_a_safe_command_through_and_refuses_a_destructive_one_with_why() {
    let world = world();
    let laptop = &world.laptop;
    let thread = start(laptop, "Run `cargo test -p upload`");
    let entries = wait_for(laptop, &thread, "Ran cargo test -p upload.");
    let allowed = verdicts(&entries);
    assert_eq!(allowed.len(), 1, "{allowed:#?}");
    assert_eq!(allowed[0]["verdict"], "allow");
    assert_eq!(allowed[0]["call"], "run-1");
    assert_eq!(allowed[0]["judge"], "Fake v1 on Fast", "the thread's own agent on its model judges");
    assert!(allowed[0]["reason"].as_str().is_some_and(|reason| reason.contains("does what you asked")));

    prompt(laptop, &thread, "Run `rm -rf ~/.cache`");
    let entries = wait_for(laptop, &thread, "I was not allowed to run rm -rf ~/.cache.");
    let refused = verdicts(&entries);
    assert_eq!(refused.last().expect("a verdict")["verdict"], "refuse");
    // Told why, as a message from Silt once its turn ended, as this agent takes no steers.
    assert!(
        eventually(|| history(laptop, &thread).iter().any(|entry| entry["kind"] == "user"
            && entry["text"].as_str().is_some_and(|text| text.contains("Silt's Auto check refused `rm -rf ~/.cache`")))),
        "the agent is told why: {:#?}",
        history(laptop, &thread)
    );

    prompt(laptop, &thread, "Run `cargo test -p upload`");
    assert!(
        eventually(|| verdicts(&history(laptop, &thread)).last().is_some_and(|last| last["verdict"] == "before")),
        "the same call goes through again without its judge: {:#?}",
        verdicts(&history(laptop, &thread))
    );
}

#[test]
fn a_check_without_a_verdict_leaves_the_call_to_the_person() {
    let world = world();
    let laptop = &world.laptop;
    let thread = start(laptop, "Run `echo unreadable`");
    let mut asked = serde_json::Value::Null;
    assert!(
        eventually(|| {
            asked = agent(laptop, &thread)["permissions"][0].clone();
            asked["unchecked"].is_string()
        }),
        "the request waits for the person, saying why: {asked:#?}\n{}",
        laptop.daemon_log()
    );
    assert!(asked["unchecked"].as_str().is_some_and(|why| why.contains("wasn't a verdict")), "{asked:#?}");
    assert_eq!(agent(laptop, &thread)["state"], "awaiting_permission");
    laptop.web(
        "POST",
        &format!("/api/threads/{thread}/agent/answer"),
        Some(serde_json::json!({"id": asked["id"], "option": "allow"})),
    );
    let entries = wait_for(laptop, &thread, "Ran echo unreadable.");
    assert_eq!(verdicts(&entries)[0]["verdict"], "unchecked");
}

#[test]
fn the_agent_asks_the_person_to_allow_a_refused_call_once() {
    let world = world();
    let laptop = &world.laptop;
    let thread = start(laptop, "Run `git push --force origin fix` and escalate if refused");
    let mut escalated = serde_json::Value::Null;
    assert!(
        eventually(|| {
            escalated = agent(laptop, &thread)["escalations"][0].clone();
            escalated.is_object()
        }),
        "the escalation waits for the person: {:#?}\n{}",
        history(laptop, &thread),
        laptop.daemon_log()
    );
    assert_eq!(escalated["call"], "git push --force origin fix");
    assert_eq!(escalated["why"], "The person asked for it.");
    assert!(escalated["refused"].as_str().is_some_and(|why| why.contains("rewrites")));
    assert_eq!(agent(laptop, &thread)["state"], "awaiting_permission", "it waits on the person");
    laptop.web(
        "POST",
        &format!("/api/threads/{thread}/agent/escalation"),
        Some(serde_json::json!({"id": escalated["id"], "allow": true, "words": "Fine, it's your branch."})),
    );
    let entries = wait_for(laptop, &thread, "Ran it after all.");
    assert!(said(&entries, "it's your branch"), "the person's words reach the agent: {entries:#?}");
    let last = verdicts(&entries).pop().expect("a verdict");
    assert_eq!(last["verdict"], "granted", "{last:#?}");
    assert_eq!(last["call"], "run-2");
}

#[test]
fn a_thread_chooses_another_agent_to_check_for_it() {
    let world = world();
    let laptop = &world.laptop;
    let thread = start(laptop, "Which model");
    wait_for(laptop, &thread, "Model: fast");
    assert_eq!(agent(laptop, &thread)["auto_check"]["judge"]["kind"], "thread");
    laptop.web(
        "POST",
        &format!("/api/threads/{thread}/agent/judge"),
        Some(serde_json::json!({"judge": {"kind": "agent", "agent": "other", "model": "smart"}})),
    );
    assert_eq!(agent(laptop, &thread)["auto_check"]["chosen"]["agent"], "other");
    prompt(laptop, &thread, "Run `cargo build`");
    let entries = wait_for(laptop, &thread, "Ran cargo build.");
    let judged = verdicts(&entries);
    // The model's name is known once the agent has run here; until then, its id.
    let judge = judged[0]["judge"].as_str().unwrap_or_default();
    assert!(["Fake other on Smart", "Fake other on smart"].contains(&judge), "{judged:#?}");
    assert!(judged[0]["reason"].as_str().is_some_and(|reason| reason.contains("judged on smart")), "on the model chosen");
}
