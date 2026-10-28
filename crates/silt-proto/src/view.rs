//! What the web UI reads, as both a daemon and the hosting server answer it
//! (ADR 0011, ADR 0026), so the two can't drift apart.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{AgentState, Capabilities, InboxAction, Snooze, Startable, ThreadStatus, Visibility};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// How the agent the web UI last ran in it here stands (ADR 0022).
    #[serde(default)]
    pub agent_state: Option<AgentState>,
    /// When something last happened in it.
    #[serde(default)]
    pub last_ms: i64,
    /// A merge request from it was approved.
    #[serde(default)]
    pub merged: bool,
    /// A merge request into it waits for this person's review.
    #[serde(default)]
    pub review: bool,
    #[serde(default)]
    pub mine: Mine,
    /// Its own visibility; none when it follows its project's or org's.
    #[serde(default)]
    pub visibility: Option<Visibility>,
    /// The server it syncs with (ADR 0032); none without one.
    #[serde(default)]
    pub server: Option<String>,
    /// It stays on this machine only, by its project's choice.
    #[serde(default)]
    pub local_only: bool,
    /// The Claude Code cloud session it records, as claude.ai names it
    /// (ADR 0040). A fork doesn't inherit it.
    #[serde(default)]
    pub cloud: Option<String>,
}

/// The cloud session among a thread's own deltas.
pub fn cloud_session<'a>(own: impl IntoIterator<Item = &'a silt_log::Delta>) -> Option<String> {
    own.into_iter().find_map(|delta| match &delta.op {
        silt_log::Op::Conv(silt_log::ConvOp::CloudSession { id, .. }) => Some(id.to_string()),
        _ => None,
    })
}

/// What this person keeps about a thread for themselves (ADR 0024).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Mine {
    pub pinned: bool,
    pub snoozed_until: Option<i64>,
    /// When it was snoozed, so activity since can wake it.
    pub snoozed_at: Option<i64>,
    pub wake_on_activity: bool,
    pub settled: bool,
    /// When they last opened it.
    pub seen_ms: Option<i64>,
}

impl Mine {
    /// A person's [`crate::ThreadState`] as the web UI reads it.
    pub fn of(state: Option<&crate::ThreadState>) -> Self {
        let value = |field: Option<crate::Stamped<bool>>| field.is_some_and(|field| field.value);
        let stamped = state.and_then(|state| state.snooze);
        let snooze = stamped.map(|snooze| snooze.value);
        Self {
            pinned: value(state.and_then(|state| state.pinned)),
            snoozed_until: snooze.and_then(|snooze| snooze.until),
            snoozed_at: stamped.filter(|snooze| snooze.value.until.is_some()).map(|snooze| snooze.at_ms),
            wake_on_activity: snooze.is_some_and(|snooze| snooze.wake_on_activity),
            settled: value(state.and_then(|state| state.settled)),
            seen_ms: state.and_then(|state| state.seen_ms).map(|seen| seen.value),
        }
    }
}

/// A change to [`Mine`]: the fields it sets; `seen` marks the thread seen now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct StateChange {
    pub pinned: Option<bool>,
    pub snooze: Option<Snooze>,
    pub settled: Option<bool>,
    pub seen: bool,
}

