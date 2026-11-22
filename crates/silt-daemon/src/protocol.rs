use std::path::PathBuf;

use serde::{Deserialize, Serialize};
pub use silt_proto::view::{
    AnswerView, ChoiceView, CommentLocation, CommentOn, CommentView, CommitView, CutView, DaemonStatus, DeviceAgent,
    DeviceChange, DeviceView, FileEdit, HistoryEntry, HistoryPage, ImageView, InboxEntry, LineSpan, Mine, MrDetails, MrFile,
    MrGuide, MrSummary, NewLink, PlanStepView, Project, QuestionView, ReplyView, SearchFilters, SearchResult, ShowLine,
    StateChange, TaskView, ThreadLink, ThreadSummary, ToolOutcome, ToolView, TurnInfo, UsageView,
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
    /// Runs `exe daemon restart` once no agent works in this daemon, as
    /// stopping it now would end their turns.
    RestartWhenIdle {
        exe: PathBuf,
    },
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
    /// Erases a Local only thread and its forks for good (ADR 0092); without
    /// `confirm`, says what would go.
    ThreadErase {
        thread: String,
        #[serde(default)]
        confirm: bool,
    },
    /// The local models here, the catalog's others, and what this machine
    /// can run (ADR 0092).
    Models,
    /// Starts downloading a model, one at a time: a catalog name,
    /// `owner/repo:file.gguf` on Hugging Face, or the absolute path of a GGUF
    /// file, which is used where it is. Answers with the model's name.
    ModelPull {
        source: String,
    },
    ModelRemove {
        model: String,
    },
    /// GGUF models on Hugging Face matching `query`, the most downloaded
    /// first; with an empty query, the most downloaded of all.
    ModelSearch {
        query: String,
    },
    /// A Hugging Face repository's quantizations, as this machine would run
    /// each.
    ModelFiles {
        repo: String,
    },
    /// Starts downloading llama.cpp's latest build for this machine.
    RuntimePull,
    /// Whether a newer Silt is out, and how an update is going (ADR 0100),
    /// for a page in the desktop app built from `desktop`.
    Update {
        #[serde(default)]
        desktop: Option<String>,
    },
    /// Checks for a newer Silt now.
    UpdateCheck,
    /// Starts updating this machine's Silt.
    UpdateStart,
    /// Whether an installed update waits for agents to finish before the
    /// daemon restarts.
    UpdateWaiting,
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
        /// Leave out the edits of other agents working in the same clone
        /// (ADR 0089).
        #[serde(default)]
        hide_others: bool,
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
        #[serde(default)]
        hide_others: bool,
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
        #[serde(default)]
        hide_others: bool,
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
        #[serde(default)]
        hide_others: bool,
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
        #[serde(default)]
        hide_others: bool,
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
        /// The version of the history the asker holds: the page starts
        /// earlier than `offset` where that changed since.
        #[serde(default)]
        known: Option<usize>,
        /// Long outputs and images left out, as a page of the web UI's
        /// carries them (`silt_proto::view::cut_for_page`).
        #[serde(default)]
        cut: bool,
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
        /// The asking agent's thread, left out; else the one checked out
        /// at `cwd`, which in a clone several agents share may be another's.
        #[serde(default)]
        thread: Option<String>,
    },
    /// The other agents present in `thread`'s project, as its agent hears
    /// of them (ADR 0090).
    Agents {
        thread: String,
    },
    /// A message from `from`'s agent to the agent of the thread `to` names,
    /// handed to it after its next tool call or with its next prompt.
    Message {
        from: String,
        to: String,
        text: String,
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
        #[serde(default)]
        hide_others: bool,
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
        artifact: Box<NewArtifact>,
    },
    /// What a run reads before making an artifact: the design system that
    /// applies, or the one named, and the others it may use (ADR 0071);
    /// only inside [`Request::Scoped`].
    ArtifactGuide {
        #[serde(default)]
        design_system: Option<String>,
    },
    /// The person's design systems and Silt's own, and a clone's when one
    /// is named, to pick a default from.
    DesignSystems {
        #[serde(default)]
        clone: Option<PathBuf>,
    },
    /// The themes for Silt's UI and the pick (ADR 0098).
    Themes,
    /// One theme's page: its tokens, checks, fonts and radii.
    ThemeDetail {
        id: String,
    },
    SetAppearance {
        settings: silt_proto::view::AppearanceSettings,
    },
    /// A face a theme of the person's ships, for `/theme-fonts/` (ADR 0099).
    ThemeFont {
        id: String,
        file: String,
    },
    /// A VS Code theme or a Zed family the person picked, saved as theirs.
    ImportTheme {
        text: String,
    },
    /// A copy of a theme saved as the person's, to edit.
    DuplicateTheme {
        from: String,
    },
    /// The CSS for the pick, or for `light` and `dark` in its place, with
    /// the pick's mode.
    ThemeCss {
        #[serde(default)]
        light: Option<String>,
        #[serde(default)]
        dark: Option<String>,
        /// Text and layout in place of what's saved, so a page changing it
        /// never gets the CSS of what was saved before (ADR 0107).
        #[serde(default)]
        text: Option<silt_theme::TextSettings>,
    },
    /// How this machine's web UI loads artifacts' libraries and fonts.
    ArtifactSettings,
    SetArtifactSettings {
        settings: ArtifactSettings,
    },
    /// A read or write of an artifact's page data (ADR 0072).
    PageData {
        thread: String,
        artifact: String,
        asked: silt_proto::page::PageRequest,
        /// Read as another person at this level, to check what the rules
        /// hide from them; writes are refused with it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        as_level: Option<silt_proto::page::Level>,
    },
    /// A version an HTML artifact's page saved itself as (ADR 0072); refused
    /// as a conflict when `base` isn't the latest version.
    PublishArtifact {
        thread: String,
        artifact: String,
        html: String,
        #[serde(default)]
        base: Option<u32>,
    },
    /// A page's question for this machine's agent (ADR 0072); its answer
    /// streams as `sample` bus events named `ask`. With no `artifact`, the
    /// person asks from elsewhere and their server checked the page.
    Sample {
        thread: String,
        #[serde(default)]
        artifact: Option<String>,
        #[serde(default)]
        ask: Option<String>,
        input: serde_json::Value,
        #[serde(default)]
        images: Vec<silt_proto::artifact::SampleImage>,
        #[serde(default)]
        tools: Vec<silt_proto::artifact::SampleTool>,
        #[serde(default)]
        model: silt_proto::artifact::SampleModel,
    },
    /// A doc or a deck as a PDF, printed from `html`, the page the browser
    /// drew of it, by this machine's headless browser; a deck's page says
    /// its own size.
    ArtifactPdf {
        thread: String,
        artifact: String,
        html: String,
    },
    /// Keeps a doc, a sheet or a whiteboard as it stands as its next
    /// version, from the person's Save version.
    SaveEditedVersion {
        thread: String,
        artifact: String,
        #[serde(default)]
        note: Option<String>,
    },
    /// The agent answering a page's question named `ask` calls one of the
    /// page's tools, from `silt sample-tools`; answered once the page runs it.
    SampleToolCall {
        ask: String,
        name: String,
        input: serde_json::Value,
    },
    /// Stops the page's question named `ask` while it runs.
    StopSample {
        thread: String,
        artifact: String,
        ask: String,
    },
    /// The page's answer to a call of one of its tools.
    SampleToolResult {
        thread: String,
        artifact: String,
        ask: String,
        call: u32,
        result: SampleToolResult,
    },
    /// The files people added to an artifact's page (ADR 0074).
    PageAssets {
        thread: String,
        artifact: String,
        asked: silt_proto::page::AssetRequest,
    },
    /// Where a page's room is relayed: the server the thread syncs with, so
    /// the room spans every machine viewing it, or none to relay it here.
    RoomRoute {
        thread: String,
    },
    /// A view joins a page's room (ADR 0081); the answer is who hears what.
    RoomJoin {
        key: silt_proto::rooms::RoomKey,
        peer: String,
    },
    RoomSend {
        key: silt_proto::rooms::RoomKey,
        peer: String,
        sent: silt_proto::rooms::Sent,
    },
    RoomLeave {
        key: silt_proto::rooms::RoomKey,
        peer: String,
    },
    /// The thread's agent emits to a page's room and reads what was said.
    RoomAgent {
        thread: String,
        artifact: String,
        #[serde(default)]
        emit: Option<(String, serde_json::Value)>,
        /// The agent's presence, as a view's: what it's on, say.
        #[serde(default)]
        presence: Option<serde_json::Value>,
    },
    /// The MCP servers this machine lists for pages (ADR 0080).
    PageConnectors,
    SetPageConnectors {
        connectors: Vec<crate::connectors::PageConnector>,
    },
    /// Adds the servers Claude Code lists, every tool off.
    ImportPageConnectors,
    /// A page connector's tools, started if it isn't running.
    ConnectorTools {
        name: String,
    },
    /// Keeps a header's value for an HTTP page connector in the keyring, or
    /// forgets it.
    SetConnectorSecret {
        name: String,
        header: String,
        #[serde(default)]
        value: Option<String>,
    },
    /// A page's call of a connector's tool, or its description (ADR 0080).
    PageMcp {
        thread: String,
        artifact: String,
        asked: crate::connectors::McpAsk,
    },
    /// A doc's text, or edits of it (ADR 0077).
    DocData {
        thread: String,
        artifact: String,
        asked: silt_proto::edited::DocRequest,
    },
    /// A sheet's cells and values, or changes to it (ADR 0077).
    SheetData {
        thread: String,
        artifact: String,
        asked: silt_proto::edited::SheetRequest,
    },
    /// The thread's files, for a page that declares `files` (ADR 0078).
    PageFiles {
        thread: String,
        artifact: String,
        asked: silt_proto::page::FilesRequest,
    },
    /// One added file's bytes, for the page's frame.
    PageAsset {
        thread: String,
        artifact: String,
        asset: String,
    },
    /// A file beside a version's page, for its frame (ADR 0074).
    ArtifactFile {
        thread: String,
        id: String,
        #[serde(default)]
        version: Option<u32>,
        path: String,
    },
    /// Every artifact in the threads here, for the Artifacts page (ADR 0075).
    ArtifactListing,
    /// An artifact's thumbnail as a JPEG, rendered the first time it's asked
    /// for where a headless browser is installed.
    ArtifactThumbnail {
        thread: String,
        artifact: String,
    },
    /// Keeps a thumbnail rendered off the event loop.
    KeepThumbnail {
        thread: String,
        artifact: String,
        version: u32,
        jpeg: String,
    },
    PinArtifact {
        thread: String,
        artifact: String,
        pinned: bool,
    },
    /// Hides an artifact for everyone, or shows it again; the owner's to do.
    ArchiveArtifact {
        thread: String,
        artifact: String,
        archived: bool,
    },
    /// Who views an artifact's page, as its `user` capability answers.
    PageViewer {
        thread: String,
    },
    /// What the person allowed or refused an artifact's page (ADR 0072).
    ArtifactGrants {
        thread: String,
        artifact: String,
    },
    SetArtifactGrant {
        thread: String,
        artifact: String,
        capability: String,
        #[serde(default)]
        state: Option<String>,
        /// The version the viewer saw, whose declaration the grant is for;
        /// the latest when none.
        #[serde(default)]
        version: Option<u32>,
    },
    /// The design system picked for each project on this machine.
    ProjectDesignSystems,
    /// Every design system here, grouped by project, with what uses each and
    /// the ones threads are still making (ADR 0088).
    DesignCatalog,
    /// One design system's page: its rules, tokens with their values,
    /// components, cover, assets and the CSS a frame gets. A thread names
    /// the folder a system it's making is in.
    DesignSystem {
        source: DesignSource,
        name: String,
        #[serde(default)]
        clone: Option<PathBuf>,
        #[serde(default)]
        thread: Option<String>,
    },
    /// One of a design system's assets, by its path under `assets/`.
    DesignAsset {
        source: DesignSource,
        name: String,
        #[serde(default)]
        clone: Option<PathBuf>,
        #[serde(default)]
        thread: Option<String>,
        path: String,
    },
    /// Starts a thread whose agent makes a design system, or changes one;
    /// answers the thread.
    DesignStart(DesignStart),
    /// The design system a thread was started to make or change, or wrote
    /// since it loaded the design-system skill, if any.
    ThreadDesignSystem {
        thread: String,
    },
    /// A thread loaded the design-system skill; a run's own thread when
    /// inside [`Request::Scoped`].
    DesignSkillUsed {
        thread: String,
    },
    /// Copies a system a thread's folder holds, a chat's, to the person's
    /// own; answers the copy's row.
    KeepDesignSystem {
        thread: String,
        name: String,
    },
    /// Picks a project's design system on this machine, or drops the pick.
    SetProjectDesignSystem {
        repo: String,
        #[serde(default)]
        design_system: Option<String>,
    },
    /// A version of an artifact of the run's thread, the latest by default,
    /// for the agent to revise; only inside [`Request::Scoped`].
    ArtifactRead {
        id: String,
        #[serde(default)]
        version: Option<u32>,
    },
    /// An artifact's content, by the blob a history entry of the thread names.
    ArtifactContent {
        thread: String,
        blob: String,
    },
    /// A version of one of the thread's artifacts, the latest when `version`
    /// is none, for its frame (ADR 0074).
    ArtifactVersion {
        thread: String,
        id: String,
        #[serde(default)]
        version: Option<u32>,
    },
    /// The tools of the plugins on in the run's workspace (ADR 0093); only
    /// inside [`Request::Scoped`].
    PluginTools,
    /// Calls a plugin's tool for the run; only inside [`Request::Scoped`].
    PluginCall {
        tool: String,
        input: serde_json::Value,
    },
    /// Waits on an approval the run's call made; only inside
    /// [`Request::Scoped`].
    PluginWait {
        approval: u64,
    },
    /// This machine's plugins.
    Plugins,
    /// Turns a plugin on or off in a workspace.
    PluginTurn {
        id: String,
        workspace: String,
        on: bool,
    },
    PluginRemove {
        name: String,
    },
    /// The person's rule for one of a plugin's actions; `None` takes it away.
    PluginRule {
        id: String,
        action: String,
        rule: Option<silt_plugin::Rule>,
        #[serde(default)]
        field: Option<String>,
        #[serde(default)]
        pattern: Option<String>,
    },
    /// One of a plugin's settings; `null` resets it.
    PluginSet {
        id: String,
        key: String,
        value: serde_json::Value,
        #[serde(default)]
        environment: Option<String>,
        #[serde(default)]
        workspace: Option<String>,
    },
    /// The patterns that pick one of a plugin's environments; none removes it.
    PluginEnvironment {
        id: String,
        name: String,
        patterns: Vec<String>,
    },
    /// How a plugin is doing: its build, its tools and its calls.
    PluginActivity {
        id: String,
    },
    /// Trusts a publisher, or stops trusting it.
    PluginTrust {
        publisher: String,
        trusted: bool,
    },
    /// Plugin calls waiting for the person, a thread's alone when one is
    /// named; with `all`, those answered too.
    Approvals {
        #[serde(default)]
        thread: Option<String>,
        #[serde(default)]
        all: bool,
    },
    /// The person's answer to an approval, naming the hash of the request
    /// they were shown, and what else it allows from then on.
    ApprovalAnswer {
        id: u64,
        approve: bool,
        hash: String,
        #[serde(default)]
        grant: Option<crate::plugins::GrantScope>,
    },
    /// Binds a terminal run's token, made before its new thread was, to the
    /// thread (ADR 0093).
    BindRunToken {
        token: String,
        thread: String,
    },
    /// Holds `devices` for the thread's sandboxes, or hands back those it
    /// holds already (ADR 0093).
    RelayDevices {
        thread: String,
        devices: Vec<String>,
    },
    /// Lets go of a device relayed to the thread, ending its session.
    ReleaseDevice {
        thread: String,
        device: String,
    },
    /// Ends a grant: calls it covered ask again.
    GrantRevoke {
        id: u64,
    },
    /// Takes a folder back from a thread's agents (ADR 0102).
    FolderRevoke {
        thread: String,
        path: String,
    },
    /// What reached a thread's sandboxes through Silt: plugin calls,
    /// grants, and what the gateway refused (ADR 0093).
    Access {
        thread: String,
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
    /// How much of its plan `agent` has used, with the sign-in it would run
    /// with in `thread`'s folder, else this machine's (ADR 0095).
    AgentUsage {
        agent: String,
        #[serde(default)]
        thread: Option<String>,
    },
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
    /// How phones on the local network reach this machine (ADR 0112).
    LanStatus,
    /// Lets phones on the local network reach this machine, or stops.
    LanTurn {
        on: bool,
    },
    /// A one-time code a phone on the network pairs with, and what it may do.
    LanPair {
        capabilities: silt_proto::Capabilities,
    },
    /// The person's answer to a phone asking to pair.
    LanAnswer {
        ask: String,
        allow: bool,
    },
    /// Renames a phone paired over the network or changes what it may do.
    LanPhoneChange {
        phone: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        capabilities: Option<silt_proto::Capabilities>,
    },
    LanPhoneRevoke {
        phone: String,
    },
    /// Signs every phone out and makes a new certificate authority.
    LanForget,
    /// Where `phone`'s pushes go; `None` stops them.
    LanPush {
        phone: String,
        #[serde(default)]
        subscription: Option<crate::lan::Subscription>,
    },
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
    /// The folders in `dir`, for completing a path as it's typed.
    Subfolders {
        dir: PathBuf,
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The design system to style an HTML version with, or `none`.
    #[serde(default)]
    pub design_system: Option<String>,
    /// The runtime contract an HTML version's page is written for, or
    /// `latest`; by default its last version's.
    #[serde(default)]
    pub contract: Option<String>,
    /// Changes to the latest version's source, in place of `content` or
    /// `path` (ADR 0073).
    #[serde(default)]
    pub edits: Vec<silt_proto::artifact::Edit>,
    /// What this version changed.
    #[serde(default)]
    pub note: Option<String>,
    /// What the artifact is, for its card; a version without one keeps the
    /// last.
    #[serde(default)]
    pub description: Option<String>,
    /// The capabilities the page may use (ADR 0072): none keeps the last
    /// version's, `{}` clears them, and an object replaces them all.
    #[serde(default)]
    pub capabilities: Option<serde_json::Map<String, serde_json::Value>>,
    /// Files beside an HTML version's page, by path (ADR 0074): each one
    /// added or replaced, or removed by `null`; the rest of the last
    /// version's are kept.
    #[serde(default)]
    pub files: std::collections::BTreeMap<String, Option<NewFile>>,
    /// An RFC 6902 JSON Patch of the latest version of a type's content
    /// (ADR 0076), in place of content, a path or edits.
    #[serde(default)]
    pub patch: Option<serde_json::Value>,
}

/// A file of a version, as text, as base64, or read from a file of the
/// run's checkout.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewFile {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub base64: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

/// The artifact a [`Request::PutArtifact`] kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactPut {
    pub id: String,
    pub version: u32,
    /// The design system an HTML version was styled with.
    #[serde(default)]
    pub design_system: Option<String>,
    /// What its tokens couldn't use, for the agent to fix.
    #[serde(default)]
    pub dropped: Vec<String>,
    /// The system the last version or a default named, which isn't on this
    /// machine, so the version has none (ADR 0087).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
}

