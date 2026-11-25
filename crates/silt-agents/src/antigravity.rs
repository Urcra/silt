//! Antigravity CLI (`agy`), which took Gemini CLI's place for most people.
//! Each conversation logs its steps as JSON lines, one per finished step, in
//! `~/.gemini/antigravity-cli/brain/<id>/.system_generated/logs/transcript_full.jsonl`.
//! agy resumes only from its own store, a SQLite database of protobuf steps
//! per conversation, which Silt can't write; it reads agy's index of
//! conversations for where each ran (docs/research/agents.md, ADR 0125).

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::conventions::NativeConventions;
use crate::{
    Agent, AgentError, CheckedSession, Entry, Found, Launch, LaunchPlan, Mark, OnDisk, Read, Reader, SessionIds, Start, Task,
    TaskKind, TaskStatus,
};

pub const AGENT: &str = "antigravity-cli";

/// The directory agy keeps its files in, `~/.gemini`, which it shares with
/// Gemini CLI.
fn gemini_dir() -> Option<PathBuf> {
    crate::home().map(|home| home.join(".gemini"))
}

/// Where the CLI keeps its conversations, inside `.gemini`.
pub fn app_dir(gemini: &Path) -> PathBuf {
    gemini.join("antigravity-cli")
}

fn transcript_in(app: &Path, id: &str) -> PathBuf {
    app.join("brain").join(id).join(".system_generated/logs/transcript_full.jsonl")
}

/// The conversation a transcript belongs to: `brain/<id>/.system_generated/logs/…`.
fn transcript_id(transcript: &Path) -> Option<String> {
    let id = transcript.ancestors().nth(3)?.file_name()?.to_string_lossy().into_owned();
    crate::is_safe_id(&id).then_some(id)
}

/// A conversation as agy's index lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub id: String,
    /// The folder it runs in: its first workspace.
    pub cwd: Option<PathBuf>,
    /// The conversation that started it, for a subagent's.
    pub parent: Option<String>,
}

/// Every conversation in the index `conversation_summaries.db` of `app`.
/// agy writes a conversation's row as it starts, so it is read live, and
/// left alone when agy keeps it busy.
pub fn summaries(app: &Path) -> Vec<Summary> {
    let read = || -> rusqlite::Result<Vec<Summary>> {
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = rusqlite::Connection::open_with_flags(app.join("conversation_summaries.db"), flags)?;
        connection.busy_timeout(std::time::Duration::from_millis(500))?;
        let mut statement =
            connection.prepare("SELECT conversation_id, workspace_uris, parent_conversation_id FROM conversation_summaries")?;
        let rows = statement.query_map([], |row| {
            let (id, uris, parent): (String, String, String) = (row.get(0)?, row.get(1)?, row.get(2)?);
            Ok(Summary { id, cwd: first_workspace(&uris), parent: (!parent.is_empty()).then_some(parent) })
        })?;
        Ok(rows.flatten().filter(|summary| crate::is_safe_id(&summary.id)).collect())
    };
    read().unwrap_or_default()
}

/// The first of a JSON list of `file://` URIs, as a path.
fn first_workspace(uris: &str) -> Option<PathBuf> {
    let uris: Vec<String> = serde_json::from_str(uris).ok()?;
    file_path(uris.first()?)
}

