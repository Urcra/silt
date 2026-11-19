//! Codex CLI: sessions are rollouts in
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`, one
//! `{timestamp, type, payload}` record per line (docs/research/agents.md).
//! Codex picks the id itself, so a new session is found by its cwd.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::conventions::NativeConventions;
use crate::{
    Agent, AgentError, CheckedSession, Entry, Found, Launch, LaunchPlan, Mark, OnDisk, Read, Reader, Restore, Rewrites, Role,
    SessionIds, Start, Task, TaskKind, TaskStatus, Write, translated_text,
};

pub const AGENT: &str = "codex";

/// Written as the version of translated sessions, so Codex reads them as
/// the legacy format that every version resumes.
const TRANSLATED_VERSION: &str = "0.163.0";

pub fn home() -> Option<PathBuf> {
    crate::agent_home("CODEX_HOME", ".codex")
}

/// [`home`] for a session in `cwd`, whose workspace may set its own (ADR 0065).
fn home_in(cwd: &Path) -> Option<PathBuf> {
    crate::agent_home_in("CODEX_HOME", ".codex", cwd)
}

/// Every rollout under `sessions/`, newest day first.
fn rollouts(home: &Path) -> Vec<PathBuf> {
    rollouts_from(home, &[])
}

/// The rollouts of days from `since_ms` on. Days are named in local time,
/// so the day before counts too.
pub(crate) fn rollouts_since(home: &Path, since_ms: i64) -> Vec<PathBuf> {
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
    let cwd = silt_os::canonicalize(cwd).unwrap_or_else(|_| cwd.to_owned());
    let cwd = cwd.to_string_lossy();
    rollouts_since(home, since_ms)
        .into_iter()
        .filter(|path| crate::modified_ms(path).is_some_and(|modified| modified >= since_ms))
        .filter_map(|path| Some((rollout_id(&path)?, path)))
        .filter(|(id, _)| !known.contains(id))
        .filter_map(|(id, path)| {
            let meta = session_meta(&path)?;
            (meta.get("cwd").and_then(Value::as_str) == Some(&*cwd) && parent_of(&meta).is_none()).then_some(())?;
            let started = meta.get("timestamp").and_then(Value::as_str).and_then(crate::time::parse_timestamp)?;
            (started >= since_ms).then_some((started, id, path))
        })
        .min_by_key(|(started, _, _)| *started)
        .map(|(_, id, transcript)| Found { id, transcript })
}

/// Every rollout in the Codex home `home`, with where and when it started.
pub fn on_disk_in(home: &Path) -> Vec<OnDisk> {
    let mut found: Vec<OnDisk> = rollouts(home)
        .into_iter()
        .filter_map(|path| {
            let id = rollout_id(&path)?;
            let meta = session_meta(&path);
            // A subagent's rollout is read with its parent's session.
            meta.as_ref().and_then(parent_of).is_none().then_some(())?;
            let field = |key: &str| meta.as_ref()?.get(key)?.as_str().map(str::to_owned);
            let cwd = field("cwd").map(PathBuf::from);
            let started_ms = field("timestamp").as_deref().and_then(crate::time::parse_timestamp);
            Some(OnDisk { id, transcript: path, cwd, started_ms })
        })
        .collect();
    found.sort_by(|a, b| a.started_ms.cmp(&b.started_ms).then_with(|| a.id.cmp(&b.id)));
    // A reverted thread's rollout shares its id; the first is the session.
    found.dedup_by(|later, earlier| later.id == earlier.id);
    found
}

/// Writes `session` as a rollout in the Codex home `home`. Codex trusts its
/// database's path for an id over any other file, so an existing rollout
/// of this id is replaced in place.
pub fn restore_into(home: &Path, root: &Path, cwd: &Path, session: &CheckedSession) -> Result<PathBuf, AgentError> {
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
    session: &CheckedSession,
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

/// The rollout of session `id` in the Codex home `home`.
pub fn transcript_in(home: &Path, id: &str) -> Option<PathBuf> {
    rollouts(home).into_iter().find(|path| rollout_id(path).as_deref() == Some(id))
}

/// The first line's `session_meta` payload.
fn session_meta(path: &Path) -> Option<Value> {
    let value: Value = serde_json::from_str(&crate::first_line(path)?).ok()?;
    (value.get("type")?.as_str()? == "session_meta").then(|| value.get("payload").cloned()).flatten()
}

/// The thread a subagent's rollout says spawned it.
fn parent_of(meta: &Value) -> Option<&str> {
    meta.get("parent_thread_id")
        .or_else(|| meta.pointer("/source/subagent/thread_spawn/parent_thread_id"))
        .and_then(Value::as_str)
}

/// The rollouts of the subagents `transcript`'s session spawned, by their
/// thread id: those from its day on whose `session_meta` names it.
pub fn subagent_rollouts(transcript: &Path) -> Vec<(String, PathBuf)> {
    let Some(id) = rollout_id(transcript) else {
        return Vec::new();
    };
    let day: Vec<String> = transcript
        .ancestors()
        .skip(1)
        .take(3)
        .filter_map(|dir| dir.file_name().map(|name| name.to_string_lossy().into_owned()))
        .collect();
    let (Some(home), [day, month, year]) = (transcript.ancestors().nth(5), day.as_slice()) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = rollouts_from(home, &[year, month, day])
        .into_iter()
        // A rollout's name starts with when it started, and a subagent starts after its parent.
        .filter(|path| path != transcript && started_name(path) >= started_name(transcript))
        .filter_map(|path| {
            let child = rollout_id(&path).filter(|child| crate::is_safe_id(child))?;
            (parent_of(&session_meta(&path)?)? == id).then_some((child, path))
        })
        .collect();
    found.sort();
    found
}

/// `rollout-YYYY-MM-DDThh-mm-ss`, the part of a rollout's name that says
/// when it started.
fn started_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().chars().take(27).collect()).unwrap_or_default()
}

/// Reads rollout lines, remembering which background session each
/// `write_stdin` call wrote to, as its output doesn't say.
#[derive(Default)]
pub struct CodexReader {
    stdin: HashMap<String, String>,
}

impl Reader for CodexReader {
    fn push(&mut self, line: &str) -> Read {
        let mut read = read_line(line);
        let mut news = Vec::new();
        for entry in &read.entries {
            match entry {
                Entry::ToolCall { id, name, input, .. } if name == "write_stdin" => {
                    let session = input.get("session_id").map(|session| match session {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    });
                    if let Some(session) = session {
                        self.stdin.insert(id.clone(), session);
                    }
                }
                Entry::ToolResult { tool_use_id, text, .. } => {
                    if let Some(session) = running_session(text) {
                        news.push(Task {
                            id: session,
                            call: Some(tool_use_id.clone()),
                            kind: Some(TaskKind::Background),
                            status: TaskStatus::Running,
                            side: None,
                            summary: None,
                        });
                    } else if let (Some(code), Some(session)) = (exit_code(text), self.stdin.remove(tool_use_id)) {
                        news.push(Task {
                            id: session,
                            call: None,
                            kind: Some(TaskKind::Background),
                            status: if code == 0 { TaskStatus::Completed } else { TaskStatus::Failed },
                            side: None,
                            summary: Some(format!("Exited with code {code}")),
                        });
                    }
                }
                _ => {}
            }
        }
        read.entries.extend(news.into_iter().map(Entry::Task));
        read
    }
}

/// The session a unified-exec command still runs in, from its output:
/// `Process running with session ID 3`.
fn running_session(output: &str) -> Option<String> {
    const SAID: &str = "Process running with session ID ";
    let after = &output[output.find(SAID)? + SAID.len()..];
    let id: String = after.chars().take_while(char::is_ascii_alphanumeric).collect();
    (!id.is_empty()).then_some(id)
}

/// How a background command ended, from the output that saw it end:
/// `Process exited with code 1`.
fn exit_code(output: &str) -> Option<i32> {
    const SAID: &str = "Process exited with code ";
    let after = &output[output.find(SAID)? + SAID.len()..];
    after.split_whitespace().next()?.parse().ok()
}

/// The subagent a `spawn_agent` call started, from its output's `agent_id`.
fn spawned(call: &str, output: &str) -> Option<Task> {
    let output: Value = serde_json::from_str(output).ok()?;
    let agent = output.get("agent_id")?.as_str().filter(|agent| crate::is_safe_id(agent))?;
    Some(Task {
        id: agent.to_owned(),
        call: Some(call.to_owned()),
        kind: Some(TaskKind::Subagent),
        status: TaskStatus::Running,
        side: Some(agent.to_owned()),
        summary: None,
    })
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
            read.entries.push(Entry::ToolCall { id, name, input, kind: None });
        }
        ("response_item", "function_call_output" | "custom_tool_call_output") => {
            let id = field("call_id");
            let text = match payload.get("output") {
                Some(Value::String(text)) => text.clone(),
                other => content_text(other),
            };
            read.marks.push(Mark::ToolFinished { tool_use_id: id.clone() });
            let task = spawned(&id, &text);
            read.entries.push(Entry::ToolResult { tool_use_id: id, text, is_error: false });
            read.entries.extend(task.map(Entry::Task));
        }
        ("event_msg", "item_completed") if payload.pointer("/item/type").and_then(Value::as_str) == Some("SubAgentActivity") => {
            let item = &payload["item"];
            let status = item.get("kind").and_then(Value::as_str).map_or(TaskStatus::Running, |kind| match kind {
                "interrupted" => TaskStatus::Stopped,
                other => TaskStatus::parse(other),
            });
            match item.get("agent_thread_id").and_then(Value::as_str).filter(|agent| crate::is_safe_id(agent)) {
                Some(agent) if status != TaskStatus::Running => read.entries.push(Entry::Task(Task {
                    id: agent.to_owned(),
                    call: None,
                    kind: Some(TaskKind::Subagent),
                    status,
                    side: Some(agent.to_owned()),
                    summary: None,
                })),
                _ => read.entries.push(Entry::Other { kind: "event_msg:item_completed".into() }),
            }
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
    fn model_apis(&self) -> &'static [crate::ModelApi] {
        &crate::CODEX_APIS
    }
    fn model_hosts(&self) -> &'static [&'static str] {
        &["api.openai.com", "auth.openai.com", "chatgpt.com"]
    }
    fn homes(&self, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        vec![crate::home_from(home, env, "CODEX_HOME", ".codex")]
    }

    fn sign_in(&self, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        vec![crate::home_from(home, env, "CODEX_HOME", ".codex").join("auth.json")]
    }

    fn read_only_home(&self, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        vec![crate::home_from(home, env, "CODEX_HOME", ".codex").join("config.toml")]
    }

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
        Box::new(CodexReader::default())
    }

    fn session_ids(&self) -> SessionIds {
        SessionIds::Discovered
    }

    fn transcript(&self, cwd: &Path, id: &str) -> Option<PathBuf> {
        transcript_in(&home_in(cwd)?, id)
    }

    fn side_transcripts(&self, transcript: &Path) -> Vec<(String, PathBuf)> {
        subagent_rollouts(transcript)
    }

    fn discover(&self, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
        discover_in(&home_in(cwd)?, cwd, since_ms, known)
    }

    fn on_disk(&self, _dirs: &[PathBuf]) -> Vec<OnDisk> {
        crate::agent_homes("CODEX_HOME", ".codex").iter().flat_map(|home| on_disk_in(home)).collect()
    }

    fn restore(&self, root: &Path, cwd: &Path, session: &CheckedSession) -> Result<PathBuf, AgentError> {
        let home = home_in(cwd).ok_or(AgentError::NoHome)?;
        restore_into(&home, root, cwd, session)
    }

    fn translate(&self, entries: &[Entry], cwd: &Path, id: &str) -> Option<Vec<String>> {
        Some(rollout_lines(&translated_text(entries), cwd, id))
    }

    fn bypass_flag(&self, args: &[String]) -> Option<String> {
        bypass_flag(args)
    }

    fn sandbox_support(&self) -> crate::SandboxSupport {
        crate::SandboxSupport::Sandbox
    }

    fn launch(&self, plan: &LaunchPlan) -> Result<Launch, AgentError> {
        Ok(launch_as(self.command(), plan, &["approval_policy=\"on-request\""]))
    }
}

/// The first of `args` that stops Codex, or a fork of it, asking.
pub(crate) fn bypass_flag(args: &[String]) -> Option<String> {
    crate::flag_in(
        args,
        &["--dangerously-bypass-approvals-and-sandbox", "--yolo", "--full-auto"],
        &[
            ("--sandbox", &["danger-full-access"]),
            ("-s", &["danger-full-access"]),
            ("--ask-for-approval", &["never"]),
            ("-a", &["never"]),
        ],
    )
}

/// The run of `command`, a Codex or a fork of it, with `asking`, the
/// settings that make it ask before it acts, when its workspace says so.
pub(crate) fn launch_as(command: &str, plan: &LaunchPlan, asking: &[&str]) -> Launch {
    let mut launch = Launch::new(command);
    // `codex resume <id> [prompt]`, or `codex [prompt]`: last, after the flags.
    let mut trailing = Vec::new();
    match &plan.start {
        Start::Resume { id, prompt } => {
            launch.arg("resume");
            trailing.push(id.clone());
            trailing.extend(prompt.clone());
        }
        Start::Primed { prompt: text, .. } => trailing.push(text.clone()),
        Start::New { .. } => {}
    }
    let toml = |value: &Value| value.to_string();
    launch.arg("-c").arg(format!("mcp_servers.silt.command={}", toml(&json!(plan.mcp.command.to_string_lossy()))));
    launch.arg("-c").arg(format!("mcp_servers.silt.args={}", toml(&json!(plan.mcp.args))));
    if let Some(instructions) = &plan.instructions {
        launch.arg("-c").arg(format!("developer_instructions={}", toml(&json!(instructions))));
    }
    if plan.locks.sandbox {
        launch.arg("-c").arg("sandbox_mode=\"workspace-write\"");
    }
    if plan.locks.no_bypass {
        for setting in asking {
            launch.arg("-c").arg(*setting);
        }
    }
    for arg in &plan.args {
        launch.arg(arg);
    }
    for arg in trailing {
        launch.arg(arg);
    }
    launch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restore_replaces_codex_s_own_rollout_of_the_id_or_dates_a_new_one() {
        let session =
            crate::RecordedSession { id: "id".into(), main: vec!["{}".into()], ..Default::default() }.checked().expect("safe");
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
            vec![Entry::ToolCall { id: "call_1".into(), name: "shell".into(), input: json!({"command": ["ls"]}), kind: None }]
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

    #[test]
    fn rollouts_on_disk_are_listed_once_per_session() {
        let home = tempfile::tempdir().expect("temp dir");
        let dir = home.path().join("sessions/2026/01/02");
        fs::create_dir_all(&dir).expect("mkdir");
        let id = "0199a213-81c0-7800-8aa1-bbab2a035a53";
        let meta = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"timestamp\":\"2026-01-02T03:04:05.000Z\",\"cwd\":\"/w/repo\"}}}}\n"
        );
        fs::write(dir.join(format!("rollout-2026-01-02T03-04-05-{id}.jsonl")), &meta).expect("writes");
        fs::write(dir.join(format!("rollout-2026-01-02T03-04-05-{id}_other.jsonl")), &meta).expect("writes");
        let found = on_disk_in(home.path());
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].id, id);
        assert_eq!(found[0].cwd.as_deref(), Some(Path::new("/w/repo")));
        assert!(found[0].started_ms.is_some());
    }

    #[test]
    fn a_subagent_s_rollout_is_its_parent_s_side_transcript_not_a_session() {
        let home = tempfile::tempdir().expect("temp dir");
        let (parent, child, other) = (
            "0199a213-81c0-7800-8aa1-bbab2a035a53",
            "0199a213-81c0-7800-8aa1-bbab2a035a54",
            "0199a213-81c0-7800-8aa1-bbab2a035a55",
        );
        let write = |day: &str, id: &str, meta: Value| {
            let dir = home.path().join("sessions").join(day);
            fs::create_dir_all(&dir).expect("mkdir");
            let path = dir.join(format!("rollout-2026-01-02T03-04-05-{id}.jsonl"));
            fs::write(&path, format!("{}\n", json!({"type": "session_meta", "payload": meta}))).expect("writes");
            path
        };
        let main = write("2026/01/02", parent, json!({"id": parent, "timestamp": "2026-01-02T03:04:05.000Z", "cwd": "/w/repo"}));
        let spawned = write(
            "2026/01/03",
            child,
            json!({"id": child, "timestamp": "2026-01-03T00:00:01.000Z", "cwd": "/w/repo",
                "source": {"subagent": {"thread_spawn": {"parent_thread_id": parent, "depth": 1}}}}),
        );
        write("2026/01/02", other, json!({"id": other, "cwd": "/w/repo", "parent_thread_id": "someone-else"}));
        let earlier =
            home.path().join("sessions/2026/01/02/rollout-2026-01-02T01-00-00-0199a213-81c0-7800-8aa1-bbab2a035a56.jsonl");
        let claims = json!({"type": "session_meta", "payload": {"id": "x", "parent_thread_id": parent}});
        fs::write(&earlier, format!("{claims}\n")).expect("writes");
        assert_eq!(subagent_rollouts(&main), vec![(child.to_owned(), spawned)]);
        assert_eq!(on_disk_in(home.path()).iter().map(|found| found.id.as_str()).collect::<Vec<_>>(), vec![parent]);
    }

    #[test]
    fn a_spawned_subagent_and_a_background_command_are_named_until_they_end() {
        let mut reader = CodexReader::default();
        let mut tasks = |line: Value| -> Vec<Task> {
            let read = reader.push(&line.to_string());
            read.entries.into_iter().filter_map(|entry| if let Entry::Task(task) = entry { Some(task) } else { None }).collect()
        };
        let item = |payload: Value| json!({"timestamp": "t", "type": "response_item", "payload": payload});
        let spawned = tasks(item(
            json!({"type": "function_call_output", "call_id": "c1", "output": "{\"agent_id\":\"kid-1\",\"nickname\":\"Ada\"}"}),
        ));
        assert_eq!(
            spawned,
            vec![Task {
                id: "kid-1".into(),
                call: Some("c1".into()),
                kind: Some(TaskKind::Subagent),
                status: TaskStatus::Running,
                side: Some("kid-1".into()),
                summary: None
            }]
        );
        let started = tasks(item(
            json!({"type": "function_call_output", "call_id": "c2", "output": "Wall time: 1s\nProcess running with session ID 7\nOutput:\nlistening"}),
        ));
        assert_eq!(
            (started[0].id.as_str(), started[0].call.as_deref(), started[0].status),
            ("7", Some("c2"), TaskStatus::Running)
        );
        tasks(item(
            json!({"type": "function_call", "name": "write_stdin", "arguments": "{\"session_id\":7,\"chars\":\"\"}", "call_id": "c3"}),
        ));
        let ended = tasks(item(
            json!({"type": "function_call_output", "call_id": "c3", "output": "Process exited with code 2\nOutput:\nboom"}),
        ));
        assert_eq!((ended[0].id.as_str(), ended[0].status), ("7", TaskStatus::Failed));
        let finished = tasks(json!({"timestamp": "t", "type": "event_msg", "payload": {"type": "item_completed",
            "item": {"type": "SubAgentActivity", "kind": "completed", "agent_thread_id": "kid-1"}}}));
        assert_eq!(finished[0].status, TaskStatus::Completed);
    }
}
