// The daemon's API types, written by `UPDATE_API_TYPES=1 cargo test -p silt-daemon api_types`
// from crates/silt-daemon/src/protocol.rs. Don't edit by hand.

export type AgentActivity = { name: string, state: AgentState, 
/**
 * What it is saying right now, not recorded yet.
 */
streaming: string, };

export type AgentState = "starting" | "idle" | "working" | "awaiting_permission" | "awaiting_input" | "error" | "stopped";

export type InboxRead = { ids: Array<number>, all: boolean, };

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
settings: Array<AgentSetting>, };

export type BusEvent = { "event": "threads" } | { "event": "thread", thread: string, deltas: number, } | { "event": "agent", thread: string, status: AgentStatus | null, } | { "event": "queue", thread: string, } | { "event": "agent_text", thread: string, kind: TextKind, text: string, tool_use_id: string | null, restart: boolean, } | { "event": "inbox" } | { "event": "mrs" } | { "event": "lagged" };

export type BusSubscription = { subscribe: Array<string>, unsubscribe: Array<string>, };

export type CloneInfo = { repo: string, path: string, };

export type CommentLocation = { "at": "entry", index: number, excerpt: string, } | { "at": "lines", path: string, start: number, end: number, outdated: boolean, } | { "at": "unknown" };

export type CommentOn = { "on": "entry", index: number, } | { "on": "lines", path: string, start: number, end: number, };

export type CommentView = { id: string, by: string, time_ms: number, text: string, resolved: boolean, location: CommentLocation, };

export type DaemonStatus = { server: string | null, device: string | null, sync_error: string | null, threads: number, 
/**
 * Threads whose history here differs from the server's, so they no
 * longer push.
 */
diverged: Array<string>, 
/**
 * Recovery forks made since the daemon started.
 */
recovered: Array<string>, };

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
kind: string, text: string, time_ms: number, };

export type HistoryHit = { 
/**
 * The entry's index in the thread's history.
 */
index: number, kind: string, snippet: string, };

export type HistoryPage = { thread: string, total: number, entries: Array<HistoryEntry>, };

export type InFlight = { thread: string, owner: string, files: Array<string>, last_ms: number, };

export type InboxEntry = { id: number, kind: string, thread: string, actor: string, text: string, created_ms: number, read: boolean, };

export type LiveState = { deltas: number, 
/**
 * The agent the web UI started in the thread, if any.
 */
agent: AgentStatus | null, 
/**
 * Which permission requests the daemon answers by itself.
 */
preset: Preset, };

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
conversation: Array<ShowLine>, };

export type MrSummary = { id: string, fork: string, target: string, title: string, summary: string, 
/**
 * `open`, `approved` or `declined: <reason>`.
 */
status: string, opened_ms: number, };

export type PermissionAsked = { id: number, title: string, 
/**
 * Option ids and labels.
 */
options: Array<[string, string]>, };

export type Preset = "ask_before_commands" | "ask_for_everything" | "full_access";

export type Queued = { id: number, author: string, text: string, created_ms: number, };

export type Pulled = { thread: string, from: string, 
/**
 * Conversation entries the pull brought in.
 */
messages: number, files: Array<string>, };

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
owner: string | null, status: ThreadStatus | null, repo: string | null, };

export type ShowLine = { "line": "entry", kind: string, text: string, } | { "line": "tool_call", id: string, name: string, input: string, } | { "line": "edits", tool_use_id: string | null, files: Array<FileEdit>, } | { "line": "compaction", summary: string | null, } | { "line": "turn", mark: string, } | { "line": "segment", label: string, } | { "line": "event", text: string, };

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
agent_state: AgentState | null, };

export type TurnInfo = { 
/**
 * 1-based, as `silt diff --turn` counts them.
 */
number: number, 
/**
 * The history entry of the turn's prompt.
 */
entry: number | null, prompt: string, };