/// The path a `file://` URI names. Windows' come as `file:///C:/…`.
pub fn file_path(uri: &str) -> Option<PathBuf> {
    let path = percent_decode(uri.strip_prefix("file://")?)?;
    let windows = path.len() > 2 && path.as_bytes()[0] == b'/' && path.as_bytes()[2] == b':';
    Some(PathBuf::from(if windows { path[1..].replace('/', "\\") } else { path }))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// When a transcript's first step was made.
fn started_ms(transcript: &Path) -> Option<i64> {
    let first: Value = serde_json::from_str(&crate::first_line(transcript)?).ok()?;
    crate::time::parse_timestamp(first.get("created_at")?.as_str()?)
}

/// Every conversation in `app` started by a person rather than an agent,
/// with the folder it ran in and when it started.
pub fn on_disk_in(app: &Path) -> Vec<OnDisk> {
    let mut found: Vec<OnDisk> = summaries(app)
        .into_iter()
        .filter(|summary| summary.parent.is_none())
        .filter_map(|summary| {
            let transcript = transcript_in(app, &summary.id);
            transcript.is_file().then(|| OnDisk {
                started_ms: started_ms(&transcript),
                id: summary.id,
                transcript,
                cwd: summary.cwd,
            })
        })
        .collect();
    found.sort_by(|a, b| a.started_ms.cmp(&b.started_ms).then_with(|| a.id.cmp(&b.id)));
    found
}

/// The newest conversation of `cwd` written since `since_ms` in `app`
/// whose id isn't in `known`, leaving out subagents'. Started with a prompt
/// (`-i`) in a folder it doesn't trust yet, agy lists a conversation with
/// no workspace at all, so such a one is taken when none names `cwd`.
pub fn discover_in(app: &Path, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
    let fresh: Vec<(bool, i64, PathBuf, String)> = summaries(app)
        .into_iter()
        .filter(|summary| summary.parent.is_none() && !known.contains(&summary.id))
        .filter(|summary| summary.cwd.as_deref().is_none_or(|ran| ran == cwd))
        .filter_map(|summary| {
            let transcript = transcript_in(app, &summary.id);
            let modified = crate::modified_ms(&transcript)?;
            Some((summary.cwd.is_some(), modified, transcript, summary.id))
        })
        .filter(|(_, modified, _, _)| *modified >= since_ms)
        .collect();
    fresh
        .into_iter()
        .max_by_key(|(placed, modified, _, _)| (*placed, *modified))
        .map(|(_, _, transcript, id)| Found { id, transcript })
}

/// The transcripts of the subagents the conversation in `transcript` started.
pub fn subagents_in(app: &Path, transcript: &Path) -> Vec<(String, PathBuf)> {
    let Some(id) = transcript_id(transcript) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = summaries(app)
        .into_iter()
        .filter(|summary| summary.parent.as_deref() == Some(&*id))
        .map(|summary| (transcript_in(app, &summary.id), summary.id))
        .filter(|(transcript, _)| transcript.is_file())
        .map(|(transcript, id)| (id, transcript))
        .collect();
    found.sort();
    found
}

/// What a conversation's steps point to by path, under its
/// `.system_generated` directory `dir`: a tool's output too long for its
/// step, `steps/<n>/output.txt`, and a background task's log,
/// `tasks/<task>.log`.
pub fn outputs_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for (kind, inner) in [("steps", Some("output.txt")), ("tasks", None)] {
        let Ok(entries) = fs::read_dir(dir.join(kind)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let (path, name) = match inner {
                Some(file) => (entry.path().join(file), format!("{kind}/{name}/{file}")),
                None if name.ends_with(".log") => (entry.path(), format!("{kind}/{name}")),
                None => continue,
            };
            if path.is_file() && crate::is_side_file_name(&name) {
                found.push((name, path));
            }
        }
    }
    found.sort();
    found
}

/// Whether agy can resume conversation `id` from `app`: only one it keeps
/// itself, as Silt can't write its store.
pub fn resumable_in(app: &Path, id: &str) -> bool {
    crate::is_safe_id(id) && app.join("conversations").join(format!("{id}.db")).is_file()
}

/// A tool call waiting for its result, which comes as the next step.
struct Pending {
    id: String,
    name: String,
    /// Whom a `send_message` call writes to.
    recipient: Option<String>,
}

/// Reads a transcript, pairing each tool call with the step that answers
/// it: agy writes a reply's calls in one step, then one step per call, in
/// order, and gives the calls no ids of their own.
#[derive(Default)]
pub struct AntigravityReader {
    pending: VecDeque<Pending>,
    /// Subagents by conversation id, with the call that started each, or
    /// the `send_message` that woke it last.
    subagents: HashMap<String, String>,
    /// Subagents that went idle, which a message wakes.
    idle: HashSet<String>,
    /// Commands left running in the background, by task id: the call that
    /// started each and its command line.
    commands: HashMap<String, (String, String)>,
    /// What set the agent going again after its turn ended, as a notice said.
    cause: Option<String>,
    /// Whether the last turn ended, so a step the agent takes after is a
    /// turn it opened itself.
    stopped: bool,
}

