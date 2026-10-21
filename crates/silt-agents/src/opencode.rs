//! OpenCode: sessions live in its SQLite database, so Silt reads them
//! through `opencode export <id>` and writes them back with
//! `opencode import` (docs/research/agents.md). Each finished message and
//! part becomes one record: `{"session": …}`, `{"message": …}` or
//! `{"part": …}`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::conventions::NativeConventions;
use crate::{Agent, Entry, Found, Launch, LaunchPlan, Mark, Read, Reader, RecordedSession, Rewrites, SessionIds, Start};

pub const AGENT: &str = "opencode";

/// Runs `opencode` and returns its output. The output goes through a file:
/// OpenCode cuts its output short when writing large amounts to a pipe. It
/// holds a conversation, so the file is private to this user, unique to
/// this call, and never one someone planted.
fn opencode(args: &[&str], cwd: Option<&Path>) -> io::Result<Vec<u8>> {
    let launch = Launch::new("opencode");
    let output = tempfile::Builder::new().prefix("silt-opencode-").suffix(".out").tempfile()?;
    let mut command = Command::new(&launch.program);
    command.args(args).stdin(Stdio::null()).stdout(output.reopen()?).stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let result = command.output();
    let stdout = fs::read(output.path());
    let result = result?;
    if result.status.success() {
        stdout
    } else {
        Err(io::Error::other(format!(
            "opencode {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&result.stderr).trim()
        )))
    }
}

/// The records of an export that are worth keeping: the session, every
/// message, and the parts that are finished.
pub fn export_records(export: &Value) -> Vec<String> {
    let mut records = Vec::new();
    if let Some(info) = export.get("info") {
        records.push(json!({ "session": info }).to_string());
    }
    for message in export.get("messages").and_then(Value::as_array).into_iter().flatten() {
        if let Some(info) = message.get("info") {
            records.push(json!({ "message": info }).to_string());
        }
        for part in message.get("parts").and_then(Value::as_array).into_iter().flatten() {
            if part_finished(part) {
                records.push(json!({ "part": part }).to_string());
            }
        }
    }
    records
}

fn part_finished(part: &Value) -> bool {
    match part.get("type").and_then(Value::as_str) {
        Some("text" | "reasoning") => {
            part.pointer("/time/end").is_some() || part.get("time").is_none() || part.get("synthetic").is_some()
        }
        Some("tool") => matches!(part.pointer("/state/status").and_then(Value::as_str), Some("completed" | "error")),
        _ => true,
    }
}

/// What makes a record distinct: one session and message record per id,
/// and each part once.
pub fn key(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    let (kind, object) = value.as_object()?.iter().next()?;
    match kind.as_str() {
        "session" => Some("session".into()),
        "message" | "part" => Some(format!("{kind}:{}", object.get("id")?.as_str()?)),
        _ => None,
    }
}

/// Reads records, remembering each message's role for its parts.
#[derive(Default)]
pub struct OpenCodeReader {
    roles: HashMap<String, String>,
}

