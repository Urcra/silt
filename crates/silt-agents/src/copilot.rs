//! GitHub Copilot CLI: each session is a directory
//! `$COPILOT_HOME/session-state/<id>/` with an append-only `events.jsonl`
//! and a `workspace.yaml` naming its directory (docs/research/agents.md).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::conventions::NativeConventions;
use crate::{Agent, Entry, Found, Launch, LaunchPlan, Mark, Read, Reader, RecordedSession, Rewrites, Role, SessionIds, Start, translated_text};

pub const AGENT: &str = "copilot-cli";

/// The version translated sessions claim to come from; Copilot requires one.
const TRANSLATED_VERSION: &str = "1.0.100";

pub fn home() -> Option<PathBuf> {
    crate::agent_home("COPILOT_HOME", ".copilot")
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
    match kind {
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
            for request in requests {
                let field = |key: &str| request.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
                read.entries.push(Entry::ToolCall {
                    id: field("toolCallId"),
                    name: field("name"),
                    input: request.get("arguments").cloned().unwrap_or(Value::Null),
                });
            }
        }
        "assistant.reasoning" => read.entries.push(Entry::Thinking { text: text("content") }),
        "tool.execution_start" => read.marks.push(Mark::ToolStarted {
            tool_use_id: text("toolCallId"),
            tool: text("toolName"),
        }),
        "tool.execution_complete" => {
            let success = data.get("success").and_then(Value::as_bool).unwrap_or(true);
            let result = data
                .pointer("/result/content")
                .or_else(|| data.pointer("/error/message"))
                .map(|value| value.as_str().map_or_else(|| value.to_string(), str::to_owned))
                .unwrap_or_default();
            read.marks.push(Mark::ToolFinished { tool_use_id: text("toolCallId") });
            read.entries.push(Entry::ToolResult {
                tool_use_id: text("toolCallId"),
                text: result,
                is_error: !success,
            });
        }
        "session.compaction_complete" => {
            let summary = text("summaryContent");
            read.entries.push(Entry::Compaction {
                summary: (!summary.is_empty()).then_some(summary),
            });
        }
        other => read.entries.push(Entry::Other { kind: other.to_owned() }),
    }
    read
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

/// Writes `session` as a session directory in the Copilot home `home`.
pub fn restore_into(home: &Path, root: &Path, cwd: &Path, session: &RecordedSession) -> io::Result<PathBuf> {
    let dir = session_dir(home, &session.id);
    let rewrites = Rewrites::new(session, Some(&dir.to_string_lossy()), &cwd.to_string_lossy(), &root.to_string_lossy());
    fs::create_dir_all(&dir)?;
    crate::replace_transcript(&dir.join("events.jsonl"), &rewrites.lines(&session.main))?;
    fs::write(dir.join("workspace.yaml"), workspace_yaml(&session.id, cwd, root))?;
    for (name, bytes) in &session.attachments {
        if crate::is_side_file_name(name) {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().expect("joined onto the session dir"))?;
            fs::write(path, bytes)?;
        }
    }
    Ok(dir.join("events.jsonl"))
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

    fn transcript(&self, _cwd: &Path, id: &str) -> Option<PathBuf> {
        Some(session_dir(&home()?, id).join("events.jsonl"))
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
        discover_in(&home()?, cwd, since_ms, known)
    }

    fn restore(&self, root: &Path, cwd: &Path, session: &RecordedSession) -> io::Result<PathBuf> {
        let home = home().ok_or_else(|| io::Error::other("HOME is not set"))?;
        restore_into(&home, root, cwd, session)
    }

    fn translate(&self, entries: &[Entry], cwd: &Path, id: &str) -> Option<Vec<String>> {
        Some(event_lines(&translated_text(entries), cwd, id))
    }

    fn launch(&self, plan: &LaunchPlan) -> io::Result<Launch> {
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
        json!({"name": "silt", "description": "Silt's skills, instructions and notices", "version": env!("CARGO_PKG_VERSION")}).to_string(),
    )?;
    crate::link_skills(&dir.join("skills"), &plan.skills)?;
    let command = |bash: String| json!([{"type": "command", "bash": bash, "timeoutSec": 30}]);
    let mut hooks = serde_json::Map::new();
    if let Some(instructions) = &plan.instructions {
        let context = dir.join("session-start.json");
        fs::write(&context, json!({ "additionalContext": instructions }).to_string())?;
        hooks.insert("sessionStart".into(), command(format!("cat {}", crate::shell_quote(&context.to_string_lossy()))));
    }
    hooks.insert(
        "userPromptSubmitted".into(),
        command(format!("{} --agent copilot UserPromptSubmit", plan.hook_command)),
    );
    fs::create_dir_all(dir.join("hooks"))?;
    fs::write(dir.join("hooks/hooks.json"), json!({"version": 1, "hooks": hooks}).to_string())?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_call_without_tools_ends_the_turn_when_it_ends() {
        let event = |kind: &str, data: &str| format!(r#"{{"type":"{kind}","data":{data},"id":"x","timestamp":"t","parentId":null}}"#);
        let mut reader = CopilotReader::default();
        let mut marks = |line: String| reader.push(&line).marks;
        assert_eq!(marks(event("user.message", r#"{"content":"go"}"#)), vec![Mark::PromptSubmitted]);
        assert_eq!(marks(event("assistant.turn_start", r#"{"turnId":"0"}"#)), Vec::new());
        let asks = r#"{"content":"Editing.","toolRequests":[{"toolCallId":"c1","name":"edit"}]}"#;
        assert_eq!(marks(event("assistant.message", asks)), Vec::new());
        assert_eq!(marks(event("assistant.turn_end", r#"{"turnId":"0"}"#)), Vec::new(), "tools come next");
        assert_eq!(marks(event("assistant.turn_start", r#"{"turnId":"1"}"#)), Vec::new());
        let answers = r#"{"content":"Done.","toolRequests":[]}"#;
        assert_eq!(marks(event("assistant.message", answers)), Vec::new(), "the call may still go on");
        assert_eq!(marks(event("assistant.turn_end", r#"{"turnId":"1"}"#)), vec![Mark::Stopped]);
        assert_eq!(marks(event("assistant.turn_start", r#"{"turnId":"2"}"#)), Vec::new());
        assert_eq!(marks(event("assistant.turn_end", r#"{"turnId":"2"}"#)), vec![Mark::Stopped], "an empty or cancelled reply");
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
            Entry::ToolCall {
                id: "call_1".into(),
                name: "bash".into(),
                input: json!({"command": "ls"})
            }
        );
        assert_eq!(
            entries[3],
            Entry::ToolResult {
                tool_use_id: "call_1".into(),
                text: "a.txt".into(),
                is_error: false
            }
        );
        assert_eq!(entries[4], Entry::Assistant { text: "There is a.txt.".into() });
        let marks: Vec<Mark> = reads.into_iter().flat_map(|read| read.marks).collect();
        assert_eq!(
            marks,
            vec![
                Mark::PromptSubmitted,
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
}
