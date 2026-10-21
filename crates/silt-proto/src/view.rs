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
    /// `user`, `assistant`, `thinking`, `tool_call`, `tool_result`,
    /// `compaction`, `merge` or `pull`.
    pub kind: String,
    pub text: String,
    pub time_ms: i64,
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
    /// The history entry of the turn's prompt.
    pub entry: Option<usize>,
    pub prompt: String,
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
    /// Its threads: those the person can see on the server, or those here.
    pub threads: u32,
    /// Merge requests still open into its threads, as this machine knows them.
    pub open_mrs: u32,
    /// This machine's clone of it.
    pub clone: Option<PathBuf>,
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
