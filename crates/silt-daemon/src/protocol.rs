use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One request per line on the daemon socket, answered by one response line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    Ping,
    /// Starts recording a new agent session in a new thread. Without a
    /// `session_id` the daemon finds the session once the agent writes it.
    StartSession {
        cwd: PathBuf,
        agent: String,
        session_id: Option<String>,
        isolated: bool,
        /// The session already ran, so the clone's uncommitted changes may
        /// be its edits rather than where it started.
        #[serde(default)]
        existing: bool,
    },
    /// An agent hook fired; the input is what the agent passed to the hook.
    Hook { event: String, input: serde_json::Value },
    /// The agent running in `thread` exited: record the rest of its session.
    EndSession { thread: String },
    Threads,
    Show { thread: String },
    /// What the thread changed: one `turn`, or everything since `base`
    /// (`merge-base`, `last-commit`, `last-turn`, `commit:<sha>`), by
    /// default since it started.
    Diff {
        thread: String,
        turn: Option<usize>,
        #[serde(default)]
        base: Option<String>,
    },
    /// Searches the server when connected (with `filters`), else this
    /// machine's index.
    Search {
        query: String,
        limit: u32,
        #[serde(default)]
        filters: SearchFilters,
    },
    /// Re-reads the server settings after `silt auth login`.
    Reload,
    Status,
    /// Continues a thread on this machine (ADR 0009), with its own agent or
    /// with `agent` (ADR 0012).
    Resume {
        thread: String,
        cwd: PathBuf,
        here: bool,
        #[serde(default)]
        agent: Option<String>,
    },
    /// Starts a fork of a thread on this machine (ADR 0010).
    Fork {
        thread: String,
        cwd: PathBuf,
        here: bool,
        #[serde(default)]
        agent: Option<String>,
    },
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
    /// A page of a thread's full history, across compactions and forks.
    HistoryRead { thread: String, offset: usize, limit: usize },
    /// Search within a thread's history and the threads it came from.
    HistorySearch { thread: String, query: String, limit: u32 },
    /// The thread's turns, for showing each turn's diff next to its prompt.
    Turns { thread: String },
    /// Starts the web UI if needed; answers with its URL.
    OpenWeb {
        /// The clone `silt open` ran in; its repository's threads can then
        /// be opened from the web UI even when they started elsewhere.
        #[serde(default)]
        cwd: Option<PathBuf>,
    },
    /// The turns that changed a file of the thread.
    FileHistory { thread: String, path: String },
    /// Other threads of the repository at `cwd` that changed `path`.
    InFlight { cwd: PathBuf, path: String },
    /// Comments on a history entry or on lines of a file (ADR 0011).
    /// `cwd` is a clone to read the thread's files from if needed.
    Comment {
        thread: String,
        on: CommentOn,
        text: String,
        cwd: PathBuf,
    },
    Comments { thread: String, cwd: PathBuf },
    ResolveComment { thread: String, comment: String, cwd: PathBuf },
    /// Sets the visibility of the project at `cwd`, or of `thread`'s family.
    SetVisibility {
        cwd: PathBuf,
        thread: Option<String>,
        visibility: silt_proto::Visibility,
    },
    /// The server's notifications for this user (ADR 0013).
    Inbox {
        #[serde(default)]
        all: bool,
    },
    InboxRead { read: silt_proto::InboxRead },
    /// Moves a thread to the trash, everywhere (ADR 0013).
    ThreadDelete { thread: String },
    ThreadRestore { thread: String },
    /// Removes a thread in the trash for good, everywhere.
    ThreadPurge { thread: String },
    /// The threads in the trash.
    Trash,
    /// This machine's viewer of a thread: what file is open and the draft
    /// prompt; answers with everyone else on it (ADR 0014).
    Presence {
        thread: String,
        #[serde(default)]
        file: Option<String>,
        #[serde(default)]
        draft: String,
        #[serde(default)]
        leaving: bool,
    },
    /// The family's deltas after the first `after`, for the browser's
    /// replica (ADR 0014).
    Replica { thread: String, after: usize },
    /// A git blob's text, for the browser's replica.
    GitText { thread: String, oid: String },
    /// Edits made in the browser's replica, postcard and base64 encoded.
    BrowserDeltas { thread: String, deltas: String },
    /// A fingerprint of the thread's live state.
    Live { thread: String },
    /// Lets `user` write into the thread, or stops it (ADR 0014).
    ThreadShare { thread: String, user: String, remove: bool },
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
    /// The session's id, when the daemon chose it.
    pub session_id: Option<String>,
    pub checkout: PathBuf,
    /// Where to start the agent so its relative position in the repo is kept.
    pub agent_cwd: PathBuf,
    pub mount: Option<Mount>,
}