/// How artifacts' frames load libraries and fonts (ADR 0071).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ArtifactSettings {
    /// Through Silt's vendor route, which fetches and keeps each file, so
    /// the viewer's browser never contacts a CDN; else from the CDNs.
    #[serde(default)]
    pub proxy: bool,
}

/// What [`Request::ArtifactRead`] answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRead {
    pub id: String,
    pub version: u32,
    /// The latest version of the artifact, which may be later than this.
    pub latest: u32,
    pub title: String,
    pub kind: String,
    pub media_type: String,
    #[serde(default)]
    pub design_system: Option<String>,
    /// Base64.
    pub data: String,
    /// The CSS its design system compiled to, which its frame is given.
    #[serde(default)]
    pub style: Option<String>,
}

/// Where a design system comes from (ADR 0071).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum DesignSource {
    /// The repository's `.silt/design-systems/`.
    Project,
    /// The person's, under Silt's data dir.
    Personal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignSystemView {
    /// What `design_system` takes.
    pub name: String,
    /// The name its tokens give it.
    pub title: String,
    pub source: DesignSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readme: Option<String>,
    /// Its custom properties and type classes, by family.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tokens: std::collections::BTreeMap<String, Vec<String>>,
    /// Its components, given with its README (ADR 0086).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<DesignComponentView>,
    /// Its folder and its assets' paths under `assets/`, given with its
    /// README (ADR 0088).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<String>,
}

