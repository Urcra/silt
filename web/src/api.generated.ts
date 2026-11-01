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

export type OrgChange = { "change": "create", org: string, } | { "change": "add_member", org: string, user: string, owner: boolean, } | { "change": "remove_member", org: string, user: string, } | { "change": "create_team", org: string, team: string, } | { "change": "remove_team", org: string, team: string, } | { "change": "add_to_team", org: string, team: string, user: string, } | { "change": "remove_from_team", org: string, team: string, user: string, } | { "change": "default_visibility", org: string, visibility: Visibility, } | { "change": "links", org: string, on: boolean, } | { "change": "move_project", repo: string, org: string, } | { "change": "project_settings", repo: string, name: string | null, default_agent: string | null, default_preset: string | null, git_notes: GitNotes | null, forge_comments: boolean | null, };

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

export type GitNotes = "off" | "local" | "push";

export type AgentChoice = { id: string, name: string, description: string, 
/**
 * Its modes as it last listed them here, for a new thread to choose
 * from before it runs.
 */
modes: AgentModes | null, 
/**
 * Its settings, such as its model, as it last listed them here.
 */
settings: Array<AgentSetting>, 
/**
 * Whether it takes images attached to a prompt; none until it's been
 * seen here.
 */
takes_images: boolean | null, 
/**
 * Files other than text and images, likewise.
 */
takes_files: boolean | null, 
/**
 * The commands it offered when it last ran here.
 */
commands: Array<AgentCommand>, };

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

export type AgentCommand = { name: string, description: string, 
/**
 * What to write after it, when it takes anything.
 */
hint: string | null, };

export type AgentStart = { "kind": "resume", id: string, } | { "kind": "primed", id: string | null, prompt: string, };

export type AgentStatus = { agent: string, 
/**
 * The agent's registry id, as `AgentChoice` names it.
 */
agent_id: string, state: AgentState, 
/**
 * What the agent is saying or thinking right now, not recorded yet.
 */
streaming: string, thinking: string, permissions: Array<PermissionAsked>, 
/**
 * Questions the agent asked and waits on.
 */
questions: Array<QuestionAsked>, 
/**
 * The sign-in it waits on, when its own expired (ADR 0054).
 */
sign_in: SignInAsked | null, 
/**
 * How much of its context window the session uses, when it says.
 */
context: ContextUse | null, error: string | null, 
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
takes_files: boolean, 
/**
 * The commands it offers, which a prompt starting with `/` runs.
 */
commands: Array<AgentCommand>, };

export type QuestionAsked = { id: number, message: string, fields: Array<QuestionFieldAsked>, };

export type SignInAsked = { 
/**
 * The ways the agent lists, in its order.
 */
methods: Array<SignInChoice>, 
/**
 * The sign-in under way, when one is.
 */
running: SignInShown | null, 
/**
 * Why the last one didn't sign the agent in.
 */
failed: string | null, };

export type SignInChoice = { id: string, name: string, description: string | null, 
/**
 * The agent signs in by itself, on its machine, with nothing to show
 * here.
 */
by_agent: boolean, };

export type SignInShown = { 
/**
 * The method running.
 */
method: string, 
/**
 * The page to open, in any browser.
 */
link: string | null, 
/**
 * The code to enter on that page.
 */
code: string | null, 
/**
 * It waits for a code from the page to be pasted back.
 */
asks_code: boolean, 
/**
 * A code was pasted back, and the sign-in hasn't ended since.
 */
code_sent: boolean, 
/**
 * Its last lines, without escapes.
 */
tail: string, };

export type ContextUse = { used: number, size: number, };

export type QuestionFieldAsked = { 
/**
 * The key its answer goes under.
 */
key: string, title: string, description: string | null, 
/**
 * Its choices; none for a free answer.
 */
options: Array<ChoiceAsked>, multi: boolean, 
/**
 * The form can't be sent without it; the rest may be left empty.
 */
required: boolean, };

export type ChoiceAsked = { 
/**
 * What answering with it sends.
 */
value: string, label: string, description: string | null, };

export type BusEvent = { "event": "threads" } | { "event": "thread", thread: string, deltas: number, } | { "event": "agent", thread: string, status: AgentStatus | null, } | { "event": "queue", thread: string, } | { "event": "agent_text", thread: string, kind: TextKind, text: string, tool_use_id: string | null, restart: boolean, } | { "event": "inbox" } | { "event": "mrs" } | { "event": "agents" } | { "event": "importing", done: number, total: number, } | { "event": "lagged" };

export type BusSubscription = { subscribe: Array<string>, unsubscribe: Array<string>, };

export type CloneInfo = { repo: string, path: string, };

export type CloudLaunch = { 
/**
 * As claude.ai names it.
 */
id: string, url: string, repo: string, task: string, started_ms: number, 
/**
 * What claude warned about while starting it.
 */
warnings: Array<string>, 
/**
 * The thread recording it, once one does.
 */
thread: string | null, };

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
 * `user`, `wake` (the agent went on by itself), `assistant`,
 * `thinking`, `tool_call`, `tool_result`, `compaction`, `merge` or
 * `pull`.
 */
