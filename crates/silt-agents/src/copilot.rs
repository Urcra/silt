//! GitHub Copilot CLI: each session is a directory
//! `$COPILOT_HOME/session-state/<id>/` with an append-only `events.jsonl`
//! and a `workspace.yaml` naming its directory (docs/research/agents.md).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::conventions::NativeConventions;
use crate::{
    Agent, AgentError, CheckedSession, Entry, Found, Launch, LaunchPlan, Mark, OnDisk, Read, Reader, Restore, Rewrites, Role,
    SessionIds, Start, Task, TaskKind, TaskStatus, Write, translated_text,
};

pub const AGENT: &str = "copilot-cli";

/// The version translated sessions claim to come from; Copilot requires one.
const TRANSLATED_VERSION: &str = "1.0.100";

pub fn home() -> Option<PathBuf> {
    crate::agent_home("COPILOT_HOME", ".copilot")
}

/// [`home`] for a session in `cwd`, whose workspace may set its own (ADR 0065).
fn home_in(cwd: &Path) -> Option<PathBuf> {
    crate::agent_home_in("COPILOT_HOME", ".copilot", cwd)
}

fn session_dir(home: &Path, id: &str) -> PathBuf {
    home.join("session-state").join(id)
}

/// Reads events, ending the turn when a model call that asked for no tool
/// ends: also when the reply was empty or cancelled.
#[derive(Default)]
pub struct CopilotReader {
    /// Inside `assistant.turn_start` … `turn_end`, which older versions
    /// don't write; a reply without tools then ends the turn itself.
    in_call: bool,
    asked_for_tools: bool,
}

impl Reader for CopilotReader {
    fn push(&mut self, line: &str) -> Read {
        let mut read = read_line(line);
        // A subagent's events neither start nor end the session's turns.
        if read.side.is_some() {
            read.marks.clear();
            return read;
        }
        let kind = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|value| value.get("type").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default();
        match kind.as_str() {
            "assistant.turn_start" => {
                self.in_call = true;
                self.asked_for_tools = false;
            }
            "assistant.message" => {
                self.asked_for_tools |= read.entries.iter().any(|entry| matches!(entry, Entry::ToolCall { .. }));
                if self.in_call {
                    read.marks.retain(|mark| *mark != Mark::Stopped);
                }
            }
            "assistant.turn_end" => {
                if self.in_call && !self.asked_for_tools {
                    read.marks.push(Mark::Stopped);
                }
                self.in_call = false;
            }
            _ => {}
        }
        read
    }
}

