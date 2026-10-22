// The daemon's API types, written by `UPDATE_API_TYPES=1 cargo test -p silt-daemon api_types`
// from crates/silt-daemon/src/protocol.rs. Don't edit by hand.

export type AgentActivity = { name: string, state: AgentState, 
/**
 * What it is saying right now, not recorded yet.
 */
streaming: string, };

export type AgentState = "starting" | "idle" | "working" | "awaiting_permission" | "awaiting_input" | "error" | "stopped";

export type Capabilities = { answer_permissions: boolean, send_prompts: boolean, run_agents: boolean, review: boolean, };

export type InboxAction = { "do": "permission", id: number, title: string, options: Array<[string, string]>, } | { "do": "plan" };

export type KindSetting = { kind: string, notify: boolean, };

export type Startable = { id: string, name: string, };

export type InboxRead = { ids: Array<number>, all: boolean, };

export type OrgChange = { "change": "create", org: string, } | { "change": "add_member", org: string, user: string, owner: boolean, } | { "change": "remove_member", org: string, user: string, } | { "change": "create_team", org: string, team: string, } | { "change": "remove_team", org: string, team: string, } | { "change": "add_to_team", org: string, team: string, user: string, } | { "change": "remove_from_team", org: string, team: string, user: string, } | { "change": "default_visibility", org: string, visibility: Visibility, } | { "change": "links", org: string, on: boolean, } | { "change": "move_project", repo: string, org: string, } | { "change": "project_settings", repo: string, name: string | null, default_agent: string | null, default_preset: string | null, };

export type OrgInfo = { name: string, role: OrgRole, 
/**
 * The caller owns it or administers the server.
 */
may_change: boolean, default_visibility: Visibility, members: Array<OrgMember>, teams: Array<TeamInfo>, projects: Array<string>, 
/**
 * Its threads may have read-only links.
 */
links: boolean, };

export type OrgMember = { user: string, role: OrgRole, };

export type OrgRole = "owner" | "member" | "admin";

export type TeamInfo = { name: string, members: Array<string>, };

export type Presence = { user: string, device: string, file: string | null, line: number | null, draft: string, agent: AgentActivity | null, 
/**
 * Milliseconds since they were last heard from.
 */
age_ms: number, };

export type PresenceReply = { 
/**
 * Everyone else on the thread.
 */
people: Array<Presence>, };

export type ThreadStatus = "active" | "idle" | "merged";

export type Visibility = string;

export type AgentChoice = { id: string, name: string, description: string, };

export type AgentChoiceItem = { id: string, name: string, description: string | null, };

export type AgentChoices = { agents: Array<AgentChoice>, clones: Array<CloneInfo>, };

export type AgentModes = { current: string, available: Array<AgentChoiceItem>, 
/**
 * The mode Plan first switches to.
 */
plan: string | null, };

export type AgentSetting = { id: string, name: string, 
/**
 * `model`, `thought_level`, … as the agent tags it.
 */
category: string | null, current: string, choices: Array<AgentChoiceItem>, };

export type AgentStart = { "kind": "resume", id: string, } | { "kind": "primed", id: string | null, prompt: string, };

export type AgentStatus = { agent: string, state: AgentState, 
/**
 * What the agent is saying or thinking right now, not recorded yet.
 */
streaming: string, thinking: string, permissions: Array<PermissionAsked>, error: string | null, 
/**
 * The agent's modes, when it has any.
 */
modes: AgentModes | null, 
/**
 * Its settings with a choice of values, such as its model.
 */
settings: Array<AgentSetting>, 
/**
 * What it takes attached to a prompt.
 */
takes_images: boolean, 
/**
 * Files other than text and images.
 */
takes_files: boolean, };

export type BusEvent = { "event": "threads" } | { "event": "thread", thread: string, deltas: number, } | { "event": "agent", thread: string, status: AgentStatus | null, } | { "event": "queue", thread: string, } | { "event": "agent_text", thread: string, kind: TextKind, text: string, tool_use_id: string | null, restart: boolean, } | { "event": "inbox" } | { "event": "mrs" } | { "event": "lagged" };

export type BusSubscription = { subscribe: Array<string>, unsubscribe: Array<string>, };

export type CloneInfo = { repo: string, path: string, };

