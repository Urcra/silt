//! Silt's ACP host driving the scripted agent.

use std::time::Duration;

use silt_acp::{Command, Event, Start, StopReason, Update, start};
use tokio::sync::mpsc;

async fn next(events: &mut mpsc::UnboundedReceiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(10), events.recv()).await.expect("the agent answers").expect("events keep coming")
}

#[tokio::test]
async fn a_session_streams_asks_permission_and_loads_again() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").expect("write");
    let options = Start {
        program: env!("CARGO_BIN_EXE_silt-fake-agent").into(),
        args: Vec::new(),
        env: Vec::new(),
        cwd: dir.path().to_owned(),
        mcp: None,
        load: None,
    };
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options.clone(), sender);
    let Event::Ready { session, loaded, agent: name, .. } = next(&mut events).await else {
        panic!("the session starts first");
    };
    assert!(!loaded);
    assert_eq!(name, "fake-agent");

    assert!(agent.send(Command::Prompt("add acp".into())));
    let mut seen = Vec::new();
    loop {
        match next(&mut events).await {
            Event::Permission(permission) => {
                assert_eq!(permission.title, "Edit src/main.rs");
                assert!(agent.send(Command::Answer { id: permission.id, option: Some("allow".into()) }));
            }
            Event::TurnEnded { reason } => {
                assert_eq!(reason, StopReason::EndTurn);
                break;
            }
            Event::Update(update) => seen.push(update),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(seen.contains(&Update::Thought("Thinking about the prompt.".into())));
    assert!(
        seen.iter().any(
            |update| matches!(update, Update::ToolCall { title, kind, .. } if title == "Edit src/main.rs" && kind == "edit")
        )
    );
    assert!(seen.iter().any(|update| matches!(update, Update::ToolUpdate { status: Some(status), .. } if status == "completed")));
    assert!(seen.contains(&Update::AgentText("Done: add acp".into())));
    assert_eq!(std::fs::read_to_string(dir.path().join("src/main.rs")).expect("read"), "fn main() {}\nfn acp() {}\n");
    agent.send(Command::Stop);
    while !matches!(next(&mut events).await, Event::Exited) {}

    // This agent keeps sessions in memory, so a new process can't load the
    // session: the host starts a new one instead.
    let (sender, mut events) = mpsc::unbounded_channel();
    let again = start(Start { load: Some(session), ..options }, sender);
    assert!(matches!(next(&mut events).await, Event::Ready { loaded: false, .. }));
    again.send(Command::Stop);
}

fn options(dir: &std::path::Path, env: Vec<(String, String)>) -> Start {
    std::fs::create_dir_all(dir.join("src")).expect("mkdir");
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").expect("write");
    Start {
        program: env!("CARGO_BIN_EXE_silt-fake-agent").into(),
        args: Vec::new(),
        env,
        cwd: dir.to_owned(),
        mcp: None,
        load: None,
    }
}

#[tokio::test]
async fn an_agent_that_dies_is_reported_gone() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), vec![("FAKE_ACP_CRASH".into(), "1".into())]), sender);
    assert!(matches!(next(&mut events).await, Event::Ready { .. }));
    agent.send(Command::Prompt("crash".into()));
    while !matches!(next(&mut events).await, Event::Exited) {}
}

#[tokio::test]
async fn cancelling_answers_the_pending_permission_request() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), Vec::new()), sender);
    assert!(matches!(next(&mut events).await, Event::Ready { .. }));
    agent.send(Command::Prompt("then cancel".into()));
    while !matches!(next(&mut events).await, Event::Permission(_)) {}
    agent.send(Command::Cancel);
    loop {
        match next(&mut events).await {
            Event::TurnEnded { .. } => break,
            Event::Update(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(std::fs::read_to_string(dir.path().join("src/main.rs")).expect("read"), "fn main() {}\n");
    agent.send(Command::Stop);
}

#[tokio::test]
async fn modes_and_settings_come_with_the_session_and_change_on_request() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), Vec::new()), sender);
    let Event::Ready { modes: Some(modes), settings, .. } = next(&mut events).await else {
        panic!("the session starts with its modes");
    };
    assert_eq!(modes.current, "code");
    assert_eq!(silt_acp::plan_mode(&modes).map(|mode| mode.id.as_str()), Some("plan"));
    assert_eq!(settings[0].id, "model");
    assert_eq!(settings[0].current, "fast");
    assert_eq!(settings[0].choices.len(), 2);

    agent.send(Command::SetMode("nonsense".into()));
    assert!(matches!(next(&mut events).await, Event::Refused(_)));
    agent.send(Command::SetSetting { id: "model".into(), value: "smart".into() });
    let Event::Settings(settings) = next(&mut events).await else {
        panic!("the agent says how its settings stand");
    };
    assert_eq!(settings[0].current, "smart");

    // The mode is asked for before the prompt, so the turn runs in it.
    agent.send(Command::SetMode("plan".into()));
    agent.send(Command::Prompt("add acp".into()));
    let mut said = Vec::new();
    loop {
        match next(&mut events).await {
            Event::ModeChanged(mode) => assert_eq!(mode, "plan"),
            Event::Update(Update::AgentText(text)) => said.push(text),
            Event::TurnEnded { .. } => break,
            Event::Update(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(said, ["Plan: add acp"]);
    assert_eq!(std::fs::read_to_string(dir.path().join("src/main.rs")).expect("read"), "fn main() {}\n");
    agent.send(Command::Stop);
}

#[tokio::test]
async fn an_agent_without_modes_says_so() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), vec![("FAKE_ACP_NO_MODES".into(), "1".into())]), sender);
    assert!(matches!(next(&mut events).await, Event::Ready { modes: None, settings, .. } if settings.is_empty()));
    agent.send(Command::Stop);
}
