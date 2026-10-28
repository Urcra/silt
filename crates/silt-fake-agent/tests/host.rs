//! Silt's ACP host driving the scripted agent.

use std::time::Duration;

use silt_acp::{Accepts, Attachment, Command, Event, Start, StopReason, Update, start};
use tokio::sync::mpsc;

fn prompt(text: &str) -> Command {
    Command::Prompt { text: text.into(), attachments: Vec::new() }
}

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

    assert!(agent.send(prompt("add acp")));
    let mut seen = Vec::new();
    let mut context = None;
    let mut used = None;
    loop {
        match next(&mut events).await {
            Event::Context { used, size } => context = Some((used, size)),
            Event::TurnUsage { input, output } => used = Some((input, output)),
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
    assert_eq!(context, Some((53_000, 200_000)));
    assert_eq!(used, Some((1_600, 240)));
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
async fn an_agent_that_answers_v2_with_v1_is_spoken_to_in_v1() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), Vec::new()), sender);
    assert!(matches!(next(&mut events).await, Event::Ready { reports_work: false, .. }));
    agent.send(Command::Stop);
}

/// What an agent speaking v2 does over one prompt, up to its stop.
async fn turn_v2(events: &mut mpsc::UnboundedReceiver<Event>, agent: &silt_acp::AgentHandle) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = next(events).await;
        if let Event::Permission(permission) = &event {
            assert_eq!(permission.title, "Edit src/main.rs");
            assert_eq!((permission.tool_call_id.as_str(), permission.tool_kind.as_deref()), ("edit-1", Some("edit")));
            assert!(agent.send(Command::Answer { id: permission.id, option: Some("allow".into()) }));
        }
        let ended = matches!(event, Event::TurnEnded { .. });
        seen.push(event);
        if ended {
            return seen;
        }
    }
}

#[tokio::test]
async fn an_agent_speaking_v2_says_when_it_works_also_on_what_it_takes_up_by_itself() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), vec![("FAKE_ACP_V2".into(), "1".into())]), sender);
    let Event::Ready { modes: Some(modes), settings, reports_work: true, loaded: false, agent: name, .. } =
        next(&mut events).await
    else {
        panic!("the session starts in v2, with its mode among its settings");
    };
    assert_eq!(name, "fake-agent");
    assert_eq!(modes.current, "code");
    assert_eq!(settings.iter().map(|setting| setting.id.as_str()).collect::<Vec<_>>(), ["mode", "model"]);

    assert!(agent.send(prompt("add acp Later")));
    let turn = turn_v2(&mut events, &agent).await;
    assert!(matches!(turn[0], Event::Working));
    assert!(
        turn.iter().any(|event| matches!(event, Event::Update(Update::Thought(text)) if text == "Thinking about the prompt."))
    );
    let output = turn.iter().find_map(|event| match event {
        Event::Update(Update::ToolUpdate { id, status: Some(status), output, .. }) if id == "run-1" && status == "completed" => {
            output.clone()
        }
        _ => None,
    });
    assert_eq!(output.as_deref(), Some("fn main() {}\n"), "a command's result is what its terminal printed");
    assert!(turn.iter().any(|event| matches!(event, Event::Update(Update::AgentText(text)) if text == "Done: add acp Later")));
    assert!(matches!(
        &turn[turn.len() - 2..],
        [Event::TurnUsage { input: 1_600, output: 240 }, Event::TurnEnded { reason: StopReason::EndTurn }]
    ));
    assert_eq!(std::fs::read_to_string(dir.path().join("src/main.rs")).expect("read"), "fn main() {}\nfn acp() {}\n");

    // Nobody prompted it: it went on by itself and says so, and when it stops.
    let own = turn_v2(&mut events, &agent).await;
    assert!(matches!(own[0], Event::Working));
    assert!(own.iter().any(|event| matches!(event, Event::Update(Update::AgentText(text)) if text == "Went on by itself.")));
    assert!(matches!(own.last(), Some(Event::TurnEnded { reason: StopReason::EndTurn })));

    agent.send(Command::SetMode("plan".into()));
    assert!(matches!(next(&mut events).await, Event::ModeChanged(mode) if mode == "plan"));
    assert!(matches!(next(&mut events).await, Event::Settings(settings) if settings[0].current == "plan"));
    agent.send(Command::SetMode("nonsense".into()));
    assert!(matches!(next(&mut events).await, Event::Refused(_)));
    agent.send(Command::Stop);
    while !matches!(next(&mut events).await, Event::Exited) {}
}