export type CommentLocation = { "at": "entry", index: number, excerpt: string, } | { "at": "lines", path: string, start: number, end: number, outdated: boolean, } | { "at": "unknown" };

export type CommentOn = { "on": "entry", index: number, } | { "on": "lines", path: string, start: number, end: number, };

export type CommentView = { id: string, by: string, time_ms: number, text: string, resolved: boolean, location: CommentLocation, 
/**
 * The conversation under it, oldest first (ADR 0025).
 */
replies: Array<ReplyView>, };

export type CommitView = { commit: string, subject: string | null, branch: string | null, turn: number | null, time_ms: number, };

export type FileAt = { path: string, 
/**
 * The version shown, for a link that keeps showing it.
 */
at: string, 
/**
 * `None` for a binary file.
 */
text: string | null, };

export type GraphQuery = { 
/**
 * Only this project's threads, by its normalized remote.
 */
project: string | null, 
/**
 * Only this family's threads.
 */
family: string | null, 
/**
 * Only the person's own threads.
 */
mine: boolean, 
/**
 * Comma-separated kinds to show; all by default.
 */
kinds: string | null, 
/**
 * Settled threads too.
 */
settled: boolean, 
/**
 * Rows older than this, for the next page.
 */
before: number | null, limit: number | null, };

export type GraphRow = { 
/**
 * The delta it comes from, which no other row shares.
 */
id: string, 
/**
 * `turn`, `commit`, `fork`, `request` or `merge`.
 */
kind: string, thread: string, family: string, time_ms: number, 
/**
 * A turn's number, or the turn a commit came in.
 */
turn: number | null, 
/**
 * A turn's prompt, a commit's subject or id, a request's or merge's
 * title.
 */
text: string, 
/**
 * The other thread: a fork's source, a request's target, a merge's
 * fork.
 */
other: string | null, 
/**
 * A request's state: `open`, `approved` or `declined`.
 */
status: string | null, owner: string, agent: string | null, 
/**
 * Its thread is settled for the person.
 */
settled: boolean, 
/**
 * A commit's id, beside its subject in `text`.
 */
commit: string | null, };

export type DaemonStatus = { 
/**
 * The default server's URL, and this machine's device there.
 */
server: string | null, device: string | null, 
/**
 * Every server this machine is signed in to, the default first (ADR 0032).
 */
servers: Array<SignIn>, sync_error: string | null, threads: number, 
/**
 * Threads whose history here differs from the server's, so they no
 * longer push.
 */
diverged: Array<string>, 
/**
 * Recovery forks made since the daemon started.
 */
recovered: Array<string>, 
/**
 * The daemon's own version, as the web UI's status bar shows it.
 */
version: string, 
/**
 * The recovery forks behind `recovered`, by thread, for the web UI's
 * reconcile page.
 */
recoveries: Array<RecoveryView>, };

export type RecoveryView = { 
/**
 * The thread, now as the server has it.
 */
thread: string, 
/**
 * The fork of it holding this machine's work.
 */
recovery: string, time_ms: number, };

export type SignIn = { url: string, user: string, device: string, 
/**
 * This device's name there.
 */
name: string, sync_error: string | null, };

export type DeviceAgent = { thread: string, title: string | null, agent: string, state: AgentState, };

export type DeviceChange = { name: string | null, capabilities: Capabilities | null, };

export type DeviceView = { id: string, 
/**
 * `device`, a machine with a daemon, or `phone`, a paired browser.
 */
kind: string, name: string, created_ms: number, last_used_ms: number, 
/**
 * Heard from in the last minute and a half.
 */
online: boolean, 
/**
 * The device asking, when a device asks.
 */
this: boolean, capabilities: Capabilities, 
/**
 * The agents it runs, as it last reported them.
 */
agents: Array<DeviceAgent>, 
/**
 * The agents it could start, to continue a thread there.
 */
startable: Array<Startable>, };

export type PhonePairing = { code: string, 
/**
 * What the QR code holds: the page that signs the phone in.
 */
url: string, 
/**
 * The QR code, as an SVG image.
 */
qr_svg: string, expires_ms: number, };

export type NewLink = { days: number, frozen: boolean, };

