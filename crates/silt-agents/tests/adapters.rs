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
    let cwd = silt_os::canonicalize(dir.path()).expect("canonical");
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
fn qudiks_sessions_are_codex_rollouts_in_its_own_home() {
    let agent = silt_agents::agent("qudiks").expect("qudiks");
    let (home, (_dir, cwd)) = (tempfile::tempdir().expect("temp dir"), cwd());
    let id = "019a0000-0000-7000-8000-000000000002";
    let transcript = silt_agents::codex::restore_into(home.path(), &cwd, &cwd, &recorded(agent, &cwd, id)).expect("restores");
    finds_as_a_new_run(|since, known| silt_agents::codex::discover_in(home.path(), &cwd, since, known), id, &transcript);
    assert_eq!(
        read_back(agent, &transcript),
        [Entry::User { text: "Fix the lexer".into() }, Entry::Assistant { text: "Fixed.".into() }]
    );
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

#[test]
fn antigravity_conversations_are_found_by_where_its_index_says_they_run() {
    let (app, (_dir, cwd)) = (tempfile::tempdir().expect("temp dir"), cwd());
    let app = app.path();
    let index = rusqlite::Connection::open(app.join("conversation_summaries.db")).expect("index");
    index
        .execute_batch(
            "CREATE TABLE conversation_summaries (conversation_id text PRIMARY KEY, title text NOT NULL DEFAULT '', \
             workspace_uris text NOT NULL, parent_conversation_id text NOT NULL DEFAULT '')",
        )
        .expect("schema");
    let uri = format!("file://{}", cwd.to_string_lossy().replace(' ', "%20"));
    let (main, sub, elsewhere) =
        ("6fa48f18-aae5-426b-82a7-92400134ab4f", "39497b42-d96f-4954-855d-69dd29a7d6f3", "de4301f9-46e5-43e1-b9a6-b8b03d007caf");
    for (id, uris, parent) in [
        (main, serde_json::json!([uri]), ""),
        (sub, serde_json::json!([uri]), main),
        (elsewhere, serde_json::json!(["file:///somewhere/else"]), ""),
    ] {
        index
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, workspace_uris, parent_conversation_id) VALUES (?1, ?2, ?3)",
                (id, uris.to_string(), parent),
            )
            .expect("row");
        let transcript = app.join("brain").join(id).join(".system_generated/logs/transcript_full.jsonl");
        fs::create_dir_all(transcript.parent().expect("dir")).expect("mkdir");
        fs::write(
            &transcript,
            "{\"step_index\":0,\"type\":\"USER_INPUT\",\"created_at\":\"2026-11-13T07:51:40Z\",\"content\":\"hi\"}\n",
        )
        .expect("transcript");
    }
    let transcript = app.join("brain").join(main).join(".system_generated/logs/transcript_full.jsonl");
    finds_as_a_new_run(|since, known| silt_agents::antigravity::discover_in(app, &cwd, since, known), main, &transcript);

    // Started with `-i` in a folder agy doesn't trust yet, a conversation
    // names no workspace; it is found only when none names the folder.
    let unplaced = "0c1f0000-0000-4000-8000-000000000004";
    index
        .execute("INSERT INTO conversation_summaries (conversation_id, workspace_uris) VALUES (?1, '')", [unplaced])
        .expect("row");
    let untrusted = app.join("brain").join(unplaced).join(".system_generated/logs/transcript_full.jsonl");
    fs::create_dir_all(untrusted.parent().expect("dir")).expect("mkdir");
    fs::write(&untrusted, "{}\n").expect("transcript");
    let since = silt_agents::time::parse_timestamp(&silt_agents::time::timestamp_now()).expect("now") - 60_000;
    let found = |known: &[String]| silt_agents::antigravity::discover_in(app, &cwd, since, known).map(|found| found.id);
    assert_eq!(found(&[]).as_deref(), Some(main), "one that names the folder comes first");
    assert_eq!(found(&[main.to_owned()]).as_deref(), Some(unplaced));
    index.execute("DELETE FROM conversation_summaries WHERE conversation_id = ?1", [unplaced]).expect("delete");

    let on_disk = silt_agents::antigravity::on_disk_in(app);
    let found: Vec<(&str, Option<&Path>)> = on_disk.iter().map(|found| (found.id.as_str(), found.cwd.as_deref())).collect();
    assert_eq!(
        found,
        [(main, Some(cwd.as_path())), (elsewhere, Some(Path::new("/somewhere/else")))],
        "subagents' aren't imported"
    );
    assert_eq!(
        silt_agents::antigravity::subagents_in(app, &transcript),
        [(sub.to_owned(), app.join("brain").join(sub).join(".system_generated/logs/transcript_full.jsonl"))]
    );
    assert!(!silt_agents::antigravity::resumable_in(app, main), "agy keeps no store of it here");
    fs::create_dir_all(app.join("conversations")).expect("mkdir");
    fs::write(app.join("conversations").join(format!("{main}.db")), b"").expect("store");
    assert!(silt_agents::antigravity::resumable_in(app, main));
}