/// The conversation steps and turn marks in one event.
pub fn read_line(line: &str) -> Read {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Read::unparseable();
    };
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("unknown");
    let data = value.get("data").cloned().unwrap_or(Value::Null);
    let text = |key: &str| match data.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(other) if !other.is_null() => other.to_string(),
        _ => String::new(),
    };
    let mut read = Read::default();
    let agent = value.get("agentId").and_then(Value::as_str).filter(|agent| crate::is_safe_id(agent)).map(str::to_owned);
    if !kind.starts_with("subagent.") {
        read.side.clone_from(&agent);
    }
    let subagent = |status: TaskStatus, call: Option<String>, summary: Option<String>| {
        agent.clone().map(|agent| Task {
            id: agent.clone(),
            call,
            kind: Some(TaskKind::Subagent),
            status,
            side: Some(agent),
            summary,
        })
    };
    match kind {
        "subagent.started" => {
            let name = ["agentDisplayName", "agentName"].iter().find_map(|key| data.get(*key).and_then(Value::as_str));
            let call = Some(text("toolCallId")).filter(|call| !call.is_empty());
            read.entries.extend(subagent(TaskStatus::Running, call, name.map(str::to_owned)).map(Entry::Task));
        }
        "subagent.completed" => {
            let cancelled = data.get("cancelled").and_then(Value::as_bool) == Some(true);
            let status = if cancelled { TaskStatus::Stopped } else { TaskStatus::Completed };
            read.entries.extend(subagent(status, None, None).map(Entry::Task));
        }
        "subagent.failed" => {
            let error = Some(text("error")).filter(|error| !error.is_empty());
            read.entries.extend(subagent(TaskStatus::Failed, None, error).map(Entry::Task));
        }
        "system.notification" => match notified(&data) {
            Some(task) => read.entries.push(Entry::Task(task)),
            None => read.entries.push(Entry::Other { kind: kind.to_owned() }),
        },
        "user.message" => {
            read.marks.push(Mark::PromptSubmitted);
            read.entries.push(Entry::User { text: text("content") });
        }
        "assistant.message" => {
            let content = text("content");
            let requests = data.get("toolRequests").and_then(Value::as_array).cloned().unwrap_or_default();
            if !content.is_empty() {
                read.entries.push(Entry::Assistant { text: content });
                if requests.is_empty() {
                    read.marks.push(Mark::Stopped);
                }
            }
            // The one tool asked for is under way from here: Copilot writes
            // `tool.execution_start` after the tool's edits can land, so a
            // busy machine could see them first and credit no tool. With
            // several asked for, which one edited waits for their starts.
            if let [request] = requests.as_slice() {
                let field = |key: &str| request.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
                read.marks.push(Mark::ToolStarted { tool_use_id: field("toolCallId"), tool: field("name") });
            }
            for request in requests {
                let field = |key: &str| request.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
                read.entries.push(Entry::ToolCall {
                    id: field("toolCallId"),
                    name: field("name"),
                    input: request.get("arguments").cloned().unwrap_or(Value::Null),
                    kind: None,
                });
            }
        }
        "assistant.reasoning" => read.entries.push(Entry::Thinking { text: text("content") }),
        "tool.execution_start" => read.marks.push(Mark::ToolStarted { tool_use_id: text("toolCallId"), tool: text("toolName") }),
        "tool.execution_complete" => {
            let success = data.get("success").and_then(Value::as_bool).unwrap_or(true);
            let result = data
                .pointer("/result/content")
                .or_else(|| data.pointer("/error/message"))
                .map(|value| value.as_str().map_or_else(|| value.to_string(), str::to_owned))
                .unwrap_or_default();
            read.marks.push(Mark::ToolFinished { tool_use_id: text("toolCallId") });
            read.entries.push(Entry::ToolResult { tool_use_id: text("toolCallId"), text: result, is_error: !success });
        }
        "session.compaction_complete" => {
            let summary = text("summaryContent");
            read.entries.push(Entry::Compaction { summary: (!summary.is_empty()).then_some(summary) });
        }
        other => read.entries.push(Entry::Other { kind: other.to_owned() }),
    }
    read
}

/// What a `system.notification` says ended: a shell left running, or a
/// subagent.
fn notified(data: &Value) -> Option<Task> {
    let kind = data.get("kind")?;
    let field = |key: &str| kind.get(key).and_then(Value::as_str).map(str::to_owned);
    match kind.get("type")?.as_str()? {
        "shell_completed" | "shell_detached_completed" => {
            let code = kind.get("exitCode").and_then(Value::as_i64);
            Some(Task {
                id: field("shellId")?,
                call: None,
                kind: Some(TaskKind::Background),
                status: if code.is_none_or(|code| code == 0) { TaskStatus::Completed } else { TaskStatus::Failed },
                side: None,
                summary: field("description"),
            })
        }
        "agent_completed" => {
            let agent = field("agentId").filter(|agent| crate::is_safe_id(agent))?;
            Some(Task {
                id: agent.clone(),
                call: None,
                kind: Some(TaskKind::Subagent),
                status: field("status").map_or(TaskStatus::Completed, |status| TaskStatus::parse(&status)),
                side: Some(agent),
                summary: None,
            })
        }
        _ => None,
    }
}

