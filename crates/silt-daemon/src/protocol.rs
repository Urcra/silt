use std::path::PathBuf;

use serde::{Deserialize, Serialize};
pub use silt_proto::view::{
    AnswerView, ChoiceView, CommentLocation, CommentOn, CommentView, CommitView, DaemonStatus, DeviceAgent, DeviceChange,
    DeviceView, FileEdit, HistoryEntry, HistoryPage, ImageView, InboxEntry, LineSpan, Mine, MrDetails, MrFile, MrGuide,
    MrSummary, NewLink, PlanStepView, Project, QuestionView, ReplyView, SearchFilters, SearchResult, ShowLine, StateChange,
    TaskView, ThreadLink, ThreadSummary, ToolOutcome, ToolView, TurnInfo, UsageView,
};
pub use silt_proto::{ChoiceAsked, QuestionAsked, QuestionFieldAsked, SignInAsked, SignInChoice, SignInShown};

/// One request per line on the daemon socket, answered by one response line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    Ping,
    /// Where this daemon looks for agents' sessions: each of
    /// `silt_agents::HOME_VARIABLES` with its value here.
    AgentHomes,
    /// Whether this daemon's program was replaced since it started.
    DaemonBuild,
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
        /// The Claude Code cloud session it runs as, from the container.
        #[serde(default)]
        cloud: Option<silt_types::CloudSessionId>,
    },
    /// An agent hook fired; the input is what the agent passed to the hook.
    Hook {
        event: String,
        input: serde_json::Value,
    },
    /// The agent running in `thread` exited: record the rest of its session.
    EndSession {
        thread: String,
    },
    /// This machine's clone of the thread's repository, for requests made
    /// from the web UI, which has no working directory.
    CloneOf {
        thread: String,
    },
    /// This machine's settings for chats (ADR 0059).
    ChatSettings,
    SetChatSettings {
        settings: ChatSettings,
    },
    /// Makes a chat's folder a project of its own, with the chat's threads
    /// in it (ADR 0059). Answers with the project's name.
    MakeProject {
        thread: String,
    },
    /// Take over recording the running agent in `thread` from the wrapper
    /// that recorded it on its own until the daemon answered (ADR 0018).
    Adopt {
        thread: String,
    },
    Threads,
    Show {
        thread: String,
    },
    /// What the thread changed: one `turn`, or everything since `base`
    /// (`merge-base`, `last-commit`, `last-turn`, `commit:<sha>`), by
    /// default since it started; or how it differs from a branch as git has
    /// it now (`main`, `branch:<name>`).
    Diff {
        thread: String,
        turn: Option<usize>,
        /// With `turn`, the last of a range of turns.
        #[serde(default)]
        to_turn: Option<usize>,
        #[serde(default)]
        base: Option<String>,
        /// Only these of the changed files, in path order: a page of them.
        #[serde(default)]
        files: Option<FilePage>,
        /// Only the file at this path, new or old.
        #[serde(default)]
        path: Option<String>,
        /// Leave out what git itself rewrote, as a checkout, pull or stash
        /// does (ADR 0069); not against a branch, which is git's already.
        #[serde(default)]
        hide_git: bool,
    },
    /// [`Request::Diff`]'s files, each with its counts and size but not its
    /// lines.
    DiffStats {
        thread: String,
        turn: Option<usize>,
        #[serde(default)]
        to_turn: Option<usize>,
        #[serde(default)]
        base: Option<String>,
        #[serde(default)]
        hide_git: bool,
    },
    /// For [`Request::Diff`]'s files the person marked seen, whether each is
    /// as they saw it, and the lines new since (ADR 0067).
    ThreadSeen {
        thread: String,
        turn: Option<usize>,
        #[serde(default)]
        to_turn: Option<usize>,
        #[serde(default)]
        base: Option<String>,
        /// As [`Request::Diff`]'s.
        #[serde(default)]
        hide_git: bool,
    },
    /// Marks a file of the thread seen, or not, at a version the review read.
    MarkThreadSeen {
        thread: String,
        mark: silt_proto::view::MarkSeen,
    },
    /// For [`Request::Diff`]'s files, the turn that last wrote each of their
    /// new lines.
    DiffTurns {
        thread: String,
        turn: Option<usize>,
        #[serde(default)]
        to_turn: Option<usize>,
        #[serde(default)]
        base: Option<String>,
        #[serde(default)]
        files: Option<FilePage>,
        #[serde(default)]
        path: Option<String>,
        /// As [`Request::Diff`]'s.
        #[serde(default)]
        hide_git: bool,
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
    /// The sessions on disk not yet recorded or imported (ADR 0043), of
    /// `agent` or all, that ran under one of `paths` or anywhere.
    ImportCandidates {
        #[serde(default)]
        paths: Vec<PathBuf>,
        #[serde(default)]
        agent: Option<String>,
    },
    /// Imports one of them into a thread kept on this machine.
    Import {
        candidate: ImportCandidate,
    },
    /// The folders the agents' sessions on disk ran in, for a first open
    /// (ADR 0044).
    FoundFolders,
    /// Imports every session on disk not yet imported that ran under one
    /// of `paths`, or anywhere without any. How many it imported.
    ImportFound {
        #[serde(default)]
        paths: Vec<PathBuf>,
    },
    /// Sends imported threads kept on this machine to `server`, or the
    /// default one: `threads`, those of `project`, or with neither all.
    Send {
        #[serde(default)]
        threads: Vec<String>,
        #[serde(default)]
        project: Option<String>,
        #[serde(default)]
        server: Option<String>,
    },
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
        /// Private from the start, not made private afterwards.
        #[serde(default)]
        private: bool,
        /// From where this turn ended rather than from now (ADR 0029).
        #[serde(default)]
        turn: Option<usize>,
    },
    /// Merges a fork's source into it; `thread` defaults to the one checked
    /// out at `cwd`.
    Pull {
        thread: Option<String>,
        cwd: PathBuf,
    },
    /// Takes the thread back to how turn `turn` left it (ADR 0029);
    /// `thread` defaults to the one checked out at `cwd`.
    Revert {
        thread: Option<String>,
        cwd: PathBuf,
        turn: usize,
    },
    MrCreate {
        thread: Option<String>,
        cwd: PathBuf,
        title: Option<String>,
        summary: Option<String>,
    },
    MrList {
        thread: Option<String>,
    },
    MrShow {
        mr: String,
        /// As [`Request::Diff`]'s.
        #[serde(default)]
        hide_git: bool,
    },
    MrApprove {
        mr: String,
    },
    MrDecline {
        mr: String,
        reason: Option<String>,
    },
    /// Keeps a merge request open, asking its fork for changes first.
    MrRequestChanges {
        mr: String,
        reason: String,
    },
    /// Keeps `text` as the request's change guide, or without it asks the
    /// fork's agent running here to write one.
    MrGuide {
        mr: String,
        #[serde(default)]
        text: Option<String>,
    },
    /// Ticks a file of a merge request as looked at, or not, for this person.
    MrSeen {
        mr: String,
        path: String,
        seen: bool,
    },
    /// A page of a thread's full history, across compactions and forks.
    HistoryRead {
        thread: String,
        offset: usize,
        limit: usize,
    },
    /// Search within a thread's history and the threads it came from.
    HistorySearch {
        thread: String,
        query: String,
        limit: u32,
    },
    /// The thread's turns, for showing each turn's diff next to its prompt.
    Turns {
        thread: String,
    },
    /// The thread's files a checkout on this machine doesn't write: names
    /// its file system can't hold, or that differ only in case.
    Unwritten {
        thread: String,
    },
    /// The commits the thread made, with their turns (ADR 0030).
    Commits {
        thread: String,
    },
    /// The history graph over the families here.
    Graph {
        query: silt_proto::view::GraphQuery,
    },
    /// The threads and turns that made commits starting with `commit`.
    FindCommit {
        commit: String,
    },
    /// Starts the web UI if needed; answers with its URL.
    OpenWeb {
        /// The clone `silt open` ran in; its repository's threads can then
        /// be opened from the web UI even when they started elsewhere.
        #[serde(default)]
        cwd: Option<PathBuf>,
        /// A URL that works once, for a browser's command line, where other
        /// users could read a lasting token.
        #[serde(default)]
        once: bool,
    },
    /// The turns that changed a file of the thread.
    FileHistory {
        thread: String,
        path: String,
    },
    /// A file of the thread at a version, for a permalink (ADR 0031).
    FileAt {
        thread: String,
        query: silt_proto::view::FileQuery,
    },
    /// Other threads of the repository at `cwd` that changed `path`.
    InFlight {
        cwd: PathBuf,
        path: String,
    },
    /// Who wrote each line of `path`, as seen from `cwd` (ADR 0042).
    Blame {
        cwd: PathBuf,
        path: String,
    },
    /// Comments on a history entry or on lines of a file (ADR 0011).
    /// `cwd` is a clone to read the thread's files from if needed.
    Comment {
        thread: String,
        on: CommentOn,
        text: String,
        cwd: PathBuf,
    },
    Comments {
        thread: String,
        /// A diff base to place code comments in too, such as `last-turn`.
        #[serde(default)]
        base: Option<String>,
        /// Place code comments as a diff with git's rewrites left out shows
        /// the lines (ADR 0069).
        #[serde(default)]
        hide_git: bool,
        cwd: PathBuf,
    },
    /// Anchors lines for a pending review comment, recording nothing
    /// (ADR 0066).
    AnchorLines {
        thread: String,
        on: CommentOn,
        cwd: PathBuf,
    },
    /// Where pending review comments' anchors are now, and at `base`.
    PlaceAnchors {
        thread: String,
        place: silt_proto::view::PlaceAnchors,
        cwd: PathBuf,
    },
    /// Marks comments as sent to the thread's agent (ADR 0055).
    CommentsSent {
        thread: String,
        comments: Vec<String>,
        cwd: PathBuf,
    },
    ResolveComment {
        thread: String,
        comment: String,
        cwd: PathBuf,
    },
    /// Answers a comment, under it.
    Reply {
        thread: String,
        comment: String,
        text: String,
        cwd: PathBuf,
    },
    /// Sets the visibility of the project at `cwd`, or of `thread`'s family.
    SetVisibility {
        cwd: PathBuf,
        thread: Option<String>,
        visibility: silt_proto::Visibility,
    },
    /// `request` asked of the server at `server` where it belongs to no
    /// thread, such as orgs and devices, rather than the default (ADR 0032).
    OnServer {
        server: String,
        asking: Box<Request>,
    },
    /// What an agent's run shows the person (ADR 0068); only inside
    /// [`Request::Scoped`], for the thread its token was made for.
    PutArtifact {
        artifact: NewArtifact,
    },
    /// An artifact's content, by the blob a history entry of the thread names.
    ArtifactContent {
        thread: String,
        blob: String,
    },
    /// A request of an agent's history tools, answered only for the
    /// workspace its run's token was made for (ADR 0065).
    Scoped {
        token: String,
        asking: Box<Request>,
    },
    /// The server's notifications for this user (ADR 0013).
    Inbox {
        #[serde(default)]
        all: bool,
    },
    InboxRead {
        read: silt_proto::InboxRead,
    },
    /// Moves a thread to the trash, everywhere (ADR 0013).
    ThreadDelete {
        thread: String,
    },
    ThreadRestore {
        thread: String,
    },
    /// Removes a thread in the trash for good, everywhere.
    ThreadPurge {
        thread: String,
    },
    /// The threads in the trash.
    Trash,
    /// This machine's viewer of a thread: what file is open and the draft
    /// prompt; answers with everyone else on it (ADR 0014).
    Presence {
        thread: String,
        #[serde(default)]
        file: Option<String>,
        #[serde(default)]
        line: Option<u32>,
        #[serde(default)]
        draft: String,
        #[serde(default)]
        leaving: bool,
    },
    /// The family's deltas after the first `after`, for the browser's
    /// replica (ADR 0014).
    Replica {
        thread: String,
        after: usize,
    },
    /// A git blob's text, for the browser's replica.
    GitText {
        thread: String,
        oid: String,
    },
    /// Edits made in the browser's replica, postcard and base64 encoded.
    BrowserDeltas {
        thread: String,
        deltas: String,
    },
    /// A fingerprint of the thread's live state.
    Live {
        thread: String,
    },
    /// Lets `user` write into the thread, or stops it (ADR 0014).
    ThreadShare {
        thread: String,
        user: String,
        remove: bool,
    },
    /// The agents the web UI can start, and the clones to start new threads
    /// in.
    AcpAgents,
    /// Starts an ACP agent in `thread`, or in a new thread of `clone`
    /// (ADR 0014). Answers with the thread.
    AcpStart {
        #[serde(default)]
        thread: Option<String>,
        #[serde(default)]
        clone: Option<PathBuf>,
        agent: String,
        #[serde(default)]
        options: StartOptions,
    },
    /// The branches a new thread of `clone` can start from.
    Branches {
        clone: PathBuf,
    },
    /// Starts a Claude Code cloud session for `clone` with `task` as its
    /// first message (ADR 0040).
    CloudStart {
        clone: PathBuf,
        task: String,
    },
    /// A cloud session this daemon started; sent by the start itself.
    CloudLaunched {
        launch: CloudLaunch,
    },
    /// The cloud sessions this daemon started.
    CloudLaunches,
    /// Queues `text` into the cloud session the thread records.
    CloudSend {
        thread: String,
        text: String,
    },
    /// The projects the person works in, here and on the server.
    Projects,
    /// Pins, snoozes, settles or marks seen a thread, for this person.
    ThreadState {
        thread: String,
        change: StateChange,
    },
    /// The person's orgs, their members, teams and projects.
    Orgs,
    /// Which kinds of notification notify the person, on the server.
    NotificationSettings,
    NotificationSetting {
        setting: silt_proto::KindSetting,
    },
    /// A one-time code that signs a phone in to the server's web app.
    PairPhone,
    /// The thread's read-only links (ADR 0027), on the server.
    ThreadLinks {
        thread: String,
    },
    ThreadLink {
        thread: String,
        link: NewLink,
    },
    ThreadLinkRevoke {
        thread: String,
        link: i64,
    },
    /// The person's devices, on the server.
    Devices,
    DeviceChange {
        id: String,
        change: DeviceChange,
    },
    DeviceRevoke {
        id: String,
    },
    /// Changes an org, a team or a project's place, on the server.
    OrgChange {
        change: silt_proto::OrgChange,
    },
    /// Makes the clone at `path` this machine's clone of its project.
    AddClone {
        path: PathBuf,
    },
    /// Clones `url` to `path` with the person's git, then adds the clone.
    CloneProject {
        url: String,
        path: PathBuf,
    },
    /// Makes the folder `path` with an empty git repository, then adds it
    /// (ADR 0053).
    NewProject {
        path: PathBuf,
    },
    /// Every workspace, the projects each holds and its rules (ADR 0065).
    Workspaces,
    /// Whether a terminal may run `agent` (its adapter's id) with `args`,
    /// and the locks, variables and token it runs with: asked before the
    /// session starts, so a refusal leaves nothing behind.
    RunRules {
        /// The thread a resume or fork continues; a new run names its folder.
        thread: Option<String>,
        cwd: Option<PathBuf>,
        agent: String,
        args: Vec<String>,
    },
    /// Replaces the workspaces with these, as `workspaces.json` keeps them.
    SetWorkspaces {
        workspaces: silt_workspace::Workspaces,
    },
    /// Puts the project `repo` in a workspace, or back under the match rules
    /// with none.
    AssignProject {
        repo: String,
        workspace: Option<String>,
    },
    /// Where the project `repo`'s new threads go (ADR 0032): a server's
    /// URL, or `local` for this machine only.
    ProjectHome {
        repo: String,
        home: String,
    },
    /// Sends the person's draft, with teammates' drafts, as the next prompt,
    /// or queues it while the agent is busy (ADR 0022).
    AcpPrompt {
        thread: String,
        text: String,
        /// Plan first: the agent writes a plan to approve before editing.
        #[serde(default)]
        plan: bool,
        #[serde(default)]
        attachments: Vec<PromptAttachment>,
    },
    /// Switches the thread's agent to one of its modes, kept for the
    /// thread's later runs.
    AcpMode {
        thread: String,
        mode: String,
    },
    /// Picks one of a setting's choices, such as the agent's model.
    AcpSetting {
        thread: String,
        id: String,
        value: String,
    },
    /// Chooses which of the thread's agent's permission requests the daemon
    /// answers by itself.
    AgentPreset {
        thread: String,
        preset: Preset,
    },
    /// Interrupts the agent's turn to tell it `text`.
    AcpSteer {
        thread: String,
        text: String,
    },
    /// The prompts waiting for the thread's agent, next first.
    Queue {
        thread: String,
    },
    QueueAdd {
        thread: String,
        text: String,
    },
    QueueEdit {
        thread: String,
        id: i64,
        text: String,
    },
    QueueMove {
        thread: String,
        id: i64,
        /// The place to move it to, from the front.
        to: usize,
    },
    QueueRemove {
        thread: String,
        id: i64,
    },
    /// Sends a queued prompt now, interrupting the agent if it works.
    QueueSend {
        thread: String,
        id: i64,
    },
    AcpCancel {
        thread: String,
    },
    /// Ends a command the agent left running in the background, by the call
    /// that started it.
    AcpStopTask {
        thread: String,
        call: String,
    },
    /// Answers a permission request with an option, or refuses it.
    AcpAnswer {
        thread: String,
        id: u64,
        #[serde(default)]
        option: Option<String>,
    },
    /// Answers a question with each field's chosen values, or declines it.
    AcpReply {
        thread: String,
        id: u64,
        #[serde(default)]
        answers: Option<Vec<(String, Vec<String>)>>,
    },
    AcpStop {
        thread: String,
    },
    /// Signs the agent in by one of the ways it lists (ADR 0056).
    AcpSignIn {
        thread: String,
        method: String,
    },
    /// A code from the sign-in's page, for the sign-in to take.
    AcpSignInCode {
        thread: String,
        code: String,
    },
    AcpSignInCancel {
        thread: String,
    },
    /// Pushes the thread's branch and answers the forge's page for a new
    /// pull request of it.
    OpenPullRequest {
        thread: String,
    },
    /// Why the thread can't open a pull request now, or none when it can.
    PullRequestBlocker {
        thread: String,
    },
    /// Compresses chunks stored uncompressed and gives the space deleted
    /// rows left back to the disk, holding the store meanwhile.
    Compact,
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
        Self::Error { message: message.to_string() }
    }

    pub fn failure(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Error { message: error_chain(error) }
    }
}

