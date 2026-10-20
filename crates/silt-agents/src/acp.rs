//! Sessions of agents the web UI starts over ACP (ADR 0014). Silt writes
//! these records itself as the agent works, one JSON object per line:
//! `session`, `prompt`, `message`, `thought`, `tool_call`, `tool_result`,
//! `plan` and `stop`.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::conventions::NativeConventions;
use crate::{Agent, Entry, Found, Launch, LaunchPlan, Mark, Read, Reader, RecordedSession, SessionIds, Stateless};

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

pub fn prompt(text: &str, by: &str) -> String {
    json!({"t": "prompt", "text": text, "by": by}).to_string()
}

pub fn message(text: &str) -> String {
    json!({"t": "message", "text": text}).to_string()
}

pub fn thought(text: &str) -> String {
    json!({"t": "thought", "text": text}).to_string()
}

pub fn tool_call(id: &str, title: &str, kind: &str, input: Option<&Value>) -> String {
    json!({"t": "tool_call", "id": id, "title": title, "kind": kind, "input": input}).to_string()
}

pub fn tool_result(id: &str, status: &str, output: Option<&str>) -> String {
    json!({"t": "tool_result", "id": id, "status": status, "output": output}).to_string()
}

pub fn plan(entries: &[String]) -> String {
    json!({"t": "plan", "entries": entries}).to_string()
}

pub fn stop(reason: &str) -> String {
    json!({"t": "stop", "reason": reason}).to_string()
}

pub fn read_line(line: &str) -> Read {
    let mut read = Read::default();
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        read.entries.push(Entry::Other { kind: "unparseable".into() });
        return read;
    };
    let field = |key: &str| record[key].as_str().unwrap_or_default().to_owned();
    match record["t"].as_str().unwrap_or_default() {
        "prompt" => {
            read.marks.push(Mark::PromptSubmitted);
            read.entries.push(Entry::User { text: field("text") });
        }
        "message" => read.entries.push(Entry::Assistant { text: field("text") }),
        "thought" => read.entries.push(Entry::Thinking { text: field("text") }),
        "tool_call" => {
            read.marks.push(Mark::ToolStarted {
                tool_use_id: field("id"),
                tool: field("title"),
            });
            read.entries.push(Entry::ToolCall {
                id: field("id"),
                name: field("title"),
                input: record["input"].clone(),
            });
        }
        "tool_result" => {
            read.marks.push(Mark::ToolFinished { tool_use_id: field("id") });
            read.entries.push(Entry::ToolResult {
                tool_use_id: field("id"),
                text: field("output"),
                is_error: field("status") == "failed",
            });
        }
        "plan" => read.entries.push(Entry::Other { kind: "plan".into() }),
        "stop" => {
            read.marks.push(Mark::Stopped);
            read.entries.push(Entry::Other { kind: "stop".into() });
        }
        other => read.entries.push(Entry::Other { kind: other.to_owned() }),
    }
    read
}

/// Sessions Silt recorded itself; they continue in the web UI, or in any
/// agent CLI through translation.
pub struct Acp;

fn unsupported() -> io::Error {
    io::Error::other("ACP sessions continue from the web UI; resume the thread with `--agent` to use an agent CLI")
}

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
        NativeConventions {
            rules: &[],
            skill_dirs: &[],
            personal_skills: false,
            given_skills: false,
        }
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

    fn restore(&self, _root: &Path, _cwd: &Path, _session: &RecordedSession) -> io::Result<PathBuf> {
        Err(unsupported())
    }

    fn launch(&self, _plan: &LaunchPlan) -> io::Result<Launch> {
        Err(unsupported())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_read_back_as_a_conversation() {
        let lines = [
            session("fake", "Fake agent", "s1"),
            prompt("add acp", "alice"),
            thought("planning"),
            tool_call("c1", "Edit src/main.rs", "edit", Some(&json!({"path": "src/main.rs"}))),
            tool_result("c1", "completed", Some("edited")),
            message("Done"),
            stop("end_turn"),
        ];
        let reads: Vec<Read> = lines.iter().map(|line| read_line(line)).collect();
        let entries: Vec<Entry> = reads.iter().flat_map(|read| read.entries.clone()).filter(|entry| !matches!(entry, Entry::Other { .. })).collect();
        assert_eq!(entries[0], Entry::User { text: "add acp".into() });
        assert_eq!(entries[1], Entry::Thinking { text: "planning".into() });
        assert_eq!(entries[4], Entry::Assistant { text: "Done".into() });
        let marks: Vec<Mark> = reads.into_iter().flat_map(|read| read.marks).collect();
        assert_eq!(marks.first(), Some(&Mark::PromptSubmitted));
        assert_eq!(marks.last(), Some(&Mark::Stopped));
        assert_eq!(session_of(&lines[0]), Some(("fake".into(), "s1".into())));
        assert_eq!(session_of(&lines[1]), None);
    }
}