/// One of a design system's components: its example and the note it opens with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignComponentView {
    pub name: String,
    pub about: String,
    pub html: String,
    /// Its guidelines, `components/<name>.md` (ADR 0088).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<String>,
}

/// Every design system on this machine, for Settings (ADR 0088).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignCatalog {
    /// Each project's own, each chat's, then the person's.
    pub groups: Vec<DesignGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignGroup {
    /// The project whose repository keeps them; none for a chat's or the
    /// person's own.
    pub project: Option<DesignProject>,
    /// The chat whose folder keeps them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<DesignChat>,
    pub systems: Vec<DesignSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignChat {
    pub thread: String,
    pub title: String,
    pub folder: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignProject {
    pub repo: String,
    pub name: String,
    pub clone: String,
}

/// A design system as a row: enough to tell it apart and open it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignSummary {
    pub name: String,
    pub title: String,
    pub source: DesignSource,
    /// Its folder.
    pub dir: String,
    /// A few of its colours, in its first theme.
    pub swatches: Vec<String>,
    pub colors: u32,
    pub type_styles: u32,
    pub components: u32,
    /// What picks it as a default: "repo's default", "picked for web on
    /// this machine", "default for Work".
    pub uses: Vec<String>,
    /// The thread still making or changing it, when one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// It's only in that thread's folder so far.
    #[serde(default)]
    pub unmerged: bool,
    /// It's in that thread's folder, a chat's, which nothing merges.
    #[serde(default)]
    pub chat: bool,
}

/// One design system's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignDetail {
    pub summary: DesignSummary,
    pub readme: String,
    /// Where it was made from, as its tokens' `meta.source` says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made_from: Option<String>,
    pub themes: Vec<DesignTheme>,
    /// Its tokens with their values per theme, colours first.
    pub tokens: Vec<DesignToken>,
    pub type_styles: Vec<DesignTypeStyle>,
    /// The CSS its tokens and components compile to, as a frame gets it.
    pub css: String,
    pub components: Vec<DesignComponentView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    pub assets: Vec<DesignAssetView>,
    /// Every file in its folder, by path.
    pub files: Vec<String>,
    /// What its files hold that Silt left out, and why.
    pub dropped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignTheme {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignToken {
    pub family: String,
    pub name: String,
    /// Its value by theme id; one value holds in every theme under `*`.
    pub values: std::collections::BTreeMap<String, String>,
    pub usage: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignTypeStyle {
    pub name: String,
    pub group: String,
    /// Its font stack, from its family.
    pub family: String,
    pub font_size: String,
    pub line_height: Option<String>,
    pub font_weight: Option<String>,
    pub letter_spacing: Option<String>,
    pub usage: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignAssetView {
    pub path: String,
    pub media_type: String,
    pub size: u32,
}

/// One asset's bytes, in base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignAssetData {
    pub media_type: String,
    pub data: String,
}

/// What a design system is made from (ADR 0088).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum DesignFrom {
    /// A project's code.
    Code,
    /// A website, a brand guide or screenshots.
    Website,
    /// A description.
    Description,
}

/// Where a new design system is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum DesignKeep {
    /// The project's repository, committed.
    Repo,
    /// The person's own, synced to their devices.
    Personal,
}

/// Starts an agent making a design system, or changing one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DesignStart {
    /// The system's name as people read it, such as "Harbor"; its folder's
    /// name comes from it.
    pub title: String,
    pub agent: String,
    pub keep: DesignKeep,
    /// The project: the one built from, the one whose repository keeps it,
    /// or both.
    #[serde(default)]
    pub clone: Option<PathBuf>,
    #[serde(default)]
    pub from: Option<DesignFrom>,
    /// The website's address, for one made from a website.
    #[serde(default)]
    pub link: Option<String>,
    /// What to make it from, for one made from a description, or anything
    /// else the person adds.
    #[serde(default)]
    pub notes: Option<String>,
    /// A brand guide, screenshots or anything else sent with the prompt.
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
    /// What to change in the system `name`, which exists; a new one when
    /// unset.
    #[serde(default)]
    pub change: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// The design system a thread was started to make or change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ThreadDesign {
    pub name: String,
    pub title: String,
    pub keep: DesignKeep,
    pub changing: bool,
    /// What's written so far; none until the folder has its tokens.
    pub detail: Option<DesignDetail>,
}