/// `error` and what caused it, on one line.
pub fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut message = error.to_string();
    let mut cause = error.source();
    while let Some(source) = cause {
        let text = source.to_string();
        if !message.ends_with(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        cause = source.source();
    }
    message
}

/// The program a daemon runs, as `Request::DaemonBuild` answers: replaced
/// since it started, as installing Silt again does, it still runs the old
/// build and serves the old web UI.
/// What `silt_show` or `silt_artifact` asks to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewArtifact {
    pub title: String,
    /// `html`, `svg`, `markdown`, `mermaid` or `image`.
    pub kind: String,
    #[serde(default)]
    pub content: Option<String>,
    /// A file holding the content, relative to the run's checkout.
    #[serde(default)]
    pub path: Option<String>,
    /// An artifact of the thread's, to add a version of.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
    /// An image shown in the conversation rather than beside it.
    #[serde(default)]
    pub inline: bool,
}

/// The artifact a [`Request::PutArtifact`] kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactPut {
    pub id: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonBuild {
    pub exe: Option<PathBuf>,
    pub replaced: bool,
    /// The agents working in it, which stopping it would end: those the web
    /// UI started, unless idle.
    pub working: usize,
}

/// What compacting the store gave back: the database file's size before and
/// after, and how many chunks it compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compaction {
    pub before: u64,
    pub after: u64,
    pub recompressed: usize,
}