/// The newest session started in `cwd` no earlier than `since_ms` in the
/// Copilot home `home`, whose id isn't in `known`.
pub fn discover_in(home: &Path, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
    let cwd = cwd.to_string_lossy();
    fs::read_dir(home.join("session-state"))
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("events.jsonl"))
        .filter(|path| crate::modified_ms(path).is_some_and(|modified| modified >= since_ms))
        .filter_map(|path| {
            let id = path.parent()?.file_name()?.to_string_lossy().into_owned();
            let start: Value = serde_json::from_str(&crate::first_line(&path)?).ok()?;
            (start.pointer("/data/context/cwd")?.as_str()? == cwd && !known.contains(&id)).then_some((id, path))
        })
        .max_by_key(|(_, path)| crate::modified_ms(path))
        .map(|(id, transcript)| Found { id, transcript })
}

/// Every session in the Copilot home `home`, with where and when it started.
pub fn on_disk_in(home: &Path) -> Vec<OnDisk> {
    let Ok(entries) = fs::read_dir(home.join("session-state")) else {
        return Vec::new();
    };
    let mut found: Vec<OnDisk> = entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().into_owned();
            let transcript = entry.path().join("events.jsonl");
            if !crate::is_safe_id(&id) || !transcript.is_file() {
                return None;
            }
            let first = crate::first_line(&transcript).unwrap_or_default();
            let start: Value = serde_json::from_str(&first).unwrap_or_default();
            let cwd = start.pointer("/data/context/cwd").and_then(Value::as_str).map(PathBuf::from);
            Some(OnDisk { id, cwd, started_ms: crate::record_time(&first), transcript })
        })
        .collect();
    found.sort_by(|a, b| a.started_ms.cmp(&b.started_ms).then_with(|| a.id.cmp(&b.id)));
    found
}

/// Writes `session` as a session directory in the Copilot home `home`.
pub fn restore_into(home: &Path, root: &Path, cwd: &Path, session: &CheckedSession) -> Result<PathBuf, AgentError> {
    let restore = plan_restore(home, root, cwd, session);
    restore.apply()?;
    Ok(restore.transcript)
}

/// The files restoring `session` into the Copilot home `home` writes.
pub fn plan_restore(home: &Path, root: &Path, cwd: &Path, session: &CheckedSession) -> Restore {
    let dir = session_dir(home, &session.id);
    let rewrites = Rewrites::new(session, Some(&dir.to_string_lossy()), &cwd.to_string_lossy(), &root.to_string_lossy());
    let events = dir.join("events.jsonl");
    let mut restore = Restore {
        transcript: events.clone(),
        writes: vec![
            Write::Transcript { path: events, lines: rewrites.lines(&session.main) },
            Write::File { path: dir.join("workspace.yaml"), bytes: workspace_yaml(&session.id, cwd, root).into_bytes() },
        ],
    };
    restore.side_files(&dir, session);
    restore
}

fn workspace_yaml(id: &str, cwd: &Path, root: &Path) -> String {
    let now = crate::time::timestamp_now();
    let quote = |value: &str| serde_json::to_string(value).expect("strings serialize");
    format!(
        "id: {id}\ncwd: {}\ngit_root: {}\nclient_name: github/cli\nsummary_count: 0\nfork_count: 0\ncreated_at: {now}\nupdated_at: {now}\n",
        quote(&cwd.to_string_lossy()),
        quote(&root.to_string_lossy()),
    )
}

