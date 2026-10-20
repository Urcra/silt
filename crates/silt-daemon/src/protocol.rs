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
    /// Starts a fork of a thread on this machine (ADR 0010).
    Fork { thread: String, cwd: PathBuf, here: bool },
    /// Merges a fork's source into it; `thread` defaults to the one checked
    /// out at `cwd`.
    Pull { thread: Option<String>, cwd: PathBuf },
    MrCreate {
        thread: Option<String>,
        cwd: PathBuf,
        title: Option<String>,
        summary: Option<String>,
    },
    MrList { thread: Option<String> },
    MrShow { mr: String },
    MrApprove { mr: String },
    MrDecline { mr: String, reason: Option<String> },
    /// Sets the visibility of the project at `cwd`, or of `thread`'s family.
    SetVisibility {
        cwd: PathBuf,
        thread: Option<String>,
        visibility: silt_proto::Visibility,
    },
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
    /// Where the history moves to another thread's part, or back.
    Segment { label: String },
    /// A pull or a merge.
    Event { text: String },
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
    /// Threads whose history here differs from the server's, so they no
    /// longer push.
    pub diverged: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resumed {
    pub thread: String,
    pub checkout: PathBuf,
    pub agent_cwd: PathBuf,
    /// The agent session to continue, when the thread has one.
    pub session_id: Option<String>,
}

/// What a hook should hand back to the agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookReply {
    /// Context for the agent's next turn, such as work merged into its thread.
    pub additional_context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pulled {
    pub thread: String,
    pub from: String,
    /// Conversation entries the pull brought in.
    pub messages: usize,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MrSummary {
    pub id: String,
    pub fork: String,
    pub target: String,
    pub title: String,
    pub summary: String,
    /// `open`, `approved` or `declined: <reason>`.
    pub status: String,
    pub opened_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MrDetails {
    pub mr: MrSummary,
    /// The fork's changes since the merge base.
    pub diff: String,
    /// Places both the fork and the target changed since the merge base.
    pub conflicts: Vec<String>,
    /// The fork's conversation since it forked.
    pub conversation: Vec<ShowLine>,
}