export type ThreadLink = { id: number, created_ms: number, expires_ms: number, 
/**
 * Shows the thread as it was when made, not as it goes on.
 */
frozen: boolean, revoked: boolean, 
/**
 * Only when just made: the server keeps no copy of the token.
 */
url: string | null, };

export type FileEdit = { path: string, inserted: number, deleted: number, };

export type FileTouch = { thread: string, 
/**
 * The prompt of the turn the edits were made in.
 */
prompt: string | null, tool_use_id: string | null, by_agent: boolean, 
/**
 * Characters inserted and deleted.
 */
inserted: number, deleted: number, time_ms: number, };

export type HistoryEntry = { index: number, 
/**
 * The thread the entry was recorded in.
 */
thread: string, 
/**
 * `own`, `inherited` (before the fork) or `upstream` (brought in by a pull).
 */
origin: string, 
/**
 * `user`, `assistant`, `thinking`, `tool_call`, `tool_result`,
 * `compaction`, `merge` or `pull`.
 */
kind: string, text: string, time_ms: number, 
/**
 * A revert rewound it (ADR 0029).
 */
rewound: boolean, };

export type HistoryHit = { 
/**
 * The entry's index in the thread's history.
 */
index: number, kind: string, snippet: string, };

export type HistoryPage = { thread: string, total: number, entries: Array<HistoryEntry>, };

export type InFlight = { thread: string, owner: string, files: Array<string>, last_ms: number, };

export type InboxEntry = { id: number, kind: string, thread: string, actor: string, text: string, created_ms: number, read: boolean, action: InboxAction | null, notify: boolean, 
/**
 * The server it came from, on a machine signed in to several (ADR
 * 0032); ids are that server's.
 */
server: string | null, };

export type LiveState = { deltas: number, 
/**
 * The agent the web UI started in the thread, if any.
 */
agent: AgentStatus | null, 
/**
 * Which permission requests the daemon answers by itself.
 */
preset: Preset, };

export type Mine = { pinned: boolean, snoozed_until: number | null, 
/**
 * When it was snoozed, so activity since can wake it.
 */
snoozed_at: number | null, wake_on_activity: boolean, settled: boolean, 
/**
 * When they last opened it.
 */
seen_ms: number | null, };

export type Mount = { 
/**
 * The clone the checkout was made from.
 */
source: string, 
/**
 * Where this project's linked paths live on this machine.
 */
shared: string, };

export type MrDetails = { mr: MrSummary, 
/**
 * The fork's changes since the merge base.
 */
diff: string, 
/**
 * Places both the fork and the target changed since the merge base.
 */
conflicts: Array<string>, 
/**
 * The fork's conversation since it forked.
 */
conversation: Array<ShowLine>, 
/**
 * The files it changes, and whether this person has looked at each as
 * it is now (ADR 0024).
 */
files: Array<MrFile>, 
/**
 * A walk-through for reviewers, and who wrote it (ADR 0025).
 */
guide: MrGuide | null, };

export type MrFile = { path: string, seen: boolean, };

export type MrGuide = { text: string, by: string, };

export type MrSummary = { id: string, fork: string, target: string, title: string, summary: string, 
/**
 * `open`, `approved` or `declined: <reason>`.
 */
status: string, opened_ms: number, 
/**
 * What to change before it's taken, while it's open (ADR 0025).
 */
changes_requested: string | null, 
/**
 * The fork has moved on since changes were asked for.
 */
updated_since: boolean, };

export type PermissionAsked = { id: number, title: string, 
/**
 * Option ids and labels.
 */
options: Array<[string, string]>, };

export type Preset = "ask_before_commands" | "ask_for_everything" | "full_access";

export type Project = { 
/**
 * The normalized remote that names it.
 */
repo: string, name: string, org: string | null, origin_url: string | null, visibility: Visibility | null, default_agent: string | null, default_preset: string | null, 
/**
 * Its threads: those the person can see on the server, or those here.
 */
threads: number, 
/**
 * Merge requests still open into its threads, as this machine knows them.
 */
open_mrs: number, 
/**
 * This machine's clone of it.
 */
clone: string | null, 
/**
 * Where its new threads go from this machine (ADR 0032): a server's
 * URL, or `local`; none for the default server.
 */
home: string | null, 
/**
 * The server this record is from; none for a clone no server knows.
 */
server: string | null, };