impl StateChange {
    /// The change as a [`crate::ThreadState`] stamped `now`, to merge.
    pub fn stamped(&self, thread: silt_types::ThreadId, now: i64) -> crate::ThreadState {
        let at = |value| crate::Stamped { value, at_ms: now };
        crate::ThreadState {
            thread,
            pinned: self.pinned.map(at),
            snooze: self.snooze.map(|snooze| crate::Stamped { value: snooze, at_ms: now }),
            settled: self.settled.map(at),
            seen_ms: self.seen.then_some(crate::Stamped { value: now, at_ms: now }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "line", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum ShowLine {
    Entry {
        kind: String,
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        input: String,
    },
    Edits {
        tool_use_id: Option<String>,
        files: Vec<FileEdit>,
    },
    Compaction {
        summary: Option<String>,
    },
    Turn {
        mark: String,
    },
    /// Where the history moves to another thread's part, or back.
    Segment {
        label: String,
    },
    /// A pull or a merge.
    Event {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FileEdit {
    pub path: String,
    pub inserted: usize,
    pub deleted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SearchResult {
    pub thread: String,
    pub kind: String,
    pub snippet: String,
    /// Set when the server answered.
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub status: Option<ThreadStatus>,
    #[serde(default)]
    pub repo: Option<String>,
    /// When it was said.
    #[serde(default)]
    pub time_ms: Option<i64>,
    /// The thread's turn it was said in, when the thread is here.
    #[serde(default)]
    pub turn: Option<usize>,
    /// The agent compacted its context after it.
    #[serde(default)]
    pub before_compaction: bool,
}

/// Server search filters (ADR 0011); dates are milliseconds since the epoch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SearchFilters {
    pub repo: Option<String>,
    pub author: Option<String>,
    pub status: Option<ThreadStatus>,
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DaemonStatus {
    /// The default server's URL, and this machine's device there.
    pub server: Option<String>,
    pub device: Option<String>,
    /// Every server this machine is signed in to, the default first (ADR 0032).
    #[serde(default)]
    pub servers: Vec<SignIn>,
    pub sync_error: Option<String>,
    pub threads: usize,
    /// Threads whose history here differs from the server's, so they no
    /// longer push.
    pub diverged: Vec<String>,
    /// Recovery forks made since the daemon started.
    #[serde(default)]
    pub recovered: Vec<String>,
    /// The daemon's own version, as the web UI's status bar shows it.
    #[serde(default)]
    pub version: String,
    /// The recovery forks behind `recovered`, by thread, for the web UI's
    /// reconcile page.
    #[serde(default)]
    pub recoveries: Vec<RecoveryView>,
}

/// A thread whose history here differed from its server's, and the
/// recovery thread its work here moved to (ADR 0009).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecoveryView {
    /// The thread, now as the server has it.
    pub thread: String,
    /// The fork of it holding this machine's work.
    pub recovery: String,
    pub time_ms: i64,
}

/// This machine's sign-in to a server, without its token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SignIn {
    pub url: String,
    pub user: String,
    pub device: String,
    /// This device's name there.
    pub name: String,
    #[serde(default)]
    pub sync_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MrSummary {
    pub id: String,
    pub fork: String,
    pub target: String,
    pub title: String,
    pub summary: String,
    /// `open`, `approved` or `declined: <reason>`.
    pub status: String,
    pub opened_ms: i64,
    /// When it was approved or declined.
    #[serde(default)]
    pub decided_ms: Option<i64>,
    /// What to change before it's taken, while it's open (ADR 0025).
    #[serde(default)]
    pub changes_requested: Option<String>,
    /// The fork has moved on since changes were asked for.
    #[serde(default)]
    pub updated_since: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MrDetails {
    pub mr: MrSummary,
    /// The fork's changes since the merge base.
    pub diff: String,
    /// Places both the fork and the target changed since the merge base.
    pub conflicts: Vec<String>,
    /// The fork's conversation since it forked.
    pub conversation: Vec<ShowLine>,
    /// The files it changes, and whether this person has looked at each as
    /// it is now (ADR 0024).
    #[serde(default)]
    pub files: Vec<MrFile>,
    /// A walk-through for reviewers, and who wrote it (ADR 0025).
    #[serde(default)]
    pub guide: Option<MrGuide>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MrGuide {
    pub text: String,
    pub by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MrFile {
    pub path: String,
    pub seen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HistoryEntry {
    pub index: usize,
    /// The thread the entry was recorded in.
    pub thread: String,
    /// `own`, `inherited` (before the fork) or `upstream` (brought in by a pull).
    pub origin: String,
    /// `user`, `wake` (the agent went on by itself), `assistant`,
    /// `thinking`, `tool_call`, `tool_result`, `compaction`, `merge` or
    /// `pull`.
    pub kind: String,
    pub text: String,
    pub time_ms: i64,
    /// A revert rewound it (ADR 0029).
    #[serde(default)]
    pub rewound: bool,
    /// On a `tool_call`: what the tool does, read from its input.
    #[serde(default)]
    pub tool: Option<ToolView>,
    /// On a `tool_result`: the call it answers and how it ended.
    #[serde(default)]
    pub outcome: Option<ToolOutcome>,
    /// On an `image`: the picture the entry before it carried.
    #[serde(default)]
    pub image: Option<ImageView>,
    /// On a `usage`: what one of the agent's messages cost.
    #[serde(default)]
    pub usage: Option<UsageView>,
    /// On a `plan`, or a `tool_call` that writes one: its steps.
    #[serde(default)]
    pub plan: Option<Vec<PlanStepView>>,
    /// The subagent transcript the entry is from, by its name; none for the
    /// session's own. A `task` entry names the call that started it.
    #[serde(default)]
    pub side: Option<String>,
    /// On a `task`: news of a subagent or background command.
    #[serde(default)]
    pub task: Option<TaskView>,
}

/// News of work an agent started that runs beside its turn: a subagent, or
/// a command left running in the background. Each says what it knows, and
/// later ones for the same task say more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TaskView {
    /// The agent's id for the task.
    pub id: String,
    /// The tool call that started it, when known.
    pub call: Option<String>,
    /// `subagent` or `background`, when this news says.
    pub kind: Option<String>,
    /// `running`, `completed`, `failed` or `stopped`.
    pub status: String,
    /// A subagent's transcript, by the name its entries' `side` gives.
    pub side: Option<String>,
    /// What the agent said of how it went.
    pub summary: Option<String>,
}

/// A step of an agent's plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PlanStepView {
    pub text: String,
    /// `pending`, `in_progress` or `completed`.
    pub status: String,
}

/// A picture a prompt or a tool's result carried, as its agent recorded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ImageView {
    pub media_type: String,
    /// Base64; empty when it was too large to show.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UsageView {
    /// The message it counts; one message may be recorded more than once.
    pub message: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub input: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub output: u64,
}

/// A question an agent asked, with the choices it offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct QuestionView {
    /// A word or two naming it, such as `Progress`.
    pub header: Option<String>,
    pub question: String,
    pub multi_select: bool,
    pub options: Vec<ChoiceView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChoiceView {
    pub label: String,
    pub description: Option<String>,
}

/// What the person answered to one question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AnswerView {
    pub question: String,
    pub answer: String,
}

/// A tool call as a thread shows it, the same for every agent (ADR 0044).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ToolView {
    pub id: String,
    pub name: String,
    /// `execute`, `read`, `search`, `edit`, `question`, `fetch` or `other`.
    pub kind: String,
    /// The command it ran, the file it read or changed, or what it looked for.
    pub subject: Option<String>,
    /// Where a search looked, or what a command was for.
    pub detail: Option<String>,
    /// What a question tool asked.
    #[serde(default)]
    pub questions: Vec<QuestionView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ToolOutcome {
    /// The id of the call it answers.
    pub call: String,
    pub failed: bool,
    pub exit_code: Option<i32>,
    /// A question tool's answers.
    #[serde(default)]
    pub answers: Vec<AnswerView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HistoryPage {
    pub thread: String,
    pub total: usize,
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum CommentLocation {
    Entry {
        index: usize,
        excerpt: String,
    },
    /// Where the commented lines are now; `outdated` once they're deleted.
    Lines {
        path: String,
        start: usize,
        end: usize,
        outdated: bool,
    },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CommentView {
    pub id: String,
    pub by: String,
    pub time_ms: i64,
    pub text: String,
    pub resolved: bool,
    pub location: CommentLocation,
    /// The conversation under it, oldest first (ADR 0025).
    #[serde(default)]
    pub replies: Vec<ReplyView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReplyView {
    pub id: String,
    pub by: String,
    pub time_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TurnInfo {
    /// 1-based, as `silt diff --turn` counts them.
    pub number: usize,
    /// The history entry of the turn's prompt, or of the agent going on
    /// by itself for a turn it opened.
    pub entry: Option<usize>,
    pub prompt: String,
}

/// Whether a history entry of `kind` starts a turn: a prompt, or the agent
/// going on by itself.
pub fn opens_turn(kind: &str) -> bool {
    matches!(kind, "user" | "wake")
}

/// A notification, with the thread id as text like everywhere in this API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InboxEntry {
    pub id: i64,
    pub kind: String,
    pub thread: String,
    pub actor: String,
    pub text: String,
    pub created_ms: i64,
    pub read: bool,
    #[serde(default)]
    pub action: Option<InboxAction>,
    #[serde(default)]
    pub notify: bool,
    /// The server it came from, on a machine signed in to several (ADR
    /// 0032); ids are that server's.
    #[serde(default)]
    pub server: Option<String>,
}

/// A project as the web UI shows it (ADR 0023): the server's record when
/// connected, with this machine's clone of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Project {
    /// The normalized remote that names it.
    pub repo: String,
    pub name: String,
    pub org: Option<String>,
    pub origin_url: Option<String>,
    pub visibility: Option<Visibility>,
    pub default_agent: Option<String>,
    pub default_preset: Option<String>,
    pub git_notes: crate::GitNotes,
    pub forge_comments: bool,
    /// Its threads: those the person can see on the server, or those here.
    pub threads: u32,
    /// Merge requests still open into its threads, as this machine knows them.
    pub open_mrs: u32,
    /// This machine's clone of it.
    pub clone: Option<PathBuf>,
    /// Where its new threads go from this machine (ADR 0032): a server's
    /// URL, or `local`; none for the default server.
    #[serde(default)]
    pub home: Option<String>,
    /// The server this record is from; none for a clone no server knows.
    #[serde(default)]
    pub server: Option<String>,
}

/// Where a new comment goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum CommentOn {
    /// A history entry, by its index.
    Entry { index: usize },
    /// Lines `start..=end` (1-based) of a file as the thread has it now.
    Lines { path: String, start: usize, end: usize },
}

/// One of the person's signed-in devices (ADR 0026).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceView {
    pub id: String,
    /// `device`, a machine with a daemon, or `phone`, a paired browser.
    pub kind: String,
    pub name: String,
    pub created_ms: i64,
    pub last_used_ms: i64,
    /// Heard from in the last minute and a half.
    pub online: bool,
    /// The device asking, when a device asks.
    pub this: bool,
    pub capabilities: Capabilities,
    /// The agents it runs, as it last reported them.
    pub agents: Vec<DeviceAgent>,
    /// The agents it could start, to continue a thread there.
    pub startable: Vec<Startable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeviceAgent {
    pub thread: String,
    pub title: Option<String>,
    pub agent: String,
    pub state: AgentState,
}

/// One row of the history graph, newest first in a page (ADR 0029, 0030).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GraphRow {
    /// The delta it comes from, which no other row shares.
    pub id: String,
    /// `turn`, `commit`, `fork`, `request` or `merge`.
    pub kind: String,
    pub thread: String,
    pub family: String,
    pub time_ms: i64,
    /// A turn's number, or the turn a commit came in.
    pub turn: Option<usize>,
    /// A turn's prompt, a commit's subject or id, a request's or merge's
    /// title.
    pub text: String,
    /// The other thread: a fork's source, a request's target, a merge's
    /// fork.
    pub other: Option<String>,
    /// A request's state: `open`, `approved` or `declined`.
    pub status: Option<String>,
    pub owner: String,
    pub agent: Option<String>,
    /// Its thread is settled for the person.
    pub settled: bool,
    /// A commit's id, beside its subject in `text`.
    #[serde(default)]
    pub commit: Option<String>,
}

/// Which rows of the graph to show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct GraphQuery {
    /// Only this project's threads, by its normalized remote.
    pub project: Option<String>,
    /// Only this family's threads.
    pub family: Option<String>,
    /// Only the person's own threads.
    pub mine: bool,
    /// Comma-separated kinds to show; all by default.
    pub kinds: Option<String>,
    /// Settled threads too.
    pub settled: bool,
    /// Rows older than this, for the next page.
    pub before: Option<i64>,
    pub limit: Option<usize>,
}

impl GraphQuery {
    pub const PAGE: usize = 200;

    pub fn shows(&self, kind: &str) -> bool {
        self.kinds.as_deref().is_none_or(|kinds| kinds.split(',').any(|shown| shown.trim() == kind))
    }

    /// Newest first, older than `before`, at most a page.
    pub fn page(&self, mut rows: Vec<GraphRow>) -> Vec<GraphRow> {
        rows.retain(|row| self.before.is_none_or(|before| row.time_ms < before) && (self.settled || !row.settled));
        rows.sort_by(|a, b| b.time_ms.cmp(&a.time_ms).then_with(|| b.id.cmp(&a.id)));
        rows.truncate(self.limit.unwrap_or(Self::PAGE).min(Self::PAGE));
        rows
    }
}

/// A commit a thread made, with the turn it came in (ADR 0030).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CommitView {
    pub commit: String,
    pub subject: Option<String>,
    pub branch: Option<String>,
    pub turn: Option<usize>,
    pub time_ms: i64,
}

/// Which file of a thread a permalink shows, and at which version (ADR 0031):
/// its frontier as `Frontier`'s text, the thread's head when left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileQuery {
    pub path: String,
    #[serde(default)]
    pub at: Option<String>,
}

/// A file as a permalink shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FileAt {
    pub path: String,
    /// The version shown, for a link that keeps showing it.
    pub at: String,
    /// `None` for a binary file.
    pub text: Option<String>,
}

/// A read-only link to a thread (ADR 0027).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThreadLink {
    pub id: i64,
    pub created_ms: i64,
    pub expires_ms: i64,
    /// Shows the thread as it was when made, not as it goes on.
    pub frozen: bool,
    pub revoked: bool,
    /// Only when just made: the server keeps no copy of the token.
    #[serde(default)]
    pub url: Option<String>,
}

/// What a new link is: for how many days (1, 7 or 30), and whether frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewLink {
    pub days: u32,
    #[serde(default)]
    pub frozen: bool,
}

/// A one-time code that signs a phone in (ADR 0028), to show as a QR code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PhonePairing {
    pub code: String,
    /// What the QR code holds: the page that signs the phone in.
    pub url: String,
    /// The QR code, as an SVG image.
    pub qr_svg: String,
    pub expires_ms: i64,
}

