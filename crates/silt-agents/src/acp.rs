//! Sessions of agents the web UI starts over ACP (ADR 0014). Silt writes
//! these records itself as the agent works, one JSON object per line:
//! `session`, `prompt`, `wake`, `message`, `thought`, `tool_call`,
//! `tool_result`, `plan` and `stop`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::conventions::NativeConventions;
use crate::{
    Agent, AgentError, CheckedSession, Entry, Found, Launch, LaunchPlan, Mark, Read, Reader, SessionIds, Stateless, Task,
    TaskKind, TaskStatus,
};

pub const AGENT: &str = "acp";

/// Which agent ran the session: its registry id, its name, and its own id
/// for the session, to load it again.
pub fn session(id: &str, name: &str, native: &str) -> String {
    json!({"t": "session", "id": id, "name": name, "native": native}).to_string()
}

/// The registry id and native session id a `session` record names.
pub fn session_of(line: &str) -> Option<(String, String)> {
    let record: Value = serde_json::from_str(line).ok()?;
    (record["t"] == "session").then(|| {
        let field = |key: &str| record[key].as_str().unwrap_or_default().to_owned();
        (field("id"), field("native"))
    })
}

/// The agent's name a `session` record gives, such as "Claude Code".
pub fn name_of(line: &str) -> Option<String> {
    let record: Value = serde_json::from_str(line).ok()?;
    (record["t"] == "session").then(|| record["name"].as_str().map(str::to_owned)).flatten().filter(|name| !name.is_empty())
}

/// A prompt by `by`, naming the files attached to it; each file's content
/// is recorded beside the session under that name.
/// A prompt with the names of the files attached to it, and the images
/// among them as `(media type, base64)`, kept so the history can show them.
pub fn prompt(text: &str, by: &str, attached: &[String], images: &[(String, String)]) -> String {
    let mut record = json!({"t": "prompt", "text": text, "by": by});
    if !attached.is_empty() {
        record["attached"] = json!(attached);
    }
    if !images.is_empty() {
        record["images"] = images.iter().map(|(media_type, data)| json!({"media_type": media_type, "data": data})).collect();
    }
    record.to_string()
}