impl Reader for AntigravityReader {
    fn push(&mut self, line: &str) -> Read {
        let mut read = Read::default();
        let Ok(step) = serde_json::from_str::<Value>(line) else {
            return Read::unparseable();
        };
        let text = |key: &str| step.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let index = step.get("step_index").and_then(Value::as_u64).unwrap_or_default();
        match text("type").as_str() {
            "USER_INPUT" => {
                self.pending.clear();
                self.stopped = false;
                read.marks.push(Mark::PromptSubmitted);
                read.entries.push(Entry::User { text: user_request(&text("content")) });
            }
            "PLANNER_RESPONSE" => {
                if self.stopped {
                    self.stopped = false;
                    read.marks.push(Mark::PromptSubmitted);
                    let text = match self.cause.take() {
                        Some(cause) => format!("{cause}, and the agent went on by itself."),
                        None => crate::acp::WOKE.to_owned(),
                    };
                    read.entries.push(Entry::Woke { text });
                }
                let thinking = text("thinking");
                if !thinking.is_empty() {
                    read.entries.push(Entry::Thinking { text: thinking });
                }
                let content = text("content");
                if !content.is_empty() {
                    read.entries.push(Entry::Assistant { text: content });
                }
                let tokens = |key: &str| step.get(key).and_then(Value::as_u64);
                if let (Some(input), Some(output)) = (tokens("input_tokens"), tokens("output_tokens")) {
                    read.entries.push(Entry::Usage { message: format!("step-{index}"), input, output });
                }
                let calls = step.get("tool_calls").and_then(Value::as_array).cloned().unwrap_or_default();
                for (number, call) in calls.iter().enumerate() {
                    let id = format!("step-{index}-{number}");
                    let name = call.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
                    read.marks.push(Mark::ToolStarted { tool_use_id: id.clone(), tool: name.clone() });
                    read.entries.push(Entry::ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        input: call.get("args").cloned().unwrap_or(Value::Null),
                        kind: None,
                    });
                    let recipient = call.pointer("/args/Recipient").and_then(Value::as_str).map(str::to_owned);
                    self.pending.push_back(Pending { id, name, recipient });
                }
                // A reply that calls no tool ends the turn.
                if calls.is_empty() {
                    self.stopped = true;
                    read.marks.push(Mark::Stopped);
                }
            }
            "GENERIC" => match self.pending.pop_front() {
                Some(call) => {
                    let status = text("status");
                    let done = status == "DONE";
                    // A command that outlasts its wait goes on as a task, and
                    // its call answers at once.
                    let ok = done || status == "RUNNING";
                    let said = if ok || text("error").is_empty() { text("content") } else { text("error") };
                    let said = without_stamps(&said);
                    let subagent = (call.name == "invoke_subagent" && done).then(|| started_subagent(&said)).flatten();
                    let command = (status == "RUNNING").then(|| backgrounded(&said)).flatten();
                    let cancelled = (call.name == "manage_task" && done).then(|| cancelled(&said)).flatten();
                    read.marks.push(Mark::ToolFinished { tool_use_id: call.id.clone() });
                    read.entries.push(Entry::ToolResult { tool_use_id: call.id.clone(), text: said, is_error: !ok });
                    if let Some(subagent) = subagent {
                        self.subagents.insert(subagent.clone(), call.id.clone());
                        read.entries.push(Entry::Task(task(&subagent, &call.id, TaskStatus::Running, None)));
                    }
                    if call.name == "send_message"
                        && done
                        && let Some(subagent) = call.recipient.filter(|recipient| self.idle.remove(recipient))
                        && let Some(started) = self.subagents.get_mut(&subagent)
                    {
                        started.clone_from(&call.id);
                        read.entries.push(Entry::Task(task(&subagent, &call.id, TaskStatus::Running, None)));
                    }
                    if let Some((id, line)) = command {
                        read.entries.push(Entry::Task(background(&id, &call.id, TaskStatus::Running, None)));
                        self.commands.insert(id, (call.id.clone(), line));
                    }
                    if let Some(id) = cancelled
                        && let Some((started, _)) = self.commands.remove(&id)
                    {
                        read.entries.push(Entry::Task(background(&id, &started, TaskStatus::Stopped, None)));
                    }
                }
                None => read.entries.push(Entry::Other { kind: "step".into() }),
            },
            "SYSTEM_MESSAGE" => {
                let content = text("content");
                let idle =
                    self.subagents.iter().find(|(subagent, _)| content.contains(&format!("Subagent {subagent} has gone idle")));
                if let Some((subagent, call)) = idle {
                    self.idle.insert(subagent.clone());
                    let said = content
                        .split_once("most recent response: ")
                        .map(|(_, said)| said.split("</SYSTEM_MESSAGE>").next().unwrap_or(said).trim().to_owned());
                    read.entries.push(Entry::Task(task(subagent, call, TaskStatus::Completed, said)));
                    self.cause = Some("A subagent finished".into());
                } else if let Some((id, result)) = finished(&content)
                    && let Some((call, line)) = self.commands.remove(&id)
                {
                    let status = if result.contains("exited with code 0.") { TaskStatus::Completed } else { TaskStatus::Failed };
                    read.entries.push(Entry::Task(background(&id, &call, status, Some(result))));
                    self.cause = Some(format!("`{line}` finished in the background"));
                } else {
                    read.entries.push(Entry::Other { kind: "system".into() });
                }
            }
            other => read.entries.push(Entry::Other { kind: other.to_ascii_lowercase() }),
        }
        read
    }
}