/// What [`Request::ArtifactGuide`] answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactGuide {
    /// The system that applies, with its README and tokens.
    pub system: Option<DesignSystemView>,
    /// Why it applies: asked for, the project's default, the workspace's.
    pub why: Option<String>,
    pub systems: Vec<DesignSystemView>,
    /// A default that names a system this machine doesn't have, and why it
    /// applied, so none does (ADR 0087).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<(String, String)>,
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

/// How much of its plan an agent has used (ADR 0095).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AgentUsage {
    /// The agent's name, such as Claude Code.
    pub agent: String,
    /// Its plan's name, such as Max, where the agent says.
    pub plan: Option<String>,
    pub limits: Vec<UsageLimit>,
    /// Where the person sees all of it.
    pub page: String,
    /// Why there are no limits to show, such as an API key or no sign-in.
    pub unavailable: Option<String>,
    /// Read from what the agent last recorded, as asking failed.
    pub recorded: bool,
    pub read_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct UsageLimit {
    /// Such as 5-hour limit or Premium requests.
    pub name: String,
    /// 0 to 100.
    pub used_percent: f64,
    pub resets_at_ms: Option<i64>,
}

/// The local models on this machine and what it can run (ADR 0092).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LocalModels {
    /// The `llama-server` models run with, if one was found.
    pub server: Option<String>,
    pub devices: Vec<ModelDevice>,
    /// Bytes a model may use here: the devices', else the machine's memory.
    pub room: u64,
    pub models: Vec<LocalModel>,
    /// A model's download, or llama.cpp's (`llama.cpp`), while it runs.
    pub pull: Option<ModelPull>,
    /// Why the last download failed.
    pub failed: Option<ModelFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ModelDevice {
    pub name: String,
    pub description: String,
    pub total: u64,
    pub free: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LocalModel {
    pub id: String,
    pub name: String,
    /// Where it came from: `owner/repo:file` or a file here.
    pub source: String,
    pub size: u64,
    pub context: u32,
    pub fit: ModelFit,
    /// The file `llama-server` loads.
    pub path: PathBuf,
}

/// A model repository a search on Hugging Face found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FoundModel {
    /// `owner/name`.
    pub repo: String,
    /// Downloads in the last month.
    pub downloads: u64,
    pub updated: Option<String>,
}