/// Renames a device or changes what it may do; what's left out stays.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct DeviceChange {
    pub name: Option<String>,
    pub capabilities: Option<Capabilities>,
}

#[cfg(test)]
mod tests {
    use silt_types::ThreadId;

    use super::*;
    use crate::Stamped;

    fn row(id: &str, kind: &str, time_ms: i64, settled: bool) -> GraphRow {
        GraphRow {
            id: id.into(),
            kind: kind.into(),
            thread: "t".into(),
            family: "f".into(),
            time_ms,
            turn: None,
            text: String::new(),
            other: None,
            status: None,
            owner: "alice".into(),
            agent: None,
            settled,
            commit: None,
        }
    }

    #[test]
    fn a_graph_page_is_newest_first_before_its_cursor_and_hides_settled_threads() {
        let rows =
            vec![row("a", "turn", 1, false), row("b", "commit", 3, false), row("c", "turn", 2, true), row("d", "fork", 4, false)];
        let ids = |rows: Vec<GraphRow>| rows.into_iter().map(|row| row.id).collect::<Vec<_>>();
        assert_eq!(ids(GraphQuery::default().page(rows.clone())), ["d", "b", "a"]);
        let older = GraphQuery { before: Some(4), settled: true, limit: Some(2), ..GraphQuery::default() };
        assert_eq!(ids(older.page(rows)), ["b", "c"]);
        let kinds = GraphQuery { kinds: Some("turn, fork".into()), ..GraphQuery::default() };
        assert!(kinds.shows("turn") && kinds.shows("fork") && !kinds.shows("commit"));
        assert!(GraphQuery::default().shows("merge"));
    }

    #[test]
    fn a_change_from_the_web_ui_sets_only_what_it_names() {
        let thread = ThreadId(3);
        let change = StateChange { pinned: Some(true), snooze: None, settled: None, seen: true };
        let state = change.stamped(thread, 50);
        assert_eq!(state.pinned, Some(Stamped { value: true, at_ms: 50 }));
        assert_eq!((state.snooze, state.settled), (None, None));
        assert_eq!(state.seen_ms, Some(Stamped { value: 50, at_ms: 50 }));

        let snoozed = StateChange { snooze: Some(Snooze { until: Some(900), wake_on_activity: true }), ..StateChange::default() };
        let mut both = state.clone();
        both.merge(&snoozed.stamped(thread, 60));
        let shown = Mine::of(Some(&both));
        assert!(shown.pinned && shown.wake_on_activity && !shown.settled);
        assert_eq!((shown.snoozed_until, shown.snoozed_at, shown.seen_ms), (Some(900), Some(60), Some(50)));
        assert_eq!(Mine::of(None), Mine::default());
    }
}