kind: string, text: string, time_ms: number, 
/**
 * A revert rewound it (ADR 0029).
 */
rewound: boolean, 
/**
 * On a `tool_call`: what the tool does, read from its input.
 */
tool: ToolView | null, 
/**
 * On a `tool_result`: the call it answers and how it ended.
 */
outcome: ToolOutcome | null, 
/**
 * On an `image`: the picture the entry before it carried.
 */
image: ImageView | null, 
/**
 * On a `usage`: what one of the agent's messages cost.
 */
usage: UsageView | null, 
/**
 * On a `plan`, or a `tool_call` that writes one: its steps.
 */
plan: Array<PlanStepView> | null, 
/**
 * The subagent transcript the entry is from, by its name; none for the
 * session's own. A `task` entry names the call that started it.
 */
side: string | null, 
/**
 * On a `task`: news of a subagent or background command.
 */
task: TaskView | null, };

export type ToolView = { id: string, name: string, 
/**
 * `execute`, `read`, `search`, `edit`, `question`, `fetch` or `other`.
 */
kind: string, 
/**
 * The command it ran, the file it read or changed, or what it looked for.
 */
subject: string | null, 
/**
 * Where a search looked, or what a command was for.
 */
detail: string | null, 
/**
 * What a question tool asked.
 */
questions: Array<QuestionView>, };

export type ToolOutcome = { 
/**
 * The id of the call it answers.
 */
call: string, failed: boolean, exit_code: number | null, 
/**
 * A question tool's answers.
 */
answers: Array<AnswerView>, };

export type ImageView = { media_type: string, 
/**
 * Base64; empty when it was too large to show.
 */
data: string, };

export type UsageView = { 
/**
 * The message it counts; one message may be recorded more than once.
 */
message: string, input: number, output: number, };

export type PlanStepView = { text: string, 
/**
 * `pending`, `in_progress` or `completed`.
 */
status: string, };

export type TaskView = { 
/**
 * The agent's id for the task.
 */
id: string, 
/**
 * The tool call that started it, when known.
 */
call: string | null, 
/**
 * `subagent` or `background`, when this news says.
 */
kind: string | null, 
/**
 * `running`, `completed`, `failed` or `stopped`.
 */
status: string, 
/**
 * A subagent's transcript, by the name its entries' `side` gives.
 */
side: string | null, 
/**
 * What the agent said of how it went.
 */
summary: string | null, };

export type QuestionView = { 
/**
 * A word or two naming it, such as `Progress`.
 */
header: string | null, question: string, multi_select: boolean, options: Array<ChoiceView>, };

export type ChoiceView = { label: string, description: string | null, };

export type AnswerView = { question: string, answer: string, };

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
 * When it was approved or declined.
 */
decided_ms: number | null, 
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
options: Array<[string, string]>, 
/**
 * The plan it asks to approve, as Markdown, when it is the agent's
 * own plan approval, as Claude's ExitPlanMode is.
 */
plan: string | null, 
/**
 * The tool call it asks about, whose card shows it waiting rather than running.
 */
tool_call: string | null, };

export type Preset = "ask_before_commands" | "ask_for_everything" | "full_access" | "plan";

export type Project = { 
/**
 * The normalized remote that names it.
 */
repo: string, name: string, org: string | null, origin_url: string | null, visibility: Visibility | null, default_agent: string | null, default_preset: string | null, git_notes: GitNotes, forge_comments: boolean, 
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
here: boolean, mode: string | null, 
/**
 * The agent's settings to choose once it's up, such as its model or
 * thinking level, by id and value.
 */
settings: Array<[string, string]>, preset: Preset | null, visibility: Visibility | null, 
/**
 * Sent once the agent is ready.
 */
prompt: string | null, 
/**
 * The first prompt is a Plan first one.
 */
plan: boolean, 
/**
 * Files sent with the first prompt.
 */
attachments: Array<PromptAttachment>, };

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
local_only: boolean, 
/**
 * The Claude Code cloud session it records, as claude.ai names it
 * (ADR 0040). A fork doesn't inherit it.
 */
cloud: string | null, };

export type TurnInfo = { 
/**
 * 1-based, as `silt diff --turn` counts them.
 */
number: number, 
/**
 * The history entry of the turn's prompt, or of the agent going on
 * by itself for a turn it opened.
 */
entry: number | null, prompt: string, };

export type FoundFolder = { path: string, 
/**
 * The normalized origin, empty without one.
 */
repo: string, 
/**
 * Threads start only in a git repository; outside one, its sessions
 * can still be imported.
 */
git: boolean, 
/**
 * Whether this machine knows it as a clone already.
 */
added: boolean, 
/**
 * Sessions not imported yet, by agent name, most first.
 */
sessions: Array<FoundSessions>, last_ms: number | null, };

export type FoundSessions = { agent: string, count: number, };