impl Reader for OpenCodeReader {
    fn push(&mut self, line: &str) -> Read {
        let mut read = Read::default();
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            read.entries.push(Entry::Other { kind: "unparseable".into() });
            return read;
        };
        if let Some(message) = value.get("message") {
            let role = message.get("role").and_then(Value::as_str).unwrap_or_default().to_owned();
            if role == "user" {
                read.marks.push(Mark::PromptSubmitted);
            }
            if let Some(id) = message.get("id").and_then(Value::as_str) {
                self.roles.insert(id.to_owned(), role);
            }
            read.entries.push(Entry::Other { kind: "message".into() });
            return read;
        }
        let Some(part) = value.get("part") else {
            read.entries.push(Entry::Other { kind: "session".into() });
            return read;
        };
        let field = |key: &str| part.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let role = part
            .get("messageID")
            .and_then(Value::as_str)
            .and_then(|id| self.roles.get(id))
            .map_or("assistant", String::as_str);
        match field("type").as_str() {
            "text" if part.get("synthetic").and_then(Value::as_bool) == Some(true) => {
                read.entries.push(Entry::Other { kind: "synthetic".into() });
            }
            "text" if role == "user" => read.entries.push(Entry::User { text: field("text") }),
            "text" => read.entries.push(Entry::Assistant { text: field("text") }),
            "reasoning" => read.entries.push(Entry::Thinking { text: field("text") }),
            "tool" => {
                let id = field("callID");
                let state = part.get("state").cloned().unwrap_or(Value::Null);
                let output = state
                    .get("output")
                    .or_else(|| state.get("error"))
                    .map(|value| value.as_str().map_or_else(|| value.to_string(), str::to_owned))
                    .unwrap_or_default();
                let tool = field("tool");
                read.marks.push(Mark::ToolStarted {
                    tool_use_id: id.clone(),
                    tool: tool.clone(),
                });
                read.marks.push(Mark::ToolFinished { tool_use_id: id.clone() });
                read.entries.push(Entry::ToolCall {
                    id: id.clone(),
                    name: tool,
                    input: state.get("input").cloned().unwrap_or(Value::Null),
                });
                read.entries.push(Entry::ToolResult {
                    tool_use_id: id,
                    text: output,
                    is_error: state.get("status").and_then(Value::as_str) == Some("error"),
                });
            }
            "compaction" => read.entries.push(Entry::Compaction { summary: None }),
            "step-finish" if field("reason") == "stop" => {
                read.marks.push(Mark::Stopped);
                read.entries.push(Entry::Other { kind: "step-finish".into() });
            }
            other => read.entries.push(Entry::Other { kind: other.to_owned() }),
        }
        read
    }
}

/// Rebuilds an export from records, in the order they were recorded.
pub fn export_from_records(lines: &[String]) -> Value {
    let mut info = Value::Null;
    let mut order: Vec<String> = Vec::new();
    let mut messages: HashMap<String, Value> = HashMap::new();
    let mut parts: HashMap<String, Vec<Value>> = HashMap::new();
    let mut seen_parts: HashSet<String> = HashSet::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(session) = value.get("session") {
            info = session.clone();
        } else if let Some(message) = value.get("message") {
            let id = message.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
            if !messages.contains_key(&id) {
                order.push(id.clone());
            }
            messages.insert(id, message.clone());
        } else if let Some(part) = value.get("part") {
            let id = part.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
            let message = part.get("messageID").and_then(Value::as_str).unwrap_or_default().to_owned();
            if seen_parts.insert(id) {
                parts.entry(message).or_default().push(part.clone());
            }
        }
    }
    let messages: Vec<Value> = order
        .into_iter()
        .map(|id| json!({"info": messages[&id], "parts": parts.remove(&id).unwrap_or_default()}))
        .collect();
    json!({"info": info, "messages": messages})
}

