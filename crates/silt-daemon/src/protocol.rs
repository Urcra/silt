use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One request per line on the daemon socket, answered by one response line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    Ping,
    /// Starts recording a new agent session in a new thread.
    StartSession {
        cwd: PathBuf,
        agent: String,
        session_id: String,
        isolated: bool,
    },
    /// An agent hook fired; the input is what the agent passed to the hook.
    Hook { event: String, input: serde_json::Value },
    EndSession { agent: String, session_id: String },
    Threads,
    Show { thread: String },
    Diff { thread: String, turn: Option<usize> },
    Search { query: String, limit: u32 },
    /// Re-reads the server settings after `silt auth login`.
    Reload,
    Status,
    /// Continues a thread on this machine (ADR 0009).
    Resume { thread: String, cwd: PathBuf, here: bool },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok { data: serde_json::Value },
    Error { message: String },
}

impl Response {
    pub fn ok(data: impl Serialize) -> Self {
        match serde_json::to_value(data) {
            Ok(data) => Self::Ok { data },
            Err(error) => Self::error(error),
        }
    }

    pub fn error(message: impl std::fmt::Display) -> Self {
        Self::Error {
            message: message.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStarted {
    pub thread: String,
    pub checkout: PathBuf,
    /// Where to start the agent so its relative position in the repo is kept.
    pub agent_cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: String,
    pub repo: String,
    pub title: Option<String>,
    pub created_ms: i64,
    pub messages: usize,
    pub edits: usize,
    /// `here` when checked out on this machine, `remote` when only known
    /// from the server, `moved` when another device took it over.
    pub location: String,
    pub forked_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "line", rename_all = "snake_case")]
pub enum ShowLine {
    Entry { kind: String, text: String },
    ToolCall { id: String, name: String, input: String },
    Edits { tool_use_id: Option<String>, files: Vec<FileEdit> },
    Compaction { summary: Option<String> },
    Turn { mark: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEdit {
    pub path: String,
    pub inserted: usize,
    pub deleted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub thread: String,
    pub kind: String,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub server: Option<String>,
    pub device: Option<String>,
    pub sync_error: Option<String>,
    pub threads: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resumed {
    pub thread: String,
    pub checkout: PathBuf,
    pub agent_cwd: PathBuf,
    /// The agent session to continue, when the thread has one.
    pub session_id: Option<String>,
}