/// What the person typed: agy wraps a prompt in `<USER_REQUEST>` and adds
/// the time and settings changes after it.
fn user_request(content: &str) -> String {
    let inner = content
        .split_once("<USER_REQUEST>")
        .and_then(|(_, rest)| rest.split_once("</USER_REQUEST>"))
        .map_or(content, |(request, _)| request);
    inner.trim_matches('\n').to_owned()
}

/// A step's text without the `Created At:` and `Completed At:` lines agy
/// starts each with.
fn without_stamps(text: &str) -> String {
    let mut lines = text.lines().peekable();
    while lines.peek().is_some_and(|line| line.starts_with("Created At: ") || line.starts_with("Completed At: ")) {
        lines.next();
    }
    lines.collect::<Vec<_>>().join("\n")
}

/// The conversation `invoke_subagent` started, from its answer's JSON.
fn started_subagent(said: &str) -> Option<String> {
    let start = said.find('{')?;
    let mut stream = serde_json::Deserializer::from_str(&said[start..]).into_iter::<Value>();
    let id = stream.next()?.ok()?.get("conversationId")?.as_str()?.to_owned();
    crate::is_safe_id(&id).then_some(id)
}

/// The task a command went on as, and its command line, from its call's
/// answer: `Tool is running as a background task with task id: <id>` and
/// `Task Description: <command>`.
fn backgrounded(said: &str) -> Option<(String, String)> {
    let id = said.split_once("background task with task id: ")?.1.lines().next()?.trim().to_owned();
    let line =
        said.split_once("Task Description: ").and_then(|(_, rest)| rest.lines().next()).unwrap_or_default().trim().to_owned();
    (!id.is_empty()).then_some((id, line))
}

/// The task `manage_task` stopped: `Task "<id>" cancelled.`
fn cancelled(said: &str) -> Option<String> {
    let (id, rest) = said.trim().strip_prefix("Task \"")?.split_once('"')?;
    (rest.trim() == "cancelled.").then(|| id.to_owned())
}

/// The task a notice says ended, and what it ended with:
/// `Task id "<id>" finished with result:\n<result>\n\nLog: …`.
fn finished(notice: &str) -> Option<(String, String)> {
    let (id, rest) = notice.split_once("Task id \"")?.1.split_once('"')?;
    let result = rest.trim_start().strip_prefix("finished with result:")?;
    let result = result.split("\nLog: ").next().unwrap_or(result).split("</SYSTEM_MESSAGE>").next().unwrap_or(result);
    Some((id.to_owned(), result.trim().to_owned()))
}

fn background(id: &str, call: &str, status: TaskStatus, summary: Option<String>) -> Task {
    Task { id: id.to_owned(), call: Some(call.to_owned()), kind: Some(TaskKind::Background), status, side: None, summary }
}

fn task(subagent: &str, call: &str, status: TaskStatus, summary: Option<String>) -> Task {
    Task {
        id: subagent.to_owned(),
        call: Some(call.to_owned()),
        kind: Some(TaskKind::Subagent),
        status,
        side: Some(subagent.to_owned()),
        summary,
    }
}