/// A page of a diff's files: `limit` of them from the `offset`th.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePage {
    pub offset: usize,
    pub limit: usize,
}

/// A session on disk that Silt hasn't recorded (ADR 0043).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCandidate {
    pub agent: String,
    pub id: String,
    pub transcript: PathBuf,
    pub cwd: Option<PathBuf>,
    pub started_ms: Option<i64>,
}

/// What the web UI's voice button can do on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct VoiceStatus {
    /// Whether `silt-voice` is installed.
    pub helper: bool,
    /// Whether the speech model has been downloaded.
    pub model: bool,
    pub download: Option<VoiceDownload>,
    /// Why the last download failed.
    pub failed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct VoiceDownload {
    pub done: u64,
    pub total: u64,
}

/// What a dictation says as it goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum VoiceEvent {
    /// The words of the current phrase so far; they may still change.
    Partial {
        text: String,
    },
    /// A finished phrase.
    Segment {
        text: String,
    },
    /// Silence went on past a finished phrase.
    Paused,
    /// All the audio has been heard.
    Done,
    Failed {
        message: String,
    },
}

/// A folder the agents' sessions ran in: a clone's root, or the folder
/// itself outside any git repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FoundFolder {
    pub path: PathBuf,
    /// The normalized origin, empty without one.
    pub repo: String,
    /// Threads start only in a git repository; outside one, its sessions
    /// can still be imported.
    pub git: bool,
    /// Whether this machine knows it as a clone already.
    pub added: bool,
    /// Sessions not imported yet, by agent name, most first.
    pub sessions: Vec<FoundSessions>,
    pub last_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FoundSessions {
    pub agent: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedThread {
    pub thread: String,
    pub agent: String,
    pub session_id: String,
    /// Empty for a session that ran outside any clone with an origin.
    pub project: String,
    pub cwd: Option<PathBuf>,
    pub started_ms: i64,
    /// Records read from its files.
    pub lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sent {
    pub threads: usize,
    pub server: String,
    /// Distinct secrets the push redacts from them.
    pub redacted: usize,
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
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Mount {
    /// The clone the checkout was made from.
    pub source: PathBuf,
    /// Where this project's linked paths live on this machine.
    pub shared: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Reverted {
    pub thread: String,
    pub turn: usize,
    /// The files written back.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Pulled {
    pub thread: String,
    pub from: String,
    /// Conversation entries the pull brought in.
    pub messages: usize,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HistoryHit {
    /// The entry's index in the thread's history.
    pub index: usize,
    pub kind: String,
    pub snippet: String,
}

/// Edits to one file in one turn, by one tool call or outside tool calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InFlight {
    pub thread: String,
    pub owner: String,
    pub files: Vec<String>,
    pub last_ms: i64,
}

/// A line of a file and where it came from (ADR 0042).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlameLine {
    /// From 1, as the file is on disk.
    pub line: u32,
    pub text: String,
    pub source: BlameSource,
    /// The commit git blame names; none for a line not committed.
    pub commit: Option<String>,
    pub thread: Option<String>,
    pub turn: Option<usize>,
    pub prompt: Option<String>,
    /// `person` or the agent's name.
    pub author: Option<String>,
    pub time_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlameSource {
    /// The family's log on this machine.
    Log,
    /// The commit's note in `refs/notes/silt`.
    Notes,
    /// Git alone: Silt knows nothing of the line.
    Git,
}

/// Deltas for the browser's replica (ADR 0014).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LiveState {
    pub deltas: usize,
    /// The agent the web UI started in the thread, if any.
    pub agent: Option<AgentStatus>,
    /// Which permission requests the daemon answers by itself.
    pub preset: Preset,
}

pub use silt_workspace::Preset;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AgentStatus {
    pub agent: String,
    /// The agent's registry id, as `AgentChoice` names it.
    #[serde(default)]
    pub agent_id: String,
    pub state: silt_proto::AgentState,
    /// What the agent is saying or thinking right now, not recorded yet.
    pub streaming: String,
    pub thinking: String,
    pub permissions: Vec<PermissionAsked>,
    /// Questions the agent asked and waits on.
    pub questions: Vec<QuestionAsked>,
    /// The sign-in it waits on, when its own expired (ADR 0056).
    #[serde(default)]
    pub sign_in: Option<SignInAsked>,
    /// How much of its context window the session uses, when it says.
    pub context: Option<ContextUse>,
    pub error: Option<String>,
    /// The agent's modes, when it has any.
    pub modes: Option<AgentModes>,
    /// Its settings with a choice of values, such as its model.
    pub settings: Vec<AgentSetting>,
    /// What it takes attached to a prompt.
    pub takes_images: bool,
    /// Files other than text and images.
    pub takes_files: bool,
    /// The commands it offers, which a prompt starting with `/` runs.
    #[serde(default)]
    pub commands: Vec<AgentCommand>,
    /// The calls whose commands still run in the background, which Stop
    /// ends.
    #[serde(default)]
    pub stoppable: Vec<String>,
}

/// A file sent with a prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PromptAttachment {
    pub name: String,
    pub mime: String,
    /// The content, in base64.
    pub data: String,
}

/// How an agent starts, beyond which agent: for a new thread, where it
/// works and who sees it, and for any, its mode, preset and first prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(default)]
pub struct StartOptions {
    /// A branch, tag or commit for a new thread's checkout to start from,
    /// rather than what the clone has checked out.
    pub base: Option<String>,
    /// A new thread works in the clone itself rather than a checkout of its
    /// own.
    pub here: bool,
    pub mode: Option<String>,
    /// The agent's settings to choose once it's up, such as its model or
    /// thinking level, by id and value.
    #[serde(default)]
    pub settings: Vec<(String, String)>,
    pub preset: Option<Preset>,
    pub visibility: Option<silt_proto::Visibility>,
    /// Sent once the agent is ready.
    pub prompt: Option<String>,
    /// The first prompt is a Plan first one.
    pub plan: bool,
    /// Files sent with the first prompt.
    pub attachments: Vec<PromptAttachment>,
    /// A new thread is a chat, in a folder of its own rather than a clone
    /// (ADR 0059).
    pub chat: bool,
    /// A chat the new thread continues in a project: the agent starts from
    /// its conversation.
    pub continues: Option<String>,
    /// Where in the new thread's checkout the chat's files are copied, a
    /// relative path; nowhere when unset.
    pub copy_to: Option<String>,
    /// The workspace a new chat is in, by id; the default when unset
    /// (ADR 0065).
    pub workspace: Option<String>,
}

/// The workspaces on this machine (ADR 0065).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct WorkspacesView {
    pub default: silt_workspace::WorkspaceId,
    /// Whether the person made any; until then there's the one Default,
    /// syncing with the default server.
    pub saved: bool,
    pub workspaces: Vec<WorkspaceView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct WorkspaceView {
    pub workspace: silt_workspace::Workspace,
    /// Its org's policy, as last fetched.
    pub org_policy: Option<silt_workspace::OrgPolicy>,
    /// It's bound to an org whose policy hasn't been fetched yet, so no agent
    /// starts in it.
    pub waiting_for_policy: bool,
    /// Its own rules and its org's, together.
    pub effective: silt_workspace::Policy,
    pub projects: Vec<WorkspaceProject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct WorkspaceProject {
    pub repo: String,
    pub why: silt_workspace::Why,
}

/// This machine's settings for chats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(default)]
pub struct ChatSettings {
    /// Where chats' folders go; `$SILT_HOME/chats` when unset.
    pub dir: Option<PathBuf>,
    /// The permissions a new chat starts with: full access, as a chat's
    /// folder holds only what its agent wrote there.
    pub preset: Preset,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self { dir: None, preset: Preset::FullAccess }
    }
}

/// A mode, or one value of a setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AgentChoiceItem {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AgentModes {
    pub current: String,
    pub available: Vec<AgentChoiceItem>,
    /// The mode Plan first switches to.
    pub plan: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AgentSetting {
    pub id: String,
    pub name: String,
    /// `model`, `thought_level`, … as the agent tags it.
    pub category: Option<String>,
    pub current: String,
    pub choices: Vec<AgentChoiceItem>,
}

impl From<&silt_acp::Choice> for AgentChoiceItem {
    fn from(choice: &silt_acp::Choice) -> Self {
        Self { id: choice.id.clone(), name: choice.name.clone(), description: choice.description.clone() }
    }
}

impl From<&silt_acp::Modes> for AgentModes {
    fn from(modes: &silt_acp::Modes) -> Self {
        Self {
            current: modes.current.clone(),
            available: modes.available.iter().map(AgentChoiceItem::from).collect(),
            plan: silt_acp::plan_mode(modes).map(|mode| mode.id.clone()),
        }
    }
}

impl From<&silt_acp::Setting> for AgentSetting {
    fn from(setting: &silt_acp::Setting) -> Self {
        Self {
            id: setting.id.clone(),
            name: setting.name.clone(),
            category: setting.category.clone(),
            current: setting.current.clone(),
            choices: setting.choices.iter().map(AgentChoiceItem::from).collect(),
        }
    }
}

/// A command the agent offers, run by a prompt that starts with `/` and its
/// name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AgentCommand {
    pub name: String,
    pub description: String,
    /// What to write after it, when it takes anything.
    pub hint: Option<String>,
}

impl From<&silt_acp::SlashCommand> for AgentCommand {
    fn from(command: &silt_acp::SlashCommand) -> Self {
        Self { name: command.name.clone(), description: command.description.clone(), hint: command.hint.clone() }
    }
}

/// A prompt waiting for the thread's agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Queued {
    pub id: i64,
    pub author: String,
    pub text: String,
    pub created_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PermissionAsked {
    pub id: u64,
    pub title: String,
    /// Option ids and labels.
    pub options: Vec<(String, String)>,
    /// The plan it asks to approve, as Markdown, when it is the agent's
    /// own plan approval, as Claude's ExitPlanMode is.
    #[serde(default)]
    pub plan: Option<String>,
    /// The tool call it asks about, whose card shows it waiting rather than running.
    #[serde(default)]
    pub tool_call: Option<String>,
}

/// Tokens of a context window used, of its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ContextUse {
    #[cfg_attr(test, ts(type = "number"))]
    pub used: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AgentChoices {
    pub agents: Vec<AgentChoice>,
    pub clones: Vec<CloneInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AgentChoice {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Its modes as it last listed them here, for a new thread to choose
    /// from before it runs.
    #[serde(default)]
    pub modes: Option<AgentModes>,
    /// Its settings, such as its model, as it last listed them here.
    #[serde(default)]
    pub settings: Vec<AgentSetting>,
    /// Whether it takes images attached to a prompt; none until it's been
    /// seen here.
    #[serde(default)]
    pub takes_images: Option<bool>,
    /// Files other than text and images, likewise.
    #[serde(default)]
    pub takes_files: Option<bool>,
    /// The commands it offered when it last ran here.
    #[serde(default)]
    pub commands: Vec<AgentCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CloneInfo {
    pub repo: String,
    pub path: PathBuf,
}

/// A cloud session this daemon started (ADR 0040).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CloudLaunch {
    /// As claude.ai names it.
    pub id: String,
    pub url: String,
    pub repo: String,
    pub task: String,
    pub started_ms: i64,
    /// What claude warned about while starting it.
    pub warnings: Vec<String>,
    /// The thread recording it, once one does.
    pub thread: Option<String>,
}

/// What changed, for the browsers listening on `/api/events` (ADR 0021).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum BusEvent {
    /// The thread listing may have changed.
    Threads,
    /// The thread's family has new deltas; `deltas` is its log's length.
    Thread { thread: String, deltas: usize },
    /// The agent the web UI started in the thread; `None` once it's gone.
    Agent { thread: String, status: Option<Box<AgentStatus>> },
    /// The thread's queue of prompts changed.
    Queue { thread: String },
    /// A piece of what the agent says, thinks or a tool prints, as it comes.
    /// A tool's pieces name the tool call; one that `restart`s replaces
    /// what came before it rather than following it.
    AgentText {
        thread: String,
        kind: TextKind,
        text: String,
        #[serde(default)]
        tool_use_id: Option<String>,
        #[serde(default)]
        restart: bool,
    },
    /// The inbox may have changed.
    Inbox,
    /// Merge requests may have changed.
    Mrs,
    /// What an agent offers, its modes and settings, is newly known.
    Agents,
    /// An import from the web UI has gone through `done` of its `total`
    /// sessions.
    Importing { done: usize, total: usize },
    /// The socket fell behind and missed events: fetch everything again.
    Lagged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    Message,
    Thought,
    Tool,
}

impl BusEvent {
    /// The topic a socket subscribes to for it: `threads`, `thread:<id>`,
    /// `inbox`, `mrs` or `import`; none for what every socket hears.
    pub fn topic(&self) -> Option<String> {
        match self {
            Self::Threads => Some("threads".to_owned()),
            Self::Thread { thread, .. }
            | Self::Agent { thread, .. }
            | Self::AgentText { thread, .. }
            | Self::Queue { thread } => Some(format!("thread:{thread}")),
            Self::Inbox => Some("inbox".to_owned()),
            Self::Mrs => Some("mrs".to_owned()),
            Self::Importing { .. } => Some("import".to_owned()),
            // Rare and small: every socket hears it, as any page may show a composer.
            Self::Agents | Self::Lagged => None,
        }
    }
}

/// What a browser sends on `/api/events`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BusSubscription {
    #[serde(default)]
    pub subscribe: Vec<String>,
    #[serde(default)]
    pub unsubscribe: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("could not take the lease")]
    struct Outer(#[source] std::io::Error);

    /// The web UI reads these types; its declarations of them are written
    /// from here, so the two can't drift.
    #[test]
    fn the_web_ui_declares_the_api_types_as_they_are() {
        use ts_rs::TS;
        let config = ts_rs::Config::new().with_large_int("number");
        let declarations = [
            silt_proto::AgentActivity::decl(&config),
            silt_proto::AgentState::decl(&config),
            silt_proto::Capabilities::decl(&config),
            silt_proto::InboxAction::decl(&config),
            silt_proto::KindSetting::decl(&config),
            silt_proto::Startable::decl(&config),
            silt_proto::InboxRead::decl(&config),
            silt_proto::OrgChange::decl(&config),
            silt_proto::OrgInfo::decl(&config),
            silt_proto::OrgMember::decl(&config),
            silt_proto::OrgRole::decl(&config),
            silt_proto::TeamInfo::decl(&config),
            silt_proto::Presence::decl(&config),
            silt_proto::PresenceReply::decl(&config),
            silt_proto::ThreadStatus::decl(&config),
            silt_proto::Visibility::decl(&config),
            silt_proto::GitNotes::decl(&config),
            AgentChoice::decl(&config),
            AgentChoiceItem::decl(&config),
            AgentChoices::decl(&config),
            AgentModes::decl(&config),
            AgentSetting::decl(&config),
            AgentCommand::decl(&config),
            AgentStart::decl(&config),
            AgentStatus::decl(&config),
            QuestionAsked::decl(&config),
            SignInAsked::decl(&config),
            SignInChoice::decl(&config),
            SignInShown::decl(&config),
            ContextUse::decl(&config),
            QuestionFieldAsked::decl(&config),
            ChoiceAsked::decl(&config),
            BusEvent::decl(&config),
            BusSubscription::decl(&config),
            CloneInfo::decl(&config),
            CloudLaunch::decl(&config),
            CommentLocation::decl(&config),
            LineSpan::decl(&config),
            CommentOn::decl(&config),
            silt_proto::view::PendingAnchor::decl(&config),
            silt_proto::view::PlaceAnchors::decl(&config),
            CommentView::decl(&config),
            CommitView::decl(&config),
            silt_proto::view::DiffStat::decl(&config),
            silt_proto::view::FileTurns::decl(&config),
            silt_proto::view::TurnRun::decl(&config),
            silt_proto::view::Seen::decl(&config),
            silt_proto::view::FileSeen::decl(&config),
            silt_proto::view::ThreadSeenView::decl(&config),
            silt_proto::view::MarkSeen::decl(&config),
            silt_proto::view::FileAt::decl(&config),
            silt_proto::view::GraphQuery::decl(&config),
            silt_proto::view::GraphRow::decl(&config),
            DaemonStatus::decl(&config),
            silt_proto::view::RecoveryView::decl(&config),
            silt_proto::view::SignIn::decl(&config),
            DeviceAgent::decl(&config),
            DeviceChange::decl(&config),
            DeviceView::decl(&config),
            silt_proto::view::PhonePairing::decl(&config),
            NewLink::decl(&config),
            ThreadLink::decl(&config),
            FileEdit::decl(&config),
            FileTouch::decl(&config),
            HistoryEntry::decl(&config),
            ToolView::decl(&config),
            ToolOutcome::decl(&config),
            ImageView::decl(&config),
            silt_proto::view::ArtifactView::decl(&config),
            silt_proto::view::ArtifactContent::decl(&config),
            silt_proto::view::LinkedArtifact::decl(&config),
            UsageView::decl(&config),
            PlanStepView::decl(&config),
            TaskView::decl(&config),
            QuestionView::decl(&config),
            ChoiceView::decl(&config),
            AnswerView::decl(&config),
            HistoryHit::decl(&config),
            HistoryPage::decl(&config),
            InFlight::decl(&config),
            InboxEntry::decl(&config),
            LiveState::decl(&config),
            Mine::decl(&config),
            Mount::decl(&config),
            MrDetails::decl(&config),
            MrFile::decl(&config),
            MrGuide::decl(&config),
            MrSummary::decl(&config),
            PermissionAsked::decl(&config),
            Preset::decl(&config),
            Project::decl(&config),
            PromptAttachment::decl(&config),
            Queued::decl(&config),
            ReplyView::decl(&config),
            Pulled::decl(&config),
            Reverted::decl(&config),
            ReplicaDeltas::decl(&config),
            Resumed::decl(&config),
            SearchFilters::decl(&config),
            SearchResult::decl(&config),
            ShowLine::decl(&config),
            silt_proto::Snooze::decl(&config),
            StartOptions::decl(&config),
            ChatSettings::decl(&config),
            WorkspacesView::decl(&config),
            WorkspaceView::decl(&config),
            WorkspaceProject::decl(&config),
            silt_workspace::Workspace::decl(&config),
            silt_workspace::Workspaces::decl(&config),
            silt_workspace::ServerBinding::decl(&config),
            silt_workspace::MatchRule::decl(&config),
            silt_workspace::Defaults::decl(&config),
            silt_workspace::AgentDefaults::decl(&config),
            silt_workspace::Policy::decl(&config),
            silt_workspace::OrgPolicy::decl(&config),
            silt_workspace::Why::decl(&config),
            silt_workspace::WorkspaceId::decl(&config),
            silt_workspace::AgentKey::decl(&config),
            StateChange::decl(&config),
            TextKind::decl(&config),
            ThreadSummary::decl(&config),
            TurnInfo::decl(&config),
            FoundFolder::decl(&config),
            FoundSessions::decl(&config),
            VoiceDownload::decl(&config),
            VoiceEvent::decl(&config),
            VoiceStatus::decl(&config),
        ];
        let generated = format!(
            "// The daemon's API types, written by `UPDATE_API_TYPES=1 cargo test -p silt-daemon api_types`\n// from crates/silt-daemon/src/protocol.rs. Don't edit by hand.\n\n{}\n",
            declarations.iter().map(|declaration| format!("export {declaration}")).collect::<Vec<_>>().join("\n\n")
        );
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/api.generated.ts");
        if std::env::var_os("UPDATE_API_TYPES").is_some() {
            std::fs::write(&path, &generated).expect("writes the declarations");
        }
        let written = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            written == generated,
            "web/src/api.generated.ts is out of date: run `UPDATE_API_TYPES=1 cargo test -p silt-daemon api_types`"
        );
    }

    #[test]
    fn a_failure_says_what_caused_it() {
        let error = Outer(std::io::Error::other("connection refused"));
        let Response::Error { message } = Response::failure(&error) else {
            panic!("an error");
        };
        assert_eq!(message, "could not take the lease: connection refused");
    }
}
