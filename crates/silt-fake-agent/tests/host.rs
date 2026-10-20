//! Silt's ACP host driving the scripted agent.

use std::time::Duration;

use silt_acp::{Command, Event, Start, Update, start};
use tokio::sync::mpsc;

async fn next(events: &mut mpsc::UnboundedReceiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("the agent answers")
        .expect("events keep coming")
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
    let Event::Ready { session, loaded, agent: name } = next(&mut events).await else {
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
                assert_eq!(reason, "end_turn");
                break;
            }
            Event::Update(update) => seen.push(update),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(seen.contains(&Update::Thought("Thinking about the prompt.".into())));
    assert!(seen.iter().any(|update| matches!(update, Update::ToolCall { title, kind, .. } if title == "Edit src/main.rs" && kind == "edit")));
    assert!(seen.iter().any(|update| matches!(update, Update::ToolUpdate { status: Some(status), .. } if status == "completed")));
    assert!(seen.contains(&Update::AgentText("Done: add acp".into())));
    assert_eq!(std::fs::read_to_string(dir.path().join("src/main.rs")).expect("read"), "fn main() {}\nfn acp() {}\n");
    agent.send(Command::Stop);
    while !matches!(next(&mut events).await, Event::Exited) {}

    // A new process can load the session only if the agent kept it; this
    // one keeps sessions in memory, so it starts a new one when it can't.
    let (sender, mut events) = mpsc::unbounded_channel();
    let again = start(Start { load: Some(session), ..options }, sender);
    match next(&mut events).await {
        Event::Ready { loaded, .. } => assert!(loaded, "the agent says it loads sessions"),
        Event::Failed(_) => {}
        other => panic!("unexpected {other:?}"),
    }
    again.send(Command::Stop);
}