/// How a prompt reads with the files attached to it named after its text.
fn with_attached(text: String, record: &Value) -> String {
    let attached: Vec<&str> = record["attached"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if attached.is_empty() { text } else { format!("{text}\n\n(attached: {})", attached.join(", ")) }
}

/// What a turn the agent opened itself says it began with.
pub const WOKE: &str = "The agent went on by itself, without a prompt.";

/// The agent went on by itself, as Claude does once a background task it
/// started ends: a turn opens as with a prompt, but nobody said anything.
/// `cause` says what ended just before; `stopped`, that Silt stopped the
/// turn at once, as the person had stopped the agent.
pub fn wake(cause: Option<&str>, stopped: bool) -> String {
    let mut record = json!({"t": "wake"});
    if let Some(cause) = cause {
        record["cause"] = json!(cause);
    }
    if stopped {
        record["stopped"] = json!(true);
    }
    record.to_string()
}

/// How a turn the agent opened itself reads.
fn woke(cause: Option<&str>, stopped: bool) -> String {
    let mut text = match cause {
        Some(cause) => format!("{cause}, and the agent went on by itself."),
        None => WOKE.to_owned(),
    };
    if stopped {
        text.push_str(" Silt stopped it, as you had stopped the agent.");
    }
    text
}

pub fn message(text: &str) -> String {
    json!({"t": "message", "text": text}).to_string()
}

/// A thought, stamped with when it began: it is recorded once it ends, and
/// what follows it is when it ended, so the history can say how long it took.
pub fn thought(text: &str, started_ms: i64) -> String {
    json!({"t": "thought", "text": text, "timestamp": crate::time::format_timestamp(started_ms)}).to_string()
}

pub fn tool_call(id: &str, title: &str, kind: &str, input: Option<&Value>) -> String {
    json!({"t": "tool_call", "id": id, "title": title, "kind": kind, "input": input}).to_string()
}

/// What a tool call turned out to be once the agent said more of it: ACP
/// agents announce a call with a placeholder such as "Terminal", then
/// update it with its command. It names a call already started.
pub fn tool_update(id: &str, title: &str, kind: &str, input: Option<&Value>) -> String {
    json!({"t": "tool_call", "id": id, "title": title, "kind": kind, "input": input, "update": true}).to_string()
}

/// A tool's output as it reads: a message an earlier Silt kept as a JSON
/// string, quotes and `\n`s and all, unquoted, and without the one
/// Markdown fence an agent may wrap it in, such as Claude's ```` ```console ````.
fn readable(output: String) -> String {
    let output = match serde_json::from_str::<Value>(&output) {
        Ok(Value::String(text)) => text,
        _ => output,
    };
    unfenced(output)
}

fn unfenced(output: String) -> String {
    let trimmed = output.trim();
    let inner = trimmed
        .strip_prefix("```")
        .and_then(|rest| rest.strip_suffix("```"))
        .and_then(|inner| inner.split_once('\n'))
        .map(|(_language, body)| body.strip_suffix('\n').unwrap_or(body));
    match inner {
        Some(body) if !body.contains("\n```") => body.to_owned(),
        _ => output,
    }
}

pub fn tool_result(id: &str, status: &str, output: Option<&str>) -> String {
    json!({"t": "tool_result", "id": id, "status": status, "output": output}).to_string()
}

/// The agent's plan: each step and its status.
pub fn plan(steps: &[(String, String)]) -> String {
    let entries: Vec<Value> = steps.iter().map(|(step, status)| json!({"content": step, "status": status})).collect();
    json!({"t": "plan", "entries": entries}).to_string()
}

/// What a turn took, in tokens; `id` names the record, as one message's.
pub fn usage(id: &str, input: u64, output: u64) -> String {
    json!({"t": "usage", "id": id, "input": input, "output": output}).to_string()
}

/// A record of what a subagent did, inside the tool call `parent` that
/// started it; its entries are that subagent's, named by the call.
pub fn within(record: &str, parent: &str) -> String {
    let mut record: Value = serde_json::from_str(record).unwrap_or(Value::Null);
    if let Some(fields) = record.as_object_mut() {
        fields.insert("parent".into(), Value::String(parent.to_owned()));
    }
    record.to_string()
}

/// News of work the call `call` left running beside the turn: a subagent,
/// or a command in the background, with the agent's id for it.
pub fn task(call: &str, id: Option<&str>, kind: Option<&str>, status: &str) -> String {
    json!({"t": "task", "call": call, "id": id, "kind": kind, "status": status}).to_string()
}

pub fn stop(reason: &str) -> String {
    json!({"t": "stop", "reason": reason}).to_string()
}

pub fn read_line(line: &str) -> Read {
    let mut read = Read::default();
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return Read::unparseable();
    };
    let field = |key: &str| record[key].as_str().unwrap_or_default().to_owned();
    match record["t"].as_str().unwrap_or_default() {
        "prompt" => {
            read.marks.push(Mark::PromptSubmitted);
            read.entries.push(Entry::User { text: with_attached(field("text"), &record) });
            for image in record["images"].as_array().into_iter().flatten() {
                if let (Some(media_type), Some(data)) = (image["media_type"].as_str(), image["data"].as_str()) {
                    read.entries.push(Entry::Image { media_type: media_type.to_owned(), data: data.to_owned() });
                }
            }
        }
        "wake" => {
            read.marks.push(Mark::PromptSubmitted);
            read.entries.push(Entry::Woke { text: woke(record["cause"].as_str(), record["stopped"] == true) });
        }
        "message" => read.entries.push(Entry::Assistant { text: field("text") }),
        "thought" => read.entries.push(Entry::Thinking { text: field("text") }),
        "tool_call" => {
            if record["update"] != true {
                read.marks.push(Mark::ToolStarted { tool_use_id: field("id"), tool: field("title") });
            }
            read.entries.push(Entry::ToolCall {
                id: field("id"),
                name: field("title"),
                input: record["input"].clone(),
                kind: Some(field("kind")).filter(|kind| !kind.is_empty()),
            });
        }
        "tool_result" => {
            read.marks.push(Mark::ToolFinished { tool_use_id: field("id") });
            read.entries.push(Entry::ToolResult {
                tool_use_id: field("id"),
                text: readable(field("output")),
                is_error: field("status") == "failed",
            });
        }
        "plan" => {
            let steps = record["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| match entry {
                    // Earlier records kept each step's words alone.
                    Value::String(step) => Some((step.clone(), "pending".to_owned())),
                    entry => {
                        Some((entry["content"].as_str()?.to_owned(), entry["status"].as_str().unwrap_or("pending").to_owned()))
                    }
                })
                .collect();
            read.entries.push(Entry::Plan { steps });
        }
        "usage" => read.entries.push(Entry::Usage {
            message: field("id"),
            input: record["input"].as_u64().unwrap_or(0),
            output: record["output"].as_u64().unwrap_or(0),
        }),
        "stop" => {
            read.marks.push(Mark::Stopped);
            read.entries.push(Entry::Other { kind: "stop".into() });
        }
        "task" => {
            let call = field("call");
            let kind = match record["kind"].as_str() {
                Some("subagent") => Some(TaskKind::Subagent),
                Some("background") => Some(TaskKind::Background),
                _ => None,
            };
            read.entries.push(Entry::Task(Task {
                id: record["id"].as_str().map_or_else(|| call.clone(), str::to_owned),
                // A subagent's records name the call that started it.
                side: (kind == Some(TaskKind::Subagent)).then(|| call.clone()),
                call: Some(call),
                kind,
                status: TaskStatus::parse(&field("status")),
                summary: None,
            }));
        }
        other => read.entries.push(Entry::Other { kind: other.to_owned() }),
    }
    if let Some(parent) = record["parent"].as_str().filter(|parent| crate::is_safe_id(parent)) {
        read.side = Some(parent.to_owned());
        read.marks.clear();
    }
    read
}

