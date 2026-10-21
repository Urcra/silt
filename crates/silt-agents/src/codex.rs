//! Codex CLI: sessions are rollouts in
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`, one
//! `{timestamp, type, payload}` record per line (docs/research/agents.md).
//! Codex picks the id itself, so a new session is found by its cwd.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::conventions::NativeConventions;
use crate::{
    Agent, Entry, Found, Launch, LaunchPlan, Mark, Read, Reader, RecordedSession, Restore, Rewrites, Role, SessionIds, Start,
    Stateless, Write, translated_text,
};

pub const AGENT: &str = "codex";

/// Written as the version of translated sessions, so Codex reads them as
/// the legacy format that every version resumes.
const TRANSLATED_VERSION: &str = "0.163.0";

pub fn home() -> Option<PathBuf> {
    crate::agent_home("CODEX_HOME", ".codex")
}

/// Every rollout under `sessions/`, newest day first.
fn rollouts(home: &Path) -> Vec<PathBuf> {
    rollouts_from(home, &[])
}

/// The rollouts of days from `since_ms` on. Days are named in local time,
/// so the day before counts too.
fn rollouts_since(home: &Path, since_ms: i64) -> Vec<PathBuf> {
    let stamp = crate::time::file_stamp(since_ms.saturating_sub(24 * 60 * 60 * 1000));
    rollouts_from(home, &[&stamp[..4], &stamp[5..7], &stamp[8..10]])
}

/// Rollouts in the `YYYY/MM/DD` directories that sort at or after `from`,
/// a prefix of such a date.
fn rollouts_from(home: &Path, from: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![(home.join("sessions"), 0, true)];
    while let Some((dir, depth, on_cutoff)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                let name = entry.file_name().to_string_lossy().into_owned();
                match from.get(depth).filter(|_| on_cutoff) {
                    Some(cutoff) if name.as_str() < *cutoff => {}
                    Some(cutoff) => stack.push((path, depth + 1, name == *cutoff)),
                    None => stack.push((path, depth + 1, false)),
                }
            } else if path.extension().is_some_and(|extension| extension == "jsonl")
                && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("rollout-"))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found.reverse();
    found
}

fn rollout_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    // A reverted thread's rollout is `…-<thread>_<rollout>`.
    let stem = stem.split('_').next().unwrap_or(&stem).to_owned();
    // `rollout-YYYY-MM-DDThh-mm-ss-<uuid>`, where a uuid has five parts.
    let parts: Vec<&str> = stem.rsplitn(6, '-').collect();
    (parts.len() == 6).then(|| parts[..5].iter().rev().copied().collect::<Vec<_>>().join("-"))
}

/// The session of a run started in `cwd` no earlier than `since_ms`, in
/// the Codex home `home`: the one that started first since, as a session
/// that started earlier but is still written to belongs to another run.
pub fn discover_in(home: &Path, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_owned());
    let cwd = cwd.to_string_lossy();
    rollouts_since(home, since_ms)
        .into_iter()
        .filter(|path| crate::modified_ms(path).is_some_and(|modified| modified >= since_ms))
        .filter_map(|path| Some((rollout_id(&path)?, path)))
        .filter(|(id, _)| !known.contains(id))
        .filter_map(|(id, path)| {
            let meta = session_meta(&path)?;
            (meta.get("cwd").and_then(Value::as_str) == Some(&*cwd)).then_some(())?;
            let started = meta.get("timestamp").and_then(Value::as_str).and_then(crate::time::parse_timestamp)?;
            (started >= since_ms).then_some((started, id, path))
        })
        .min_by_key(|(started, _, _)| *started)
        .map(|(_, id, transcript)| Found { id, transcript })
}

/// Writes `session` as a rollout in the Codex home `home`. Codex trusts its
/// database's path for an id over any other file, so an existing rollout
/// of this id is replaced in place.
pub fn restore_into(home: &Path, root: &Path, cwd: &Path, session: &RecordedSession) -> io::Result<PathBuf> {
    let existing = rollouts(home).into_iter().find(|path| rollout_id(path).as_deref() == Some(&*session.id));
    let restore = plan_restore(home, existing, root, cwd, session, crate::now_ms());
    restore.apply()?;
    Ok(restore.transcript)
}

/// The rollout restoring `session` writes: over `existing`, the rollout
/// Codex has of this id, or else a new one dated `now_ms`.
pub fn plan_restore(
    home: &Path,
    existing: Option<PathBuf>,
    root: &Path,
    cwd: &Path,
    session: &RecordedSession,
    now_ms: i64,
) -> Restore {
    let path = existing.unwrap_or_else(|| {
        let stamp = crate::time::file_stamp(now_ms);
        home.join("sessions")
            .join(&stamp[..4])
            .join(&stamp[5..7])
            .join(&stamp[8..10])
            .join(format!("rollout-{stamp}-{}.jsonl", session.id))
    });
    let rewrites = Rewrites::new(session, None, &cwd.to_string_lossy(), &root.to_string_lossy());
    Restore { transcript: path.clone(), writes: vec![Write::Transcript { path, lines: rewrites.lines(&session.main) }] }
}

/// The first line's `session_meta` payload.
fn session_meta(path: &Path) -> Option<Value> {
    let value: Value = serde_json::from_str(&crate::first_line(path)?).ok()?;
    (value.get("type")?.as_str()? == "session_meta").then(|| value.get("payload").cloned()).flatten()
}

/// The conversation steps and turn marks in one rollout line.
pub fn read_line(line: &str) -> Read {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Read::unparseable();
    };
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("unknown");
    let payload = value.get("payload").cloned().unwrap_or(Value::Null);
    let field = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    let mut read = Read::default();
    match (kind, payload.get("type").and_then(Value::as_str).unwrap_or_default()) {
        // What the user typed comes from the user-message events: user-role
        // response items also carry the context Codex injects.
        ("response_item", "message") => {
            let entry = match field("role").as_str() {
                "assistant" => Entry::Assistant { text: content_text(payload.get("content")) },
                other => Entry::Other { kind: format!("{other}_message") },
            };
            read.entries.push(entry);
        }
        ("event_msg", "user_message") => read.entries.push(Entry::User { text: field("message") }),
        ("event_msg", "item_completed") if payload.pointer("/item/type").and_then(Value::as_str) == Some("UserMessage") => {
            read.entries.push(Entry::User { text: content_text(payload.pointer("/item/content")) });
        }
        ("response_item", "reasoning") => {
            let text = payload
                .get("summary")
                .and_then(Value::as_array)
                .map(|parts| {
                    parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
                })
                .unwrap_or_default();
            if !text.is_empty() {
                read.entries.push(Entry::Thinking { text });
            }
        }
        ("response_item", call @ ("function_call" | "custom_tool_call" | "local_shell_call")) => {
            let id = field("call_id");
            let (name, input) = match call {
                "function_call" => {
                    let arguments = field("arguments");
                    (field("name"), serde_json::from_str(&arguments).unwrap_or(Value::String(arguments)))
                }
                "custom_tool_call" => (field("name"), Value::String(field("input"))),
                _ => ("local_shell".to_owned(), payload.get("action").cloned().unwrap_or(Value::Null)),
            };
            read.marks.push(Mark::ToolStarted { tool_use_id: id.clone(), tool: name.clone() });
            read.entries.push(Entry::ToolCall { id, name, input });
        }
        ("response_item", "function_call_output" | "custom_tool_call_output") => {
            let id = field("call_id");
            let text = match payload.get("output") {
                Some(Value::String(text)) => text.clone(),
                other => content_text(other),
            };
            read.marks.push(Mark::ToolFinished { tool_use_id: id.clone() });
            read.entries.push(Entry::ToolResult { tool_use_id: id, text, is_error: false });
        }
        ("compacted", _) => {
            let message = field("message");
            read.entries.push(Entry::Compaction { summary: (!message.is_empty()).then_some(message) });
        }
        ("event_msg", "task_started" | "turn_started") => read.marks.push(Mark::PromptSubmitted),
        ("event_msg", "task_complete" | "turn_complete" | "turn_aborted") => read.marks.push(Mark::Stopped),
        (kind, "") => read.entries.push(Entry::Other { kind: kind.to_owned() }),
        (kind, inner) => read.entries.push(Entry::Other { kind: format!("{kind}:{inner}") }),
    }
    read
}

fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => {
            parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
        }
        _ => String::new(),
    }
}

/// Rollout lines holding `messages` as plain text, in the legacy format,
/// with one turn per user message.
pub fn rollout_lines(messages: &[(Role, String)], cwd: &Path, id: &str) -> Vec<String> {
    let timestamp = crate::time::timestamp_now();
    let line = |kind: &str, payload: Value| json!({"timestamp": timestamp, "type": kind, "payload": payload}).to_string();
    let mut lines = vec![line(
        "session_meta",
        json!({
            "id": id,
            "timestamp": timestamp,
            "cwd": cwd.to_string_lossy(),
            "originator": "codex_cli_rs",
            "cli_version": TRANSLATED_VERSION,
            "source": "cli",
            "model_provider": "openai"
        }),
    )];
    let mut turn = 0;
    let mut open = false;
    for (role, text) in messages {
        match role {
            Role::User => {
                if open {
                    lines.push(line(
                        "event_msg",
                        json!({"type": "task_complete", "turn_id": format!("turn-{turn}"), "last_agent_message": null}),
                    ));
                }
                turn += 1;
                open = true;
                lines.push(line(
                    "turn_context",
                    json!({
                        "turn_id": format!("turn-{turn}"),
                        "cwd": cwd.to_string_lossy(),
                        "approval_policy": "on-request",
                        "sandbox_policy": {"type": "workspace-write", "network_access": false},
                        "model": "gpt-5.5",
                        "summary": "auto"
                    }),
                ));
                lines.push(line(
                    "event_msg",
                    json!({"type": "task_started", "turn_id": format!("turn-{turn}"), "model_context_window": null}),
                ));
                lines.push(line(
                    "response_item",
                    json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]}),
                ));
                lines.push(line("event_msg", json!({"type": "user_message", "message": text})));
            }
            Role::Assistant => {
                lines.push(line(
                    "response_item",
                    json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": text}]}),
                ));
                lines.push(line("event_msg", json!({"type": "agent_message", "message": text})));
            }
        }
    }
    if open {
        let last = messages.iter().rev().find(|(role, _)| *role == Role::Assistant).map(|(_, text)| text.clone());
        lines.push(line(
            "event_msg",
            json!({"type": "task_complete", "turn_id": format!("turn-{turn}"), "last_agent_message": last}),
        ));
    }
    lines
}

/// The Codex adapter.
pub struct Codex;

impl Agent for Codex {
    fn id(&self) -> &'static str {
        AGENT
    }

    fn command(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }

    fn native(&self) -> NativeConventions {
        NativeConventions {
            rules: &["AGENTS.md", "AGENTS.override.md"],
            skill_dirs: &[".agents/skills", ".codex/skills"],
            personal_skills: true,
            given_skills: false,
        }
    }

    fn reader(&self) -> Box<dyn Reader> {
        Box::new(Stateless(read_line))
    }

    fn session_ids(&self) -> SessionIds {
        SessionIds::Discovered
    }

    fn transcript(&self, _cwd: &Path, id: &str) -> Option<PathBuf> {
        rollouts(&home()?).into_iter().find(|path| rollout_id(path).as_deref() == Some(id))
    }

    fn discover(&self, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
        discover_in(&home()?, cwd, since_ms, known)
    }

    fn restore(&self, root: &Path, cwd: &Path, session: &RecordedSession) -> io::Result<PathBuf> {
        let home = home().ok_or_else(|| io::Error::other("HOME is not set"))?;
        restore_into(&home, root, cwd, session)
    }

    fn translate(&self, entries: &[Entry], cwd: &Path, id: &str) -> Option<Vec<String>> {
        Some(rollout_lines(&translated_text(entries), cwd, id))
    }

    fn launch(&self, plan: &LaunchPlan) -> io::Result<Launch> {
        let mut launch = Launch::new(self.command());
        let mut prompt = None;
        match &plan.start {
            Start::Resume { id } => {
                launch.arg("resume");
                prompt = Some(id.clone());
            }
            Start::Primed { prompt: text, .. } => prompt = Some(text.clone()),
            Start::New { .. } => {}
        }
        let toml = |value: &Value| value.to_string();
        launch.arg("-c").arg(format!("mcp_servers.silt.command={}", toml(&json!(plan.mcp.command.to_string_lossy()))));
        launch.arg("-c").arg(format!("mcp_servers.silt.args={}", toml(&json!(plan.mcp.args))));
        if let Some(instructions) = &plan.instructions {
            launch.arg("-c").arg(format!("developer_instructions={}", toml(&json!(instructions))));
        }
        for arg in &plan.args {
            launch.arg(arg);
        }
        if let Some(prompt) = prompt {
            launch.arg(prompt);
        }
        Ok(launch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restore_replaces_codex_s_own_rollout_of_the_id_or_dates_a_new_one() {
        let session = RecordedSession { id: "id".into(), main: vec!["{}".into()], ..Default::default() };
        let home = Path::new("/codex");
        let existing = PathBuf::from("/codex/sessions/2026/01/02/rollout-x-id.jsonl");
        let replaced = plan_restore(home, Some(existing.clone()), Path::new("/r"), Path::new("/r"), &session, 0);
        assert_eq!(replaced.transcript, existing);
        let new = plan_restore(home, None, Path::new("/r"), Path::new("/r"), &session, 0);
        assert!(new.transcript.starts_with("/codex/sessions/1970/01/01"), "{}", new.transcript.display());
        assert_eq!(new.writes.len(), 1);
    }

    #[test]
    fn looking_for_new_rollouts_skips_older_days() {
        let home = tempfile::tempdir().expect("temp dir");
        let rollout = |day: &str| {
            let dir = home.path().join("sessions").join(day);
            fs::create_dir_all(&dir).expect("mkdir");
            let path = dir.join("rollout-x.jsonl");
            fs::write(&path, "{}\n").expect("write");
            path
        };
        let (old, month_before, yesterday, today) =
            (rollout("2025/12/31"), rollout("2026/09/30"), rollout("2026/10/20"), rollout("2026/10/21"));
        let next_year = rollout("2027/01/01");
        let october_21_noon = 1_792_584_000_000;
        assert_eq!(rollouts_since(home.path(), october_21_noon), vec![next_year.clone(), today.clone(), yesterday.clone()]);
        assert_eq!(rollouts(home.path()), vec![next_year, today, yesterday, month_before, old]);
    }

    #[test]
    fn reads_messages_tools_turns_and_compaction() {
        let user = r#"{"timestamp":"t","type":"event_msg","payload":{"type":"user_message","message":"List the files"}}"#;
        assert_eq!(read_line(user).entries, vec![Entry::User { text: "List the files".into() }]);
        let paginated = r#"{"timestamp":"t","type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"1","content":[{"type":"text","text":"List the files","text_elements":[]}]}}}"#;
        assert_eq!(read_line(paginated).entries, vec![Entry::User { text: "List the files".into() }]);
        let context = r#"{"timestamp":"t","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context><cwd>/x</cwd></environment_context>"}]}}"#;
        assert_eq!(read_line(context).entries, vec![Entry::Other { kind: "user_message".into() }]);
        let call = r#"{"timestamp":"t","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"ls\"]}","call_id":"call_1"}}"#;
        let read = read_line(call);
        assert_eq!(
            read.entries,
            vec![Entry::ToolCall { id: "call_1".into(), name: "shell".into(), input: json!({"command": ["ls"]}) }]
        );
        assert_eq!(read.marks, vec![Mark::ToolStarted { tool_use_id: "call_1".into(), tool: "shell".into() }]);
        let output = r#"{"timestamp":"t","type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":"README.md\n"}}"#;
        assert_eq!(read_line(output).marks, vec![Mark::ToolFinished { tool_use_id: "call_1".into() }]);
        let reasoning = r#"{"timestamp":"t","type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"Checking"}],"encrypted_content":"x"}}"#;
        assert_eq!(read_line(reasoning).entries, vec![Entry::Thinking { text: "Checking".into() }]);
        let started = r#"{"timestamp":"t","type":"event_msg","payload":{"type":"task_started","turn_id":"1"}}"#;
        assert_eq!(read_line(started).marks, vec![Mark::PromptSubmitted]);
        let complete = r#"{"timestamp":"t","type":"event_msg","payload":{"type":"task_complete","turn_id":"1"}}"#;
        assert_eq!(read_line(complete).marks, vec![Mark::Stopped]);
        let compacted = r#"{"timestamp":"t","type":"compacted","payload":{"message":"We listed files."}}"#;
        assert_eq!(read_line(compacted).entries, vec![Entry::Compaction { summary: Some("We listed files.".into()) }]);
    }

    #[test]
    fn rollout_names_carry_the_id() {
        let path = Path::new("/h/sessions/2026/10/20/rollout-2026-10-20T12-00-00-0199f3a1-7c2e-7b1a-9d3e-5a6b7c8d9e01.jsonl");
        assert_eq!(rollout_id(path).as_deref(), Some("0199f3a1-7c2e-7b1a-9d3e-5a6b7c8d9e01"));
        let reverted = Path::new(
            "/h/sessions/2026/10/20/rollout-2026-10-20T12-00-00-0199f3a1-7c2e-7b1a-9d3e-5a6b7c8d9e01_0199f3a1-0000-7b1a-9d3e-5a6b7c8d9e02.jsonl",
        );
        assert_eq!(rollout_id(reverted).as_deref(), Some("0199f3a1-7c2e-7b1a-9d3e-5a6b7c8d9e01"));
    }

    #[test]
    fn translation_is_a_legacy_rollout_that_reads_back() {
        let lines = rollout_lines(
            &[(Role::User, "Fix it".into()), (Role::Assistant, "Fixed.".into())],
            Path::new("/repo"),
            "0199f3a1-7c2e-7b1a-9d3e-5a6b7c8d9e01",
        );
        let meta: Value = serde_json::from_str(&lines[0]).expect("json");
        assert_eq!(meta["payload"]["cwd"], "/repo");
        assert!(meta["payload"].get("history_mode").is_none());
        let entries: Vec<Entry> =
            lines.iter().flat_map(|line| read_line(line).entries).filter(|entry| !matches!(entry, Entry::Other { .. })).collect();
        assert_eq!(entries, vec![Entry::User { text: "Fix it".into() }, Entry::Assistant { text: "Fixed.".into() }]);
        assert!(lines.last().expect("lines").contains("task_complete"));
    }
}