const ID_TAIL: usize = 14;
const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// OpenCode ids are a prefix, twelve hex digits of time and fourteen random
/// characters. A fork keeps the time, so ordering holds, and gets a new
/// random tail derived from the fork's session id.
fn forked_id(id: &str, salt: &str) -> String {
    let keep = id.len().saturating_sub(ID_TAIL);
    // Ids come from recorded lines, so possibly not ASCII.
    let keep = (0..=keep).rev().find(|at| id.is_char_boundary(*at)).unwrap_or(0);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut tail = String::new();
    for byte in id.bytes().chain(salt.bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    for _ in 0..ID_TAIL {
        hash = (hash ^ (hash >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        tail.push(char::from(BASE62[usize::try_from(hash % 62).expect("small")]));
    }
    format!("{}{tail}", &id[..keep])
}

/// Message and part ids in the records, to give a fork fresh ones: the
/// database keys them globally.
fn record_ids(lines: &[String]) -> Vec<String> {
    let mut ids = Vec::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        for pointer in ["/message/id", "/part/id"] {
            if let Some(id) = value.pointer(pointer).and_then(Value::as_str) {
                ids.push(id.to_owned());
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

/// The OpenCode adapter.
pub struct OpenCode;

impl Agent for OpenCode {
    fn id(&self) -> &'static str {
        AGENT
    }

    fn command(&self) -> &'static str {
        "opencode"
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn native(&self) -> NativeConventions {
        NativeConventions {
            rules: &["AGENTS.md", "CLAUDE.md"],
            skill_dirs: &[".opencode/skills", ".claude/skills", ".agents/skills"],
            personal_skills: false,
            given_skills: false,
        }
    }

    fn reader(&self) -> Box<dyn Reader> {
        Box::<OpenCodeReader>::default()
    }

    fn session_ids(&self) -> SessionIds {
        SessionIds::Discovered
    }

    fn transcript(&self, _cwd: &Path, id: &str) -> Option<PathBuf> {
        Some(PathBuf::from(format!("opencode:{id}")))
    }

    fn discover(&self, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
        let listed = opencode(&["session", "list", "--format", "json"], Some(cwd)).ok()?;
        let start = listed.iter().position(|byte| *byte == b'[')?;
        let sessions: Vec<Value> = serde_json::from_slice(&listed[start..]).ok()?;
        let cwd = cwd.to_string_lossy();
        sessions
            .into_iter()
            .filter(|session| session.get("directory").and_then(Value::as_str) == Some(&*cwd))
            .filter(|session| session.get("created").and_then(Value::as_i64).is_some_and(|created| created >= since_ms))
            .filter_map(|session| session.get("id").and_then(Value::as_str).map(str::to_owned))
            .find(|id| !known.contains(id))
            .map(|id| Found {
                transcript: PathBuf::from(format!("opencode:{id}")),
                id,
            })
    }

    fn reads_by_snapshot(&self) -> bool {
        true
    }

    fn snapshot(&self, found: &Found) -> io::Result<Vec<String>> {
        let bytes = opencode(&["export", &found.id], None)?;
        let start = bytes.iter().position(|byte| *byte == b'{').unwrap_or(0);
        let export: Value = serde_json::from_slice(&bytes[start..]).map_err(io::Error::other)?;
        Ok(export_records(&export))
    }

    fn record_key(&self, line: &str) -> Option<String> {
        key(line)
    }

    fn fork_id(&self, from: &str) -> String {
        forked_id(from, &crate::now_ms().to_string())
    }

    fn restore(&self, root: &Path, cwd: &Path, session: &RecordedSession) -> io::Result<PathBuf> {
        let mut rewrites = Rewrites::new(session, None, &cwd.to_string_lossy(), &root.to_string_lossy());
        if !session.former_ids.contains(&session.id) {
            for id in record_ids(&session.main) {
                let new = forked_id(&id, &session.id);
                rewrites = rewrites.with(&id, &new);
            }
        }
        let lines: Vec<String> = session.main.iter().map(|line| rewrites.apply(line)).collect();
        let export = export_from_records(&lines);
        let mut file = tempfile::Builder::new().prefix("silt-opencode-").suffix(".json").tempfile()?;
        io::Write::write_all(&mut file, export.to_string().as_bytes())?;
        opencode(&["import", &file.path().to_string_lossy()], Some(cwd))?;
        Ok(PathBuf::from(format!("opencode:{}", session.id)))
    }

    fn launch(&self, plan: &LaunchPlan) -> io::Result<Launch> {
        let mut launch = Launch::new(self.command());
        match &plan.start {
            Start::Resume { id } => {
                launch.arg("--session").arg(id);
            }
            Start::Primed { prompt, .. } => {
                launch.arg("--prompt").arg(prompt);
            }
            Start::New { .. } => {}
        }
        let mut command = vec![plan.mcp.command.to_string_lossy().into_owned()];
        command.extend(plan.mcp.args.iter().cloned());
        let mut config = json!({"mcp": {"silt": {"type": "local", "command": command, "enabled": true}}});
        if let Some(instructions) = &plan.instructions {
            fs::create_dir_all(&plan.scratch)?;
            let file = plan.scratch.join("opencode-instructions.md");
            fs::write(&file, instructions)?;
            config["instructions"] = json!([file]);
        }
        launch.env("OPENCODE_CONFIG_CONTENT", config.to_string());
        for arg in &plan.args {
            launch.arg(arg);
        }
        Ok(launch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export() -> Value {
        json!({
            "info": {"id": "ses_3b6b63d04ffee8Yin4du7J6fyU", "directory": "/w/repo", "title": "t"},
            "messages": [
                {"info": {"id": "msg_c4949c2fd001MQcUSXA3sXpVlA", "role": "user", "sessionID": "ses_3b6b63d04ffee8Yin4du7J6fyU"},
                 "parts": [{"id": "prt_c4949c2fd0026rVbItfVNFab", "messageID": "msg_c4949c2fd001MQcUSXA3sXpVlA", "type": "text", "text": "List files"}]},
                {"info": {"id": "msg_c4949c30b0015PXEwqN1S2BvDp", "role": "assistant", "sessionID": "ses_3b6b63d04ffee8Yin4du7J6fyU", "path": {"cwd": "/w/repo"}},
                 "parts": [
                    {"id": "prt_a", "messageID": "msg_c4949c30b0015PXEwqN1S2BvDp", "type": "step-start"},
                    {"id": "prt_b", "messageID": "msg_c4949c30b0015PXEwqN1S2BvDp", "type": "tool", "callID": "c1", "tool": "glob", "state": {"status": "completed", "input": {"pattern": "*"}, "output": "/w/repo/a.md"}},
                    {"id": "prt_c", "messageID": "msg_c4949c30b0015PXEwqN1S2BvDp", "type": "tool", "callID": "c2", "tool": "read", "state": {"status": "running", "input": {}}},
                    {"id": "prt_d", "messageID": "msg_c4949c30b0015PXEwqN1S2BvDp", "type": "text", "text": "One file.", "time": {"start": 1, "end": 2}},
                    {"id": "prt_e", "messageID": "msg_c4949c30b0015PXEwqN1S2BvDp", "type": "step-finish", "reason": "stop"}
                 ]}
            ]
        })
    }

    #[test]
    fn records_skip_unfinished_parts_and_read_as_a_conversation() {
        let records = export_records(&export());
        assert_eq!(records.len(), 8, "the running tool call waits");
        assert_eq!(key(&records[0]).as_deref(), Some("session"));
        assert_eq!(key(&records[2]).as_deref(), Some("part:prt_c4949c2fd0026rVbItfVNFab"));
        let mut reader = OpenCodeReader::default();
        let reads: Vec<Read> = records.iter().map(|line| reader.push(line)).collect();
        let entries: Vec<Entry> = reads
            .iter()
            .flat_map(|read| read.entries.clone())
            .filter(|entry| !matches!(entry, Entry::Other { .. }))
            .collect();
        assert_eq!(entries[0], Entry::User { text: "List files".into() });
        assert!(matches!(&entries[1], Entry::ToolCall { name, .. } if name == "glob"));
        assert_eq!(entries[3], Entry::Assistant { text: "One file.".into() });
        assert!(reads.last().expect("reads").marks.contains(&Mark::Stopped));
    }

    #[test]
    fn records_rebuild_the_export() {
        let rebuilt = export_from_records(&export_records(&export()));
        assert_eq!(rebuilt["info"]["id"], "ses_3b6b63d04ffee8Yin4du7J6fyU");
        assert_eq!(rebuilt["messages"].as_array().expect("messages").len(), 2);
        assert_eq!(rebuilt["messages"][1]["parts"].as_array().expect("parts").len(), 4);
    }

    #[test]
    fn forked_ids_keep_their_time_and_shape() {
        let id = "msg_c4949c2fd001MQcUSXA3sXpVlA";
        let fork = forked_id(id, "ses_new");
        assert_eq!(fork.len(), id.len());
        assert!(fork.starts_with("msg_c4949c2fd001"));
        assert_ne!(fork, id);
        assert_eq!(fork, forked_id(id, "ses_new"));
    }
}