/// Sessions Silt recorded itself; they continue in the web UI, or in any
/// agent CLI through translation.
pub struct Acp;

impl Agent for Acp {
    fn id(&self) -> &'static str {
        AGENT
    }

    fn command(&self) -> &'static str {
        AGENT
    }

    fn display_name(&self) -> &'static str {
        "an agent from the web UI"
    }

    fn native(&self) -> NativeConventions {
        NativeConventions { rules: &[], skill_dirs: &[], personal_skills: false, given_skills: false }
    }

    fn reader(&self) -> Box<dyn Reader> {
        Box::new(Stateless(read_line))
    }

    fn session_ids(&self) -> SessionIds {
        SessionIds::Chosen
    }

    fn transcript(&self, _cwd: &Path, _id: &str) -> Option<PathBuf> {
        None
    }

    fn discover(&self, _cwd: &Path, _since_ms: i64, _known: &[String]) -> Option<Found> {
        None
    }

    fn restore(&self, _root: &Path, _cwd: &Path, _session: &CheckedSession) -> Result<PathBuf, AgentError> {
        Err(AgentError::AcpSession)
    }

    fn launch(&self, _plan: &LaunchPlan) -> Result<Launch, AgentError> {
        Err(AgentError::AcpSession)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_read_back_as_a_conversation() {
        let lines = [
            session("fake", "Fake agent", "s1"),
            prompt("add acp", "alice", &[], &[]),
            thought("planning", 1_767_323_045_250),
            tool_call("c1", "Edit src/main.rs", "edit", Some(&json!({"path": "src/main.rs"}))),
            tool_result("c1", "completed", Some("edited")),
            message("Done"),
            usage("u1", 1600, 240),
            stop("end_turn"),
        ];
        let reads: Vec<Read> = lines.iter().map(|line| read_line(line)).collect();
        let entries: Vec<Entry> =
            reads.iter().flat_map(|read| read.entries.clone()).filter(|entry| !matches!(entry, Entry::Other { .. })).collect();
        assert_eq!(entries[0], Entry::User { text: "add acp".into() });
        assert_eq!(entries[1], Entry::Thinking { text: "planning".into() });
        assert_eq!(entries[4], Entry::Assistant { text: "Done".into() });
        assert_eq!(entries[5], Entry::Usage { message: "u1".into(), input: 1600, output: 240 });
        let marks: Vec<Mark> = reads.into_iter().flat_map(|read| read.marks).collect();
        assert_eq!(marks.first(), Some(&Mark::PromptSubmitted));
        assert_eq!(marks.last(), Some(&Mark::Stopped));
        assert_eq!(session_of(&lines[0]), Some(("fake".into(), "s1".into())));
        assert_eq!(session_of(&lines[1]), None);
        assert_eq!(crate::time::line_time(&lines[2]), Some(1_767_323_045_250), "a thought keeps when it began");
    }

    #[test]
    fn a_call_s_update_names_it_without_starting_it_again_and_output_loses_its_fence() {
        let update =
            read_line(&tool_update("c1", "`python3 -m unittest`", "execute", Some(&json!({"command": "python3 -m unittest"}))));
        assert!(update.marks.is_empty(), "the call started when it was announced");
        assert!(
            matches!(&update.entries[..], [Entry::ToolCall { id, name, .. }] if id == "c1" && name == "`python3 -m unittest`")
        );
        let text = |output: &str| match &read_line(&tool_result("c1", "completed", Some(output))).entries[..] {
            [Entry::ToolResult { text, .. }] => text.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(text("```console\nRan 10 tests\n\nOK\n```"), "Ran 10 tests\n\nOK");
        assert_eq!(text("```\nExit code 2\n```"), "Exit code 2");
        assert_eq!(text("plain output"), "plain output");
        assert_eq!(
            text(r#""File created at /x.md\n\nDone""#),
            "File created at /x.md\n\nDone",
            "a JSON string reads as its text"
        );
        assert_eq!(text("42"), "42");
        assert_eq!(text("```a\n1\n```\nthen\n```b\n2\n```"), "```a\n1\n```\nthen\n```b\n2\n```", "two blocks keep theirs");
    }

    #[test]
    fn a_turn_the_agent_opened_itself_is_a_turn_without_a_prompt() {
        let named = read_line(&wake(Some("`sleep 8` finished in the background"), false));
        assert_eq!(
            named.entries,
            [Entry::Woke { text: "`sleep 8` finished in the background, and the agent went on by itself.".into() }]
        );
        let stopped = read_line(&wake(None, true));
        assert_eq!(stopped.entries, [Entry::Woke { text: format!("{WOKE} Silt stopped it, as you had stopped the agent.") }]);
        let read = read_line(&wake(None, false));
        assert_eq!(read.marks, [Mark::PromptSubmitted]);
        assert_eq!(read.entries, [Entry::Woke { text: WOKE.into() }]);
        assert_eq!(read.entries[0].kind(), "wake");
    }

    #[test]
    fn a_prompt_names_its_attachments_and_keeps_its_images() {
        let read = read_line(&prompt("look", "alice", &["prompt-1/shot.png".into()], &[("image/png".into(), "AAAA".into())]));
        assert_eq!(
            read.entries,
            [
                Entry::User { text: "look\n\n(attached: prompt-1/shot.png)".into() },
                Entry::Image { media_type: "image/png".into(), data: "AAAA".into() }
            ]
        );
    }

    #[test]
    fn a_subagent_s_records_are_its_own_and_task_news_ties_it_to_its_call() {
        let inside = read_line(&within(&tool_call("t2", "Read", "read", None), "t1"));
        assert_eq!((inside.side.as_deref(), inside.marks.len()), (Some("t1"), 0));
        assert_eq!(read_line(&message("mine")).side, None);
        assert_eq!(
            read_line(&task("t1", Some("ad7"), Some("subagent"), "running")).entries,
            vec![Entry::Task(Task {
                id: "ad7".into(),
                call: Some("t1".into()),
                kind: Some(TaskKind::Subagent),
                status: TaskStatus::Running,
                side: Some("t1".into()),
                summary: None,
            })]
        );
        let ended = read_line(&task("t3", None, None, "failed"));
        assert!(matches!(&ended.entries[0], Entry::Task(Task { id, status: TaskStatus::Failed, side: None, .. }) if id == "t3"));
    }
}