/// A managed checkout that was just mounted, for the CLI to link shared
/// paths into and run `prepare` in, where the user sees the output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    /// The clone the checkout was made from.
    pub source: PathBuf,
    /// Where this project's linked paths live on this machine.
    pub shared: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: String,
    pub repo: String,
    pub title: Option<String>,
    /// The command of the agent that recorded its latest session.
    #[serde(default)]
    pub agent: Option<String>,
    pub created_ms: i64,
    pub messages: usize,
    pub edits: usize,
    /// `here` when checked out on this machine, `remote` when only known
    /// from the server, `moved` when another device took it over.
    pub location: String,
    pub forked_from: Option<String>,
    /// The owner's user name; empty when this machine isn't on a server.
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub collaborators: Vec<String>,
    /// Whether this machine's user may write into it (ADR 0014).
    #[serde(default)]
    pub writable: bool,
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
    /// Set when the server answered.
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
}

/// Server search filters (ADR 0011); dates are milliseconds since the epoch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFilters {
    pub repo: Option<String>,
    pub author: Option<String>,
    pub status: Option<String>,
    pub branch: Option<String>,
    pub file: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}

impl SearchFilters {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
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
    /// Recovery forks made since the daemon started.
    #[serde(default)]
    pub recovered: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resumed {
    pub thread: String,
    pub checkout: PathBuf,
    pub agent_cwd: PathBuf,
    /// The agent session to continue, when the thread has one.
    pub session_id: Option<String>,
    pub mount: Option<Mount>,
    /// The agent to run and how to start it.
    pub agent: String,
    pub start: Option<AgentStart>,
}

/// How the CLI starts the agent on a resumed or forked thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentStart {
    /// The agent's own session, restored natively or translated.
    Resume { id: String },
    /// A new session whose first prompt carries the conversation.
    Primed { id: Option<String>, prompt: String },
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub index: usize,
    /// The thread the entry was recorded in.
    pub thread: String,
    /// `own`, `inherited` (before the fork) or `upstream` (brought in by a pull).
    pub origin: String,
    /// `user`, `assistant`, `thinking`, `tool_call`, `tool_result`,
    /// `compaction`, `merge` or `pull`.
    pub kind: String,
    pub text: String,
    pub time_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPage {
    pub thread: String,
    pub total: usize,
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryHit {
    /// The entry's index in the thread's history.
    pub index: usize,
    pub kind: String,
    pub snippet: String,
}

/// Edits to one file in one turn, by one tool call or outside tool calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTouch {
    pub thread: String,
    /// The prompt of the turn the edits were made in.
    pub prompt: Option<String>,
    pub tool_use_id: Option<String>,
    pub by_agent: bool,
    /// Characters inserted and deleted.
    pub inserted: usize,
    pub deleted: usize,
    pub time_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFlight {
    pub thread: String,
    pub owner: String,
    pub files: Vec<String>,
    pub last_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case")]
pub enum CommentOn {
    /// A history entry, by its index.
    Entry { index: usize },
    /// Lines `start..=end` (1-based) of a file as the thread has it now.
    Lines { path: String, start: usize, end: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum CommentLocation {
    Entry { index: usize, excerpt: String },
    /// Where the commented lines are now; `outdated` once they're deleted.
    Lines { path: String, start: usize, end: usize, outdated: bool },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentView {
    pub id: String,
    pub by: String,
    pub time_ms: i64,
    pub text: String,
    pub resolved: bool,
    pub location: CommentLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnInfo {
    /// 1-based, as `silt diff --turn` counts them.
    pub number: usize,
    /// The history entry of the turn's prompt.
    pub entry: Option<usize>,
    pub prompt: String,
}

/// A notification, with the thread id as text like everywhere in this API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxEntry {
    pub id: i64,
    pub kind: String,
    pub thread: String,
    pub actor: String,
    pub text: String,
    pub created_ms: i64,
    pub read: bool,
}

/// Deltas for the browser's replica (ADR 0014).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaDeltas {
    /// How many deltas the family has here; ask for those after next time.
    pub total: usize,
    /// postcard-encoded `Vec<Delta>`, in base64.
    pub deltas: String,
    /// Whether this machine's user may edit the thread.
    pub writable: bool,
}

/// What the web UI watches for changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveState {
    pub deltas: usize,
}