/// Events holding `messages` as plain text, chained by `parentId`.
pub fn event_lines(messages: &[(Role, String)], cwd: &Path, id: &str) -> Vec<String> {
    let timestamp = crate::time::timestamp_now();
    let mut parent: Option<String> = None;
    let mut lines = Vec::new();
    let mut push = |kind: &str, data: Value| {
        // Copilot rejects events whose ids aren't UUIDs.
        let event_id = crate::derived_uuid(id, lines.len());
        lines.push(json!({"type": kind, "data": data, "id": event_id, "timestamp": timestamp, "parentId": parent}).to_string());
        parent = Some(event_id);
    };
    push(
        "session.start",
        json!({"sessionId": id, "version": 1, "producer": "copilot-agent", "copilotVersion": TRANSLATED_VERSION, "startTime": timestamp, "context": {"cwd": cwd.to_string_lossy(), "gitRoot": cwd.to_string_lossy()}}),
    );
    for (index, (role, text)) in messages.iter().enumerate() {
        let turn = index.to_string();
        match role {
            Role::User => push(
                "user.message",
                json!({"content": text, "transformedContent": text, "messageId": format!("m{index}"), "turnId": turn}),
            ),
            Role::Assistant => push(
                "assistant.message",
                json!({"messageId": format!("m{index}"), "content": text, "toolRequests": [], "turnId": turn}),
            ),
        }
    }
    lines
}

/// The GitHub Copilot CLI adapter.
pub struct Copilot;

impl Agent for Copilot {
    fn id(&self) -> &'static str {
        AGENT
    }

    fn command(&self) -> &'static str {
        "copilot"
    }

    fn display_name(&self) -> &'static str {
        "GitHub Copilot CLI"
    }

    fn native(&self) -> NativeConventions {
        NativeConventions {
            rules: &["AGENTS.md", "CLAUDE.md", "GEMINI.md", ".github/copilot-instructions.md"],
            skill_dirs: &[".github/skills", ".agents/skills", ".claude/skills"],
            personal_skills: true,
            given_skills: true,
        }
    }

    fn reader(&self) -> Box<dyn Reader> {
        Box::<CopilotReader>::default()
    }

    fn session_ids(&self) -> SessionIds {
        SessionIds::Chosen
    }

    fn transcript(&self, cwd: &Path, id: &str) -> Option<PathBuf> {
        Some(session_dir(&home_in(cwd)?, id).join("events.jsonl"))
    }

    fn session_dir(&self, transcript: &Path) -> Option<PathBuf> {
        transcript.parent().map(Path::to_owned)
    }

    fn attachments(&self, transcript: &Path) -> Vec<(String, PathBuf)> {
        let Some(dir) = transcript.parent() else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(dir.join("checkpoints")) else {
            return Vec::new();
        };
        let mut found: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| (format!("checkpoints/{}", entry.file_name().to_string_lossy()), entry.path()))
            .collect();
        found.sort();
        found
    }

    fn discover(&self, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
        discover_in(&home_in(cwd)?, cwd, since_ms, known)
    }

    fn on_disk(&self, _dirs: &[PathBuf]) -> Vec<OnDisk> {
        crate::agent_homes("COPILOT_HOME", ".copilot").iter().flat_map(|home| on_disk_in(home)).collect()
    }

    fn restore(&self, root: &Path, cwd: &Path, session: &CheckedSession) -> Result<PathBuf, AgentError> {
        let home = home_in(cwd).ok_or(AgentError::NoHome)?;
        restore_into(&home, root, cwd, session)
    }

    fn translate(&self, entries: &[Entry], cwd: &Path, id: &str) -> Option<Vec<String>> {
        Some(event_lines(&translated_text(entries), cwd, id))
    }

    fn bypass_flag(&self, args: &[String]) -> Option<String> {
        crate::flag_in(args, &["--allow-all-tools", "--allow-all", "--yolo", "--allow-all-paths"], &[])
    }

    fn launch(&self, plan: &LaunchPlan) -> Result<Launch, AgentError> {
        let mut launch = Launch::new(self.command());
        let mut prompt = None;
        match &plan.start {
            Start::New { id: Some(id) } => {
                launch.arg("--session-id").arg(id);
            }
            Start::New { id: None } => {}
            Start::Resume { id } => {
                launch.arg("--resume").arg(id);
            }
            Start::Primed { id, prompt: text } => {
                if let Some(id) = id {
                    launch.arg("--session-id").arg(id);
                }
                prompt = Some(text.clone());
            }
        }
        let mcp = json!({"mcpServers": {"silt": {"type": "local", "command": plan.mcp.command.to_string_lossy(), "args": plan.mcp.args, "tools": ["*"]}}});
        launch.arg("--additional-mcp-config").arg(mcp.to_string());
        launch.arg("--plugin-dir").arg(plugin(plan)?);
        for arg in &plan.args {
            launch.arg(arg);
        }
        if let Some(prompt) = prompt {
            launch.arg("--interactive").arg(prompt);
        }
        Ok(launch)
    }
}