/// A repository's quantizations, as this machine would run each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ModelFiles {
    pub repo: String,
    pub architecture: Option<String>,
    /// The most context the model was trained for.
    pub context_length: Option<u64>,
    /// Smallest first.
    pub quantizations: Vec<ModelQuantization>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ModelQuantization {
    /// What a pull takes: `owner/repo:file.gguf`.
    pub source: String,
    /// Its kind, such as `Q4_K_M`.
    pub label: String,
    pub size: u64,
    /// Files it's split across.
    pub parts: u32,
    pub fit: ModelFit,
    /// The context it would start with here.
    pub context: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModelFit {
    Fits,
    /// It may load, with less context or nothing else running.
    Tight,
    TooBig,
}

impl From<silt_llm::Fit> for ModelFit {
    fn from(fit: silt_llm::Fit) -> Self {
        match fit {
            silt_llm::Fit::Fits => Self::Fits,
            silt_llm::Fit::Tight => Self::Tight,
            silt_llm::Fit::TooBig => Self::TooBig,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ModelPull {
    pub id: String,
    pub done: u64,
    /// Zero until the files' sizes are known.
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ModelFailure {
    pub id: String,
    pub reason: String,
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

/// A folder in the one a typed path is in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Subfolder {
    pub name: String,
    /// Whether it holds a git repository of its own.
    pub git: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FoundSessions {
    pub agent: String,
    /// The agent's id, for its mark.
    pub agent_id: String,
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
    /// The prompt the agent predicts the person sends next, which the
    /// composer offers while it's empty.
    #[serde(default)]
    pub suggestion: Option<String>,
    /// The calls whose commands still run in the background, which Stop
    /// ends.
    #[serde(default)]
    pub stoppable: Vec<String>,
    /// What Silt's sandbox lets it reach, when it runs in one (ADR 0093).
    #[serde(default)]
    pub sandbox: Option<silt_workspace::Reach>,
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
    /// What can make Silt's sandbox, and whether it can on this machine.
    pub backends: Vec<BackendView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BackendView {
    pub backend: silt_workspace::Backend,
    /// Why it can't make one here, if it can't.
    pub unavailable: Option<String>,
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
    /// The answer so far to a page's `sample` question.
    Sample { ask: String, text: String },
    /// The agent answering a page's question calls one of the tools the
    /// page gave it, for the page to run and answer.
    SampleTool {
        ask: String,
        call: u32,
        name: String,
        #[cfg_attr(test, ts(type = "unknown"))]
        input: serde_json::Value,
    },
    /// What the agent said to a room, for the web server to deliver; never
    /// sent to browsers.
    Room {
        #[cfg_attr(test, ts(type = "unknown"))]
        deliveries: serde_json::Value,
    },
    /// Plugin calls waiting for the person changed.
    Approvals,
    /// The themes or the pick changed: a file in `themes/` was saved, or
    /// another page picked (ADR 0098).
    Appearance,
    /// How phones on the local network reach this machine changed, or one
    /// asks to pair (ADR 0112).
    Lan,
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
            Self::Sample { ask, .. } | Self::SampleTool { ask, .. } => Some(format!("sample:{ask}")),
            Self::Room { .. } => Some("rooms".to_owned()),
            // Rare and small: every socket hears it, as any page may show a composer.
            Self::Agents | Self::Approvals | Self::Appearance | Self::Lan | Self::Lagged => None,
        }
    }
}

/// How a page's tool answered: its value, or why it failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleToolResult {
    pub ok: bool,
    #[serde(default)]
    pub value: serde_json::Value,
    #[serde(default)]
    pub error: String,
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
            crate::lan::LanView::decl(&config),
            crate::lan::LanState::decl(&config),
            crate::lan::LanAsk::decl(&config),
            crate::lan::LanPhone::decl(&config),
            crate::lan::LanPairing::decl(&config),
            crate::lan::Subscription::decl(&config),
            crate::lan::SubscriptionKeys::decl(&config),
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
            silt_access::DataClass::decl(&config),
            crate::plugins::Outcome::decl(&config),
            crate::plugins::ApprovalState::decl(&config),
            crate::plugins::ApprovalView::decl(&config),
            silt_plugin::Card::decl(&config),
            silt_plugin::Block::decl(&config),
            silt_plugin::Item::decl(&config),
            silt_plugin::FileChange::decl(&config),
            silt_plugin::Tone::decl(&config),
            silt_plugin::Effect::decl(&config),
            silt_plugin::Channel::decl(&config),
            silt_plugin::ScopedRule::decl(&config),
            silt_plugin::Rule::decl(&config),
            silt_plugin::RuleSource::decl(&config),
            silt_plugin::Glob::decl(&config),
            crate::plugins::GrantScope::decl(&config),
            crate::plugins::CallView::decl(&config),
            crate::plugins::GrantView::decl(&config),
            crate::plugins::AccessView::decl(&config),
            crate::plugins::ThreadCall::decl(&config),
            crate::plugins::PluginActivity::decl(&config),
            crate::plugins::installed::Installed::decl(&config),
            crate::plugins::installed::PluginView::decl(&config),
            crate::plugins::updates::GitUpdate::decl(&config),
            crate::plugins::install::Preview::decl(&config),
            silt_plugin::Manifest::decl(&config),
            silt_plugin::Build::decl(&config),
            silt_plugin::Capability::decl(&config),
            silt_plugin::ActionSpec::decl(&config),
            silt_plugin::ActionKind::decl(&config),
            silt_plugin::Environments::decl(&config),
            silt_plugin::SettingSpec::decl(&config),
            silt_plugin::SettingKind::decl(&config),
            silt_plugin::SettingScope::decl(&config),
            silt_plugin::CredentialSpec::decl(&config),
            silt_plugin::Send::decl(&config),
            silt_plugin::Docs::decl(&config),
            crate::plugins::installed::PluginSource::decl(&config),
            crate::plugins::FolderView::decl(&config),
            crate::plugins::NetworkRefusal::decl(&config),
            crate::plugins::RelayView::decl(&config),
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
            silt_proto::view::SharedEdits::decl(&config),
            FileTouch::decl(&config),
            HistoryEntry::decl(&config),
            ToolView::decl(&config),
            ToolOutcome::decl(&config),
            ImageView::decl(&config),
            CutView::decl(&config),
            silt_proto::view::ArtifactView::decl(&config),
            silt_proto::artifact::ArtifactFile::decl(&config),
            silt_proto::view::ArtifactListed::decl(&config),
            crate::connectors::PageConnector::decl(&config),
            crate::connectors::ConnectorTool::decl(&config),
            DesignSource::decl(&config),
            ArtifactSettings::decl(&config),
            silt_proto::view::AppearanceMode::decl(&config),
            silt_proto::view::AppearanceSettings::decl(&config),
            silt_theme::text::TextSettings::decl(&config),
            silt_theme::text::Zoom::decl(&config),
            silt_theme::text::UiSize::decl(&config),
            silt_theme::text::ChatSize::decl(&config),
            silt_theme::text::CodeSize::decl(&config),
            silt_theme::text::TabWidth::decl(&config),
            silt_theme::text::FontChoice::decl(&config),
            silt_theme::text::Density::decl(&config),
            silt_theme::text::LineHeight::decl(&config),
            silt_theme::text::ReadingWidth::decl(&config),
            silt_theme::text::DiffLayout::decl(&config),
            silt_theme::text::Detail::decl(&config),
            silt_theme::text::Grouping::decl(&config),
            silt_theme::text::Motion::decl(&config),
            silt_proto::view::ThemeSource::decl(&config),
            silt_proto::view::ThemeSummary::decl(&config),
            silt_proto::view::ThemeProblem::decl(&config),
            silt_proto::view::ThemeCatalog::decl(&config),
            silt_proto::view::ThemeColorView::decl(&config),
            silt_proto::view::ThemeSyntaxView::decl(&config),
            silt_proto::view::ThemeCheckView::decl(&config),
            silt_proto::view::ThemeDetail::decl(&config),
            silt_proto::view::ThemeCss::decl(&config),
            silt_proto::view::ThemeSaved::decl(&config),
            DesignSystemView::decl(&config),
            DesignComponentView::decl(&config),
            DesignCatalog::decl(&config),
            DesignGroup::decl(&config),
            DesignProject::decl(&config),
            DesignChat::decl(&config),
            DesignSummary::decl(&config),
            DesignDetail::decl(&config),
            DesignTheme::decl(&config),
            DesignToken::decl(&config),
            DesignTypeStyle::decl(&config),
            DesignAssetView::decl(&config),
            DesignAssetData::decl(&config),
            DesignFrom::decl(&config),
            DesignKeep::decl(&config),
            DesignStart::decl(&config),
            ThreadDesign::decl(&config),
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
            BackendView::decl(&config),
            WorkspaceProject::decl(&config),
            silt_workspace::Workspace::decl(&config),
            silt_workspace::FolderRule::decl(&config),
            silt_workspace::PluginPolicy::decl(&config),
            silt_workspace::OrgPlugin::decl(&config),
            silt_workspace::OrgPluginRule::decl(&config),
            silt_workspace::FolderAnswer::decl(&config),
            silt_workspace::Workspaces::decl(&config),
            silt_workspace::ServerBinding::decl(&config),
            silt_workspace::MatchRule::decl(&config),
            silt_workspace::Defaults::decl(&config),
            silt_workspace::AgentDefaults::decl(&config),
            silt_workspace::Policy::decl(&config),
            silt_workspace::Network::decl(&config),
            silt_workspace::Backend::decl(&config),
            silt_workspace::Devices::decl(&config),
            silt_workspace::Extra::decl(&config),
            silt_workspace::Reach::decl(&config),
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
            Subfolder::decl(&config),
            VoiceDownload::decl(&config),
            VoiceEvent::decl(&config),
            VoiceStatus::decl(&config),
            LocalModels::decl(&config),
            AgentUsage::decl(&config),
            UsageLimit::decl(&config),
            ModelDevice::decl(&config),
            LocalModel::decl(&config),
            FoundModel::decl(&config),
            ModelFiles::decl(&config),
            ModelQuantization::decl(&config),
            ModelFit::decl(&config),
            ModelPull::decl(&config),
            ModelFailure::decl(&config),
            silt_update::UpdateView::decl(&config),
            silt_update::BuildView::decl(&config),
            silt_update::Channel::decl(&config),
            silt_update::Availability::decl(&config),
            silt_update::NewCommit::decl(&config),
            silt_update::Commit::decl(&config),
            silt_update::OffReason::decl(&config),
            silt_update::RunView::decl(&config),
            silt_update::StepView::decl(&config),
            silt_update::StepStatus::decl(&config),
            silt_update::Phase::decl(&config),
            silt_update::Outcome::decl(&config),
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