export type PromptAttachment = { name: string, mime: string, 
/**
 * The content, in base64.
 */
data: string, };

export type Queued = { id: number, author: string, text: string, created_ms: number, };

export type ReplyView = { id: string, by: string, time_ms: number, text: string, };

export type Pulled = { thread: string, from: string, 
/**
 * Conversation entries the pull brought in.
 */
messages: number, files: Array<string>, };

export type Reverted = { thread: string, turn: number, 
/**
 * The files written back.
 */
files: Array<string>, };

export type ReplicaDeltas = { 
/**
 * How many deltas the family has here; ask for those after next time.
 */
total: number, 
/**
 * postcard-encoded `Vec<Delta>`, in base64.
 */
deltas: string, 
/**
 * Whether this machine's user may edit the thread.
 */
writable: boolean, };

export type Resumed = { thread: string, checkout: string, agent_cwd: string, 
/**
 * The agent session to continue, when the thread has one.
 */
session_id: string | null, mount: Mount | null, 
/**
 * The agent to run and how to start it.
 */
agent: string, start: AgentStart | null, };

export type SearchFilters = { repo: string | null, author: string | null, status: ThreadStatus | null, branch: string | null, file: string | null, since: number | null, until: number | null, };

export type SearchResult = { thread: string, kind: string, snippet: string, 
/**
 * Set when the server answered.
 */
owner: string | null, status: ThreadStatus | null, repo: string | null, 
/**
 * When it was said.
 */
time_ms: number | null, 
/**
 * The thread's turn it was said in, when the thread is here.
 */
turn: number | null, 
/**
 * The agent compacted its context after it.
 */
before_compaction: boolean, };

export type ShowLine = { "line": "entry", kind: string, text: string, } | { "line": "tool_call", id: string, name: string, input: string, } | { "line": "edits", tool_use_id: string | null, files: Array<FileEdit>, } | { "line": "compaction", summary: string | null, } | { "line": "turn", mark: string, } | { "line": "segment", label: string, } | { "line": "event", text: string, };

export type Snooze = { 
/**
 * None when not snoozed.
 */
until: number | null, 
/**
 * A new turn or merge request activity ends it early.
 */
wake_on_activity: boolean, };

export type StartOptions = { 
/**
 * A branch, tag or commit for a new thread's checkout to start from,
 * rather than what the clone has checked out.
 */
base: string | null, 
/**
 * A new thread works in the clone itself rather than a checkout of its
 * own.
 */
here: boolean, mode: string | null, preset: Preset | null, visibility: Visibility | null, 
/**
 * Sent once the agent is ready.
 */
prompt: string | null, 
/**
 * The first prompt is a Plan first one.
 */
plan: boolean, };

export type StateChange = { pinned: boolean | null, snooze: Snooze | null, settled: boolean | null, seen: boolean, };

export type TextKind = "message" | "thought" | "tool";

export type ThreadSummary = { id: string, repo: string, title: string | null, 
/**
 * The command of the agent that recorded its latest session.
 */
agent: string | null, created_ms: number, messages: number, edits: number, 
/**
 * `here` when checked out on this machine, `remote` when only known
 * from the server, `moved` when another device took it over.
 */
location: string, forked_from: string | null, 
/**
 * The owner's user name; empty when this machine isn't on a server.
 */
owner: string, collaborators: Array<string>, 
/**
 * Whether this machine's user may write into it (ADR 0014).
 */
writable: boolean, 
/**
 * How the agent the web UI last ran in it here stands (ADR 0022).
 */
agent_state: AgentState | null, 
/**
 * When something last happened in it.
 */
last_ms: number, 
/**
 * A merge request from it was approved.
 */
merged: boolean, 
/**
 * A merge request into it waits for this person's review.
 */
review: boolean, mine: Mine, 
/**
 * Its own visibility; none when it follows its project's or org's.
 */
visibility: Visibility | null, 
/**
 * The server it syncs with (ADR 0032); none without one.
 */
server: string | null, 
/**
 * It stays on this machine only, by its project's choice.
 */
local_only: boolean, };

export type TurnInfo = { 
/**
 * 1-based, as `silt diff --turn` counts them.
 */
number: number, 
/**
 * The history entry of the turn's prompt.
 */
entry: number | null, prompt: string, };