#[tokio::test]
async fn an_agent_that_dies_is_reported_gone() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), vec![("FAKE_ACP_CRASH".into(), "1".into())]), sender);
    assert!(matches!(next(&mut events).await, Event::Ready { .. }));
    agent.send(prompt("crash"));
    while !matches!(next(&mut events).await, Event::Exited) {}
}

#[tokio::test]
async fn an_agent_stuck_starting_is_stopped_all_the_same() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), vec![("FAKE_ACP_HANG".into(), "1".into())]), sender);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(events.try_recv().is_err(), "it never answers");
    agent.send(Command::Stop);
    while !matches!(next(&mut events).await, Event::Exited) {}
}

#[tokio::test]
async fn cancelling_answers_the_pending_permission_request() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), Vec::new()), sender);
    assert!(matches!(next(&mut events).await, Event::Ready { .. }));
    agent.send(prompt("then cancel"));
    while !matches!(next(&mut events).await, Event::Permission(_)) {}
    agent.send(Command::Cancel);
    loop {
        match next(&mut events).await {
            Event::TurnEnded { .. } => break,
            Event::Update(_) | Event::Context { .. } | Event::TurnUsage { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(std::fs::read_to_string(dir.path().join("src/main.rs")).expect("read"), "fn main() {}\n");
    agent.send(Command::Stop);
}

#[tokio::test]
async fn a_question_is_asked_as_a_form_and_answered_with_a_choice() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), Vec::new()), sender);
    assert!(matches!(next(&mut events).await, Event::Ready { .. }));
    agent.send(prompt("Ask me what to count"));
    let question = loop {
        match next(&mut events).await {
            Event::Question(question) => break question,
            Event::Update(_) | Event::Context { .. } | Event::TurnUsage { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    };
    assert_eq!(question.message, "What should the progress bar count?");
    let field = &question.fields[0];
    assert_eq!((field.key.as_str(), field.multi, field.options.len()), ("count", false, 3));
    assert_eq!(field.options[1].label, "Bytes sent on the socket");
    assert_eq!(field.options[1].description.as_deref(), Some("Smooth, but can run ahead of the server"));
    agent.send(Command::AnswerQuestion { id: question.id, answers: Some(vec![("count".into(), vec!["sent".into()])]) });
    let mut said = Vec::new();
    loop {
        match next(&mut events).await {
            Event::Update(Update::AgentText(text)) => said.push(text),
            Event::TurnEnded { .. } => break,
            Event::Update(_) | Event::Context { .. } | Event::TurnUsage { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(said, ["Counting sent."]);
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
    agent.send(prompt("add acp"));
    let mut said = Vec::new();
    loop {
        match next(&mut events).await {
            Event::ModeChanged(mode) => assert_eq!(mode, "plan"),
            Event::Update(Update::AgentText(text)) => said.push(text),
            Event::TurnEnded { .. } => break,
            Event::Update(_) | Event::Context { .. } | Event::TurnUsage { .. } => {}
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
    assert!(matches!(
        next(&mut events).await,
        Event::Ready { modes: None, settings, accepts, .. } if settings.is_empty() && accepts == Accepts::default()
    ));
    agent.send(Command::Stop);
}

#[tokio::test]
async fn attachments_reach_the_agent_as_it_takes_them() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (sender, mut events) = mpsc::unbounded_channel();
    let agent = start(options(dir.path(), Vec::new()), sender);
    let Event::Ready { accepts, .. } = next(&mut events).await else {
        panic!("the session starts first");
    };
    assert_eq!(accepts, Accepts { image: true, embedded: true });
    let attachments = vec![
        Attachment { name: "shot.png".into(), mime: "image/png".into(), data: vec![0x89, b'P', b'N', b'G'] },
        Attachment { name: "notes.txt".into(), mime: "text/plain".into(), data: b"hi".to_vec() },
    ];
    agent.send(Command::Prompt { text: "look".into(), attachments });
    loop {
        match next(&mut events).await {
            Event::Permission(permission) => {
                agent.send(Command::Answer { id: permission.id, option: Some("allow".into()) });
            }
            Event::Update(Update::AgentText(text)) => {
                assert_eq!(text, r#"Done: look (with an image (image/png), attachment:notes.txt saying "hi")"#);
                break;
            }
            _ => {}
        }
    }
    agent.send(Command::Stop);
}
