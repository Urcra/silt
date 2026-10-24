//! Each agent's session, translated, restored into a home of its own and
//! found again the way a new run is found.

use std::fs;
use std::path::{Path, PathBuf};

use silt_agents::{Agent, CheckedSession, Entry, Found, RecordedSession};

fn recorded(agent: &dyn Agent, cwd: &Path, id: &str) -> CheckedSession {
    let entries = [Entry::User { text: "Fix the lexer".into() }, Entry::Assistant { text: "Fixed.".into() }];
    RecordedSession { id: id.into(), main: agent.translate(&entries, cwd, id).expect("translates"), ..Default::default() }
        .checked()
        .expect("safe")
}

/// What the agent reads back from the file, as conversation.
fn read_back(agent: &dyn Agent, transcript: &Path) -> Vec<Entry> {
    let mut reader = agent.reader();
    fs::read_to_string(transcript)
        .expect("reads")
        .lines()
        .flat_map(|line| reader.push(line).entries)
        .filter(|entry| matches!(entry, Entry::User { .. } | Entry::Assistant { .. }))
        .collect()
}

fn cwd() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let cwd = dir.path().canonicalize().expect("canonical");
    (dir, cwd)
}

/// A restored session is found as a new run's, unless it is known or older
/// than the run.
fn finds_as_a_new_run(found: impl Fn(i64, &[String]) -> Option<Found>, id: &str, transcript: &Path) {
    let since = silt_agents::time::parse_timestamp(&silt_agents::time::timestamp_now()).expect("now") - 60_000;
    let first = found(since, &[]).expect("found");
    assert_eq!((first.id.as_str(), first.transcript.as_path()), (id, transcript));
    assert!(found(since, &[id.to_owned()]).is_none(), "a known session isn't found again");
    assert!(found(since + 3_600_000, &[]).is_none(), "nor one from before the run");
}

#[test]
fn codex_sessions_are_restored_found_and_read_back() {
    let agent = silt_agents::agent("codex").expect("codex");
    let (home, (_dir, cwd)) = (tempfile::tempdir().expect("temp dir"), cwd());
    let id = "019a0000-0000-7000-8000-000000000001";
    let transcript = silt_agents::codex::restore_into(home.path(), &cwd, &cwd, &recorded(agent, &cwd, id)).expect("restores");
    assert!(transcript.starts_with(home.path().join("sessions")));
    finds_as_a_new_run(|since, known| silt_agents::codex::discover_in(home.path(), &cwd, since, known), id, &transcript);
    assert_eq!(
        read_back(agent, &transcript),
        [Entry::User { text: "Fix the lexer".into() }, Entry::Assistant { text: "Fixed.".into() }]
    );
}

#[test]
fn copilot_sessions_are_restored_found_and_read_back() {
    let agent = silt_agents::agent("copilot-cli").expect("copilot");
    let (home, (_dir, cwd)) = (tempfile::tempdir().expect("temp dir"), cwd());
    let id = "6c1f0000-0000-4000-8000-000000000002";
    let transcript = silt_agents::copilot::restore_into(home.path(), &cwd, &cwd, &recorded(agent, &cwd, id)).expect("restores");
    // Quoted as YAML takes a JSON string, so Windows' backslashes come doubled.
    let quoted = serde_json::to_string(&cwd.to_string_lossy()).expect("a string");
    assert!(fs::read_to_string(transcript.with_file_name("workspace.yaml")).expect("workspace").contains(&quoted));
    finds_as_a_new_run(|since, known| silt_agents::copilot::discover_in(home.path(), &cwd, since, known), id, &transcript);
    assert_eq!(
        read_back(agent, &transcript),
        [Entry::User { text: "Fix the lexer".into() }, Entry::Assistant { text: "Fixed.".into() }]
    );
}

#[test]
fn gemini_sessions_are_restored_found_and_read_back() {
    let agent = silt_agents::agent("gemini-cli").expect("gemini");
    let (gemini, (_dir, cwd)) = (tempfile::tempdir().expect("temp dir"), cwd());
    let id = "gemini-session-3";
    let transcript = silt_agents::gemini::restore_into(gemini.path(), &cwd, &cwd, &recorded(agent, &cwd, id)).expect("restores");
    finds_as_a_new_run(|since, known| silt_agents::gemini::discover_in(gemini.path(), &cwd, since, known), id, &transcript);
    assert_eq!(read_back(agent, &transcript)[0], Entry::User { text: "Fix the lexer".into() });
    assert!(read_back(agent, &transcript).contains(&Entry::Assistant { text: "Fixed.".into() }));
}