/// agy takes no per-run settings, but reads its files from `--gemini_dir`,
/// so it runs with a `.gemini` of its own that links everything to the real
/// one except the MCP servers, which add Silt's, the global rules, which add
/// the session's instructions, and the global skills, which add the given
/// ones. Its conversations still land in the real one.
fn overlay(plan: &LaunchPlan) -> Result<PathBuf, AgentError> {
    let real = gemini_dir().ok_or(AgentError::NoHome)?;
    fs::create_dir_all(app_dir(&real))?;
    fs::create_dir_all(real.join("config"))?;
    let overlay = plan.scratch.join("antigravity-gemini");
    if overlay.exists() {
        fs::remove_dir_all(&overlay)?;
    }
    fs::create_dir_all(overlay.join("config"))?;
    link_all_but(&real, &overlay, &["config", "GEMINI.md", "skills"])?;
    link_all_but(&real.join("config"), &overlay.join("config"), &["mcp_config.json"])?;
    let mut servers: Value = fs::read_to_string(real.join("config/mcp_config.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let listed = servers.as_object_mut().expect("checked above").entry("mcpServers").or_insert_with(|| json!({}));
    if let Some(listed) = listed.as_object_mut() {
        listed.insert("silt".into(), json!({"command": plan.mcp.command.to_string_lossy(), "args": plan.mcp.args}));
    }
    fs::write(overlay.join("config/mcp_config.json"), servers.to_string())?;
    let mut rules = fs::read_to_string(real.join("GEMINI.md")).unwrap_or_default();
    if let Some(instructions) = &plan.instructions {
        if !rules.is_empty() {
            rules.push_str("\n\n");
        }
        rules.push_str(instructions);
    }
    fs::write(overlay.join("GEMINI.md"), rules)?;
    let skills = overlay.join("skills");
    crate::link_skills(&skills, &plan.skills)?;
    if let Ok(entries) = fs::read_dir(real.join("skills")) {
        crate::link_skills(&skills, &entries.flatten().map(|entry| entry.path()).collect::<Vec<_>>())?;
    }
    Ok(overlay)
}

/// Links each entry of `from` into `into`, but those named in `except`.
fn link_all_but(from: &Path, into: &Path, except: &[&str]) -> io::Result<()> {
    for entry in fs::read_dir(from)?.flatten() {
        let name = entry.file_name();
        if !except.iter().any(|except| name == *except) {
            crate::symlink(&entry.path(), &into.join(&name))?;
        }
    }
    Ok(())
}

/// The Antigravity CLI adapter.
pub struct Antigravity;

impl Agent for Antigravity {
    fn model_apis(&self) -> &'static [crate::ModelApi] {
        &crate::GEMINI_APIS
    }
    fn model_hosts(&self) -> &'static [&'static str] {
        &["generativelanguage.googleapis.com", "cloudcode-pa.googleapis.com", "oauth2.googleapis.com", "accounts.google.com"]
    }
    fn homes(&self, home: &Path, _env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        vec![home.join(".gemini"), home.join(".antigravity")]
    }

    fn id(&self) -> &'static str {
        AGENT
    }

    fn command(&self) -> &'static str {
        "agy"
    }

    fn display_name(&self) -> &'static str {
        "Antigravity CLI"
    }

    fn native(&self) -> NativeConventions {
        NativeConventions {
            rules: &["GEMINI.md", "AGENTS.md"],
            skill_dirs: &[".agents/skills"],
            personal_skills: false,
            given_skills: true,
        }
    }

    fn reader(&self) -> Box<dyn Reader> {
        Box::<AntigravityReader>::default()
    }

    fn session_ids(&self) -> SessionIds {
        SessionIds::Discovered
    }

    fn transcript(&self, _cwd: &Path, id: &str) -> Option<PathBuf> {
        let app = app_dir(&gemini_dir()?);
        crate::is_safe_id(id).then(|| transcript_in(&app, id))
    }

    /// `brain/<id>/.system_generated`, which the steps name files under.
    fn session_dir(&self, transcript: &Path) -> Option<PathBuf> {
        Some(transcript.parent()?.parent()?.to_owned())
    }

    fn attachments(&self, transcript: &Path) -> Vec<(String, PathBuf)> {
        self.session_dir(transcript).map(|dir| outputs_in(&dir)).unwrap_or_default()
    }

    fn side_transcripts(&self, transcript: &Path) -> Vec<(String, PathBuf)> {
        gemini_dir().map(|gemini| subagents_in(&app_dir(&gemini), transcript)).unwrap_or_default()
    }

    fn discover(&self, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
        discover_in(&app_dir(&gemini_dir()?), cwd, since_ms, known)
    }

    fn on_disk(&self, _dirs: &[PathBuf]) -> Vec<OnDisk> {
        gemini_dir().map(|gemini| on_disk_in(&app_dir(&gemini))).unwrap_or_default()
    }

    fn can_restore(&self, session: &CheckedSession) -> bool {
        gemini_dir().is_some_and(|gemini| resumable_in(&app_dir(&gemini), &session.id))
    }

    /// Writes nothing: agy resumes a conversation it still keeps, and any
    /// other starts primed.
    fn restore(&self, _root: &Path, _cwd: &Path, session: &CheckedSession) -> Result<PathBuf, AgentError> {
        let app = app_dir(&gemini_dir().ok_or(AgentError::NoHome)?);
        if !resumable_in(&app, &session.id) {
            return Err(AgentError::NotKept(session.id.clone()));
        }
        Ok(transcript_in(&app, &session.id))
    }

    fn bypass_flag(&self, args: &[String]) -> Option<String> {
        crate::flag_in(args, &["--yolo", "-y", "--dangerously-skip-permissions"], &[("--approval-mode", &["yolo"])])
    }

    fn launch(&self, plan: &LaunchPlan) -> Result<Launch, AgentError> {
        let mut launch = Launch::new(self.command());
        launch.arg("--gemini_dir").arg(overlay(plan)?);
        let prompt = match &plan.start {
            Start::New { .. } => None,
            Start::Resume { id, .. } => {
                launch.arg("--conversation").arg(id);
                None
            }
            Start::Primed { prompt, .. } => Some(prompt.clone()),
        };
        for arg in &plan.args {
            launch.arg(arg);
        }
        if let Some(prompt) = prompt {
            launch.arg("--prompt-interactive").arg(prompt);
        }
        Ok(launch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(lines: &[Value]) -> (Vec<Entry>, Vec<Mark>) {
        let mut reader = AntigravityReader::default();
        let reads: Vec<Read> = lines.iter().map(|line| reader.push(&line.to_string())).collect();
        (reads.iter().flat_map(|read| read.entries.clone()).collect(), reads.into_iter().flat_map(|read| read.marks).collect())
    }

    fn generic(index: u64, status: &str, content: &str) -> Value {
        json!({"step_index": index, "source": "MODEL", "type": "GENERIC", "status": status,
            "content": format!("Created At: 2026-11-13T07:51:40Z\nCompleted At: 2026-11-13T07:51:40Z\n{content}")})
    }

    /// Steps as agy 1.5.8 wrote them for a prompt that edited a file, ran a
    /// command and answered.
    #[test]
    fn steps_read_as_a_turn_with_its_tools() {
        let (entries, marks) = read_all(&[
            json!({"step_index": 0, "source": "USER_EXPLICIT", "type": "USER_INPUT", "status": "DONE",
                "content": "<USER_REQUEST>\nmake some changes\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nThe current local time is: 2026-11-13T07:51:40Z.\n</ADDITIONAL_METADATA>"}),
            json!({"step_index": 1, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "input_tokens": 10,
                "output_tokens": 5, "content": "Two at once.", "thinking": "pondering",
                "tool_calls": [{"name": "edit_file", "args": {"Path": "/w/a.txt", "Target": "hi", "Replacement": "hello"}},
                    {"name": "run_command", "args": {"CommandLine": "ls"}}]}),
            generic(2, "DONE", "Made 1 replacement in /w/a.txt"),
            json!({"step_index": 3, "source": "MODEL", "type": "GENERIC", "status": "ERROR", "error": "denied",
                "content": "Encountered error in step execution: denied"}),
            json!({"step_index": 4, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "content": "All done."}),
        ]);
        assert_eq!(
            entries,
            [
                Entry::User { text: "make some changes".into() },
                Entry::Thinking { text: "pondering".into() },
                Entry::Assistant { text: "Two at once.".into() },
                Entry::Usage { message: "step-1".into(), input: 10, output: 5 },
                Entry::ToolCall {
                    id: "step-1-0".into(),
                    name: "edit_file".into(),
                    input: json!({"Path": "/w/a.txt", "Target": "hi", "Replacement": "hello"}),
                    kind: None
                },
                Entry::ToolCall {
                    id: "step-1-1".into(),
                    name: "run_command".into(),
                    input: json!({"CommandLine": "ls"}),
                    kind: None
                },
                Entry::ToolResult {
                    tool_use_id: "step-1-0".into(),
                    text: "Made 1 replacement in /w/a.txt".into(),
                    is_error: false
                },
                Entry::ToolResult { tool_use_id: "step-1-1".into(), text: "denied".into(), is_error: true },
                Entry::Assistant { text: "All done.".into() },
            ]
        );
        assert_eq!(
            marks,
            [
                Mark::PromptSubmitted,
                Mark::ToolStarted { tool_use_id: "step-1-0".into(), tool: "edit_file".into() },
                Mark::ToolStarted { tool_use_id: "step-1-1".into(), tool: "run_command".into() },
                Mark::ToolFinished { tool_use_id: "step-1-0".into() },
                Mark::ToolFinished { tool_use_id: "step-1-1".into() },
                Mark::Stopped,
            ]
        );
    }

    #[test]
    fn a_message_to_an_idle_subagent_runs_it_again_under_the_message() {
        let sub = "39497b42-d96f-4954-855d-69dd29a7d6f3";
        let idle = |index: u64, said: &str| {
            json!({"step_index": index, "source": "SYSTEM", "type": "SYSTEM_MESSAGE", "status": "DONE",
                "content": format!("<SYSTEM_MESSAGE>\n[Message] content=Subagent {sub} has gone idle. This was the subagent's most recent response: {said}\n</SYSTEM_MESSAGE>")})
        };
        let message = |index: u64| {
            json!({"step_index": index, "type": "PLANNER_RESPONSE", "status": "DONE",
                "tool_calls": [{"name": "send_message", "args": {"Recipient": sub, "Message": "count the tests too"}}]})
        };
        let (entries, _) = read_all(&[
            json!({"step_index": 1, "type": "PLANNER_RESPONSE", "status": "DONE",
                "tool_calls": [{"name": "invoke_subagent", "args": {"Prompt": "count files", "TypeName": "self"}}]}),
            generic(2, "DONE", &format!("Created the following subagent:\n{{\n  \"conversationId\": \"{sub}\"\n}}")),
            message(3),
            generic(4, "DONE", "Message sent."),
            idle(5, "12 files"),
            message(6),
            generic(7, "DONE", "Message sent."),
            idle(8, "3 tests"),
        ]);
        let tasks: Vec<&Task> =
            entries.iter().filter_map(|entry| if let Entry::Task(task) = entry { Some(task) } else { None }).collect();
        assert_eq!(
            tasks,
            [
                &task(sub, "step-1-0", TaskStatus::Running, None),
                &task(sub, "step-1-0", TaskStatus::Completed, Some("12 files".into())),
                &task(sub, "step-6-0", TaskStatus::Running, None),
                &task(sub, "step-6-0", TaskStatus::Completed, Some("3 tests".into())),
            ],
            "the message to it at work woke nothing"
        );
    }

    #[test]
    fn a_subagent_runs_until_its_parent_hears_it_went_idle_and_goes_on() {
        let sub = "39497b42-d96f-4954-855d-69dd29a7d6f3";
        let (entries, marks) = read_all(&[
            json!({"step_index": 1, "type": "PLANNER_RESPONSE", "status": "DONE", "content": "Delegating.",
                "tool_calls": [{"name": "invoke_subagent", "args": {"Prompt": "count files", "TypeName": "self"}}]}),
            generic(
                2,
                "DONE",
                &format!(
                    "Created the following subagent:\n{{\n  \"conversationId\": \"{sub}\",\n  \"workspaceUris\": []\n}}\nThe subagent will send you a message."
                ),
            ),
            json!({"step_index": 3, "type": "PLANNER_RESPONSE", "status": "DONE", "content": "Waiting on it."}),
            json!({"step_index": 4, "source": "SYSTEM", "type": "SYSTEM_MESSAGE", "status": "DONE",
                "content": format!("<SYSTEM_MESSAGE>\n[Message] content=Subagent {sub} has gone idle. This was the subagent's most recent response: 12 files\n</SYSTEM_MESSAGE>")}),
            json!({"step_index": 5, "type": "PLANNER_RESPONSE", "status": "DONE", "content": "It counted 12."}),
        ]);
        let tasks: Vec<&Task> =
            entries.iter().filter_map(|entry| if let Entry::Task(task) = entry { Some(task) } else { None }).collect();
        assert_eq!(
            tasks,
            [
                &task(sub, "step-1-0", TaskStatus::Running, None),
                &task(sub, "step-1-0", TaskStatus::Completed, Some("12 files".into()))
            ]
        );
        assert!(entries.contains(&Entry::Woke { text: "A subagent finished, and the agent went on by itself.".into() }));
        assert_eq!(marks.iter().filter(|mark| **mark == Mark::Stopped).count(), 2, "{marks:?}");
        assert_eq!(marks.iter().filter(|mark| **mark == Mark::PromptSubmitted).count(), 1, "the turn it opened");
    }

    /// Records as agy 1.5.8 wrote them for a command that outlasted its
    /// wait, and for one stopped with `manage_task`.
    #[test]
    fn a_command_left_in_the_background_runs_until_a_notice_says_it_ended() {
        let id = "1e7426aa-34e6-4cb9-a165-266f7f750de9/task-2";
        let started = |index: u64, line: &str| {
            [
                json!({"step_index": index, "type": "PLANNER_RESPONSE", "status": "DONE",
                    "tool_calls": [{"name": "run_command", "args": {"CommandLine": line, "WaitMsBeforeAsync": 500}}]}),
                json!({"step_index": index + 1, "type": "GENERIC", "status": "RUNNING",
                    "content": format!("Created At: 2026-11-13T13:33:59Z\nTool is running as a background task with task id: {id}\nTask Description: {line}\nTask logs are available at: file:///h/task-2.log")}),
            ]
        };
        let tasks = |entries: &[Entry]| -> Vec<(TaskStatus, Option<String>)> {
            entries
                .iter()
                .filter_map(|entry| if let Entry::Task(task) = entry { Some((task.status, task.summary.clone())) } else { None })
                .collect()
        };
        let mut lines = started(1, "sleep 6; echo bg-finished").to_vec();
        lines.extend([
            json!({"step_index": 3, "type": "PLANNER_RESPONSE", "status": "DONE", "content": "It runs in the background."}),
            json!({"step_index": 4, "type": "SYSTEM_MESSAGE", "status": "DONE",
                "content": format!("<SYSTEM_MESSAGE>\n[Message] sender={id} content=Task id \"{id}\" finished with result:\nCommand exited with code 0.\n\nCommand output:\nbg-finished\r\n\nLog: file:///h/task-2.log\n</SYSTEM_MESSAGE>")}),
            json!({"step_index": 5, "type": "PLANNER_RESPONSE", "status": "DONE", "content": "Noted the end."}),
        ]);
        let (entries, _) = read_all(&lines);
        assert!(
            entries.iter().any(|entry| matches!(entry, Entry::ToolResult { is_error: false, .. })),
            "a call that went on isn't a failure"
        );
        assert_eq!(
            tasks(&entries),
            [
                (TaskStatus::Running, None),
                (TaskStatus::Completed, Some("Command exited with code 0.\n\nCommand output:\nbg-finished".into()))
            ]
        );
        assert!(entries.contains(&Entry::Woke {
            text: "`sleep 6; echo bg-finished` finished in the background, and the agent went on by itself.".into()
        }));

        let mut lines = started(1, "sleep 30").to_vec();
        lines.extend([
            json!({"step_index": 3, "type": "PLANNER_RESPONSE", "status": "DONE",
                "tool_calls": [{"name": "manage_task", "args": {"Action": "kill", "TaskId": "task-2"}}]}),
            generic(4, "DONE", &format!("Task \"{id}\" cancelled.")),
        ]);
        let (entries, _) = read_all(&lines);
        assert_eq!(tasks(&entries), [(TaskStatus::Running, None), (TaskStatus::Stopped, None)]);
    }

    #[test]
    fn long_outputs_and_task_logs_go_with_the_conversation() {
        let dir = tempfile::tempdir().expect("temp dir");
        for file in ["steps/2/output.txt", "steps/4/output.txt", "tasks/task-2.log", "messages/read.json"] {
            let path = dir.path().join(file);
            fs::create_dir_all(path.parent().expect("dir")).expect("mkdir");
            fs::write(path, "out").expect("write");
        }
        let names: Vec<String> = outputs_in(dir.path()).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["steps/2/output.txt", "steps/4/output.txt", "tasks/task-2.log"]);
    }

    #[test]
    fn workspaces_read_as_paths() {
        assert_eq!(first_workspace(r#"["file:///tmp/proj/sub%20dir"]"#), Some(PathBuf::from("/tmp/proj/sub dir")));
        assert_eq!(file_path("file:///C:/Users/me/proj"), Some(PathBuf::from("C:\\Users\\me\\proj")));
        assert_eq!(first_workspace("[]"), None);
        assert_eq!(file_path("file:///bad%zz"), None);
    }

    #[test]
    fn a_transcript_names_its_conversation() {
        let transcript = transcript_in(Path::new("/h/.gemini/antigravity-cli"), "6fa48f18-aae5-426b-82a7-92400134ab4f");
        assert_eq!(transcript_id(&transcript).as_deref(), Some("6fa48f18-aae5-426b-82a7-92400134ab4f"));
    }
}