/// A plugin for this run: the given skills, the session's instructions at
/// session start, and Silt's notices on each prompt.
fn plugin(plan: &LaunchPlan) -> io::Result<PathBuf> {
    let dir = plan.scratch.join("copilot-plugin");
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(dir.join(".plugin"))?;
    fs::write(
        dir.join(".plugin/plugin.json"),
        json!({"name": "silt", "description": "Silt's skills, instructions and notices", "version": env!("CARGO_PKG_VERSION")})
            .to_string(),
    )?;
    crate::link_skills(&dir.join("skills"), &plan.skills)?;
    // Copilot runs `powershell` on Windows and `bash` elsewhere.
    let command =
        |bash: String, powershell: String| json!([{"type": "command", "bash": bash, "powershell": powershell, "timeoutSec": 30}]);
    let mut hooks = serde_json::Map::new();
    if let Some(instructions) = &plan.instructions {
        let context = dir.join("session-start.json");
        fs::write(&context, json!({ "additionalContext": instructions }).to_string())?;
        let context = context.to_string_lossy();
        hooks.insert(
            "sessionStart".into(),
            command(
                format!("cat {}", crate::shell_quote(&context)),
                format!("Get-Content -Raw -LiteralPath {}", crate::powershell_quote(&context)),
            ),
        );
    }
    hooks.insert(
        "userPromptSubmitted".into(),
        command(
            format!("{} --agent copilot UserPromptSubmit", plan.hook_command),
            format!("& {} hook --agent copilot UserPromptSubmit", crate::powershell_quote(&plan.hook_program.to_string_lossy())),
        ),
    );
    fs::create_dir_all(dir.join("hooks"))?;
    fs::write(dir.join("hooks/hooks.json"), json!({"version": 1, "hooks": hooks}).to_string())?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restore_plans_the_session_directory_with_its_side_files() {
        let session = crate::RecordedSession {
            id: "new".into(),
            former_ids: vec!["old".into()],
            main: vec![r#"{"sessionId":"old"}"#.into()],
            attachments: vec![("files/notes.md".into(), b"notes".to_vec())],
            ..Default::default()
        }
        .checked()
        .expect("safe");
        let restore = plan_restore(Path::new("/home"), Path::new("/repo"), Path::new("/repo"), &session);
        let dir = session_dir(Path::new("/home"), "new");
        assert_eq!(restore.transcript, dir.join("events.jsonl"));
        let paths: Vec<&Path> = restore
            .writes
            .iter()
            .map(|write| match write {
                Write::Transcript { path, .. } | Write::File { path, .. } => path.as_path(),
            })
            .collect();
        assert_eq!(paths, [dir.join("events.jsonl"), dir.join("workspace.yaml"), dir.join("files/notes.md")]);
        assert!(matches!(&restore.writes[0], Write::Transcript { lines, .. } if lines.contains(r#""sessionId":"new""#)));
    }

    #[test]
    fn the_one_tool_asked_for_starts_with_the_ask() {
        let asks = r#"{"type":"assistant.message","data":{"content":"","toolRequests":[{"toolCallId":"c1","name":"edit"}]}}"#;
        let started = Mark::ToolStarted { tool_use_id: "c1".into(), tool: "edit".into() };
        assert_eq!(read_line(asks).marks, std::slice::from_ref(&started));
        let starts = r#"{"type":"tool.execution_start","data":{"toolCallId":"c1","toolName":"edit"}}"#;
        assert_eq!(read_line(starts).marks, [started], "and again when it starts, which the log records once");
        let two = r#"{"type":"assistant.message","data":{"content":"","toolRequests":[{"toolCallId":"c1","name":"edit"},{"toolCallId":"c2","name":"bash"}]}}"#;
        assert!(read_line(two).marks.is_empty(), "with two, either may edit");
    }

    #[test]
    fn a_model_call_without_tools_ends_the_turn_when_it_ends() {
        let event =
            |kind: &str, data: &str| format!(r#"{{"type":"{kind}","data":{data},"id":"x","timestamp":"t","parentId":null}}"#);
        let mut reader = CopilotReader::default();
        let mut marks = |line: String| reader.push(&line).marks;
        assert_eq!(marks(event("user.message", r#"{"content":"go"}"#)), vec![Mark::PromptSubmitted]);
        assert_eq!(marks(event("assistant.turn_start", r#"{"turnId":"0"}"#)), Vec::new());
        let asks = r#"{"content":"Editing.","toolRequests":[{"toolCallId":"c1","name":"edit"}]}"#;
        assert_eq!(
            marks(event("assistant.message", asks)),
            vec![Mark::ToolStarted { tool_use_id: "c1".into(), tool: "edit".into() }]
        );
        assert_eq!(marks(event("assistant.turn_end", r#"{"turnId":"0"}"#)), Vec::new(), "tools come next");
        assert_eq!(marks(event("assistant.turn_start", r#"{"turnId":"1"}"#)), Vec::new());
        let answers = r#"{"content":"Done.","toolRequests":[]}"#;
        assert_eq!(marks(event("assistant.message", answers)), Vec::new(), "the call may still go on");
        assert_eq!(marks(event("assistant.turn_end", r#"{"turnId":"1"}"#)), vec![Mark::Stopped]);
        assert_eq!(marks(event("assistant.turn_start", r#"{"turnId":"2"}"#)), Vec::new());
        assert_eq!(marks(event("assistant.turn_end", r#"{"turnId":"2"}"#)), vec![Mark::Stopped], "an empty or cancelled reply");
    }

    #[test]
    fn a_subagent_s_events_are_its_own_and_its_news_ties_it_to_its_call() {
        let mut reader = CopilotReader::default();
        let started = reader.push(r#"{"type":"subagent.started","agentId":"math-helper-0","data":{"toolCallId":"call_1","agentName":"math","agentDisplayName":"Math helper","executionMode":"background"}}"#);
        assert_eq!(started.side, None);
        assert_eq!(
            started.entries,
            vec![Entry::Task(Task {
                id: "math-helper-0".into(),
                call: Some("call_1".into()),
                kind: Some(TaskKind::Subagent),
                status: TaskStatus::Running,
                side: Some("math-helper-0".into()),
                summary: Some("Math helper".into()),
            })]
        );
        let said = reader.push(r#"{"type":"user.message","agentId":"math-helper-0","data":{"content":"Add 2 and 2"}}"#);
        assert_eq!((said.side.as_deref(), said.marks.len()), (Some("math-helper-0"), 0));
        let done =
            reader.push(r#"{"type":"subagent.completed","agentId":"math-helper-0","data":{"durationMs":10,"cancelled":true}}"#);
        assert!(matches!(&done.entries[0], Entry::Task(Task { status: TaskStatus::Stopped, .. })));
        let shell = reader.push(r#"{"type":"system.notification","data":{"content":"<system_notification>done","kind":{"type":"shell_completed","shellId":"sh-3","exitCode":1,"description":"npm test"}}}"#);
        assert!(
            matches!(&shell.entries[0], Entry::Task(Task { id, status: TaskStatus::Failed, kind: Some(TaskKind::Background), .. }) if id == "sh-3")
        );
        assert_eq!(reader.push(r#"{"type":"user.message","data":{"content":"next"}}"#).marks, vec![Mark::PromptSubmitted]);
    }

    #[test]
    fn reads_events() {
        let lines = [
            r#"{"type":"session.start","data":{"sessionId":"s","context":{"cwd":"/w"}},"id":"1","timestamp":"t","parentId":null}"#,
            r#"{"type":"user.message","data":{"content":"List files","transformedContent":"List files"},"id":"2","timestamp":"t","parentId":"1"}"#,
            r#"{"type":"assistant.message","data":{"content":"","toolRequests":[{"toolCallId":"call_1","name":"bash","arguments":{"command":"ls"}}]},"id":"3","timestamp":"t","parentId":"2"}"#,
            r#"{"type":"tool.execution_start","data":{"toolCallId":"call_1","toolName":"bash"},"id":"4","timestamp":"t","parentId":"3"}"#,
            r#"{"type":"tool.execution_complete","data":{"toolCallId":"call_1","success":true,"result":{"content":"a.txt"}},"id":"5","timestamp":"t","parentId":"4"}"#,
            r#"{"type":"assistant.message","data":{"content":"There is a.txt.","toolRequests":[]},"id":"6","timestamp":"t","parentId":"5"}"#,
        ];
        let reads: Vec<Read> = lines.iter().map(|line| read_line(line)).collect();
        let entries: Vec<Entry> = reads.iter().flat_map(|read| read.entries.clone()).collect();
        assert_eq!(entries[1], Entry::User { text: "List files".into() });
        assert_eq!(
            entries[2],
            Entry::ToolCall { id: "call_1".into(), name: "bash".into(), input: json!({"command": "ls"}), kind: None }
        );
        assert_eq!(entries[3], Entry::ToolResult { tool_use_id: "call_1".into(), text: "a.txt".into(), is_error: false });
        assert_eq!(entries[4], Entry::Assistant { text: "There is a.txt.".into() });
        let marks: Vec<Mark> = reads.into_iter().flat_map(|read| read.marks).collect();
        assert_eq!(
            marks,
            vec![
                Mark::PromptSubmitted,
                Mark::ToolStarted { tool_use_id: "call_1".into(), tool: "bash".into() },
                Mark::ToolStarted { tool_use_id: "call_1".into(), tool: "bash".into() },
                Mark::ToolFinished { tool_use_id: "call_1".into() },
                Mark::Stopped,
            ]
        );
    }

    #[test]
    fn translated_events_read_back() {
        let lines = event_lines(&[(Role::User, "hi".into()), (Role::Assistant, "hello".into())], Path::new("/w"), "s");
        let second: Value = serde_json::from_str(&lines[2]).expect("json");
        let first: Value = serde_json::from_str(&lines[1]).expect("json");
        assert_eq!(second["parentId"], first["id"]);
        for line in &lines {
            let event: Value = serde_json::from_str(line).expect("json");
            assert!(uuid::Uuid::parse_str(event["id"].as_str().expect("id")).is_ok(), "Copilot needs UUID ids: {event}");
        }
        let start: Value = serde_json::from_str(&lines[0]).expect("json");
        assert!(start["data"]["copilotVersion"].is_string());
        assert_eq!(read_line(&lines[2]).entries, vec![Entry::Assistant { text: "hello".into() }]);
    }

    #[test]
    fn sessions_on_disk_are_listed_with_where_they_started() {
        let home = tempfile::tempdir().expect("temp dir");
        let dir = home.path().join("session-state/s1");
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(
            dir.join("events.jsonl"),
            "{\"type\":\"session.start\",\"timestamp\":\"2026-01-02T03:04:05.000Z\",\"data\":{\"context\":{\"cwd\":\"/w/repo\"}}}\n",
        )
        .expect("writes");
        fs::create_dir_all(home.path().join("session-state/empty")).expect("mkdir");
        let found = on_disk_in(home.path());
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].cwd.as_deref(), Some(Path::new("/w/repo")));
        assert!(found[0].started_ms.is_some());
    }
}
