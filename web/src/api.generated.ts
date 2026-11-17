// The daemon's API types, written by `UPDATE_API_TYPES=1 cargo test -p silt-daemon api_types`
// from crates/silt-daemon/src/protocol.rs. Don't edit by hand.

export type AgentActivity = { name: string, state: AgentState, 
/**
 * What it is saying right now, not recorded yet.
 */
streaming: string, };

export type AgentState = "starting" | "idle" | "working" | "awaiting_permission" | "awaiting_input" | "error" | "stopped";

export type Capabilities = { answer_permissions: boolean, send_prompts: boolean, run_agents: boolean, review: boolean, 
/**
 * Ask one of the person's computers' agents a page's question (ADR 0072).
 */
sample: boolean, 
/**
 * Call the person's page connectors on one of their computers (ADR 0080).
 */
page_tools: boolean, };

export type InboxAction = { "do": "permission", id: number, title: string, options: Array<[string, string]>, } | { "do": "plan" };

export type KindSetting = { kind: string, notify: boolean, };

export type Startable = { id: string, name: string, };

export type InboxRead = { ids: Array<number>, all: boolean, };

export type OrgChange = { "change": "create", org: string, } | { "change": "add_member", org: string, user: string, owner: boolean, } | { "change": "remove_member", org: string, user: string, } | { "change": "create_team", org: string, team: string, } | { "change": "remove_team", org: string, team: string, } | { "change": "add_to_team", org: string, team: string, user: string, } | { "change": "remove_from_team", org: string, team: string, user: string, } | { "change": "default_visibility", org: string, visibility: Visibility, } | { "change": "links", org: string, on: boolean, } | { "change": "policy", org: string, policy: Policy, } | { "change": "move_project", repo: string, org: string, } | { "change": "project_settings", repo: string, name: string | null, default_agent: string | null, default_preset: string | null, git_notes: GitNotes | null, forge_comments: boolean | null, };

export type OrgInfo = { name: string, role: OrgRole, 
/**
 * The caller owns it or administers the server.
 */
may_change: boolean, default_visibility: Visibility, members: Array<OrgMember>, teams: Array<TeamInfo>, projects: Array<string>, 
/**
 * Its threads may have read-only links.
 */
links: boolean, 
/**
 * What agents started in its projects may do, on every member's
 * machine (ADR 0065).
 */
policy: Policy, };

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
 * The sign-in it waits on, when its own expired (ADR 0056).
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
commands: Array<AgentCommand>, 
/**
 * The calls whose commands still run in the background, which Stop
 * ends.
 */
stoppable: Array<string>, };

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

export type BusEvent = { "event": "threads" } | { "event": "thread", thread: string, deltas: number, } | { "event": "agent", thread: string, status: AgentStatus | null, } | { "event": "queue", thread: string, } | { "event": "agent_text", thread: string, kind: TextKind, text: string, tool_use_id: string | null, restart: boolean, } | { "event": "inbox" } | { "event": "mrs" } | { "event": "agents" } | { "event": "importing", done: number, total: number, } | { "event": "sample", ask: string, text: string, } | { "event": "sample_tool", ask: string, call: number, name: string, input: unknown, } | { "event": "room", deliveries: unknown, } | { "event": "lagged" };

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

export type CommentLocation = { "at": "entry", index: number, excerpt: string, } | { "at": "lines", path: string, start: number, end: number, outdated: boolean, 
/**
 * Where they are in the base the comments were asked against, when
 * they're there: a diff shows a comment on deleted lines beside them.
 */
before: LineSpan | null, } | { "at": "doc", artifact: string, start: number, end: number, excerpt: string, outdated: boolean, } | { "at": "cell", artifact: string, sheet: number, address: string, excerpt: string, outdated: boolean, } | { "at": "element", artifact: string, anchor: string, } | { "at": "unknown" };

export type LineSpan = { start: number, end: number, };

export type CommentOn = { "on": "entry", index: number, } | { "on": "lines", path: string, start: number, end: number, base?: string | null, 
/**
 * The lines are numbered as a diff with git's rewrites left out
 * shows them (ADR 0069).
 */
hide_git?: boolean, } | { "on": "anchored", anchor: string, } | { "on": "doc", artifact: string, start: number, end: number, } | { "on": "cell", artifact: string, sheet: number, address: string, } | { "on": "element", artifact: string, anchor: string, };

export type PendingAnchor = { anchor: string, };

export type PlaceAnchors = { base: string | null, anchors: Array<string>, hide_git?: boolean, };

export type CommentView = { id: string, by: string, time_ms: number, text: string, resolved: boolean, 
/**
 * It went to the thread's agent, in a review (ADR 0055).
 */
sent: boolean, location: CommentLocation, 
/**
 * The conversation under it, oldest first (ADR 0025).
 */
replies: Array<ReplyView>, };

export type CommitView = { commit: string, subject: string | null, branch: string | null, turn: number | null, time_ms: number, };

export type DiffStat = { 
/**
 * Its new path, or its old one when deleted.
 */
path: string, 
/**
 * Where it was before it moved.
 */
old_path: string | null, added: number, removed: number, binary: boolean, 
/**
 * How long its part of the diff is, in bytes.
 */
size: number, };

export type FileTurns = { path: string, runs: Array<TurnRun>, };

export type TurnRun = { start: number, end: number, turn: number, };

export type Seen = "seen" | "changed";

export type FileSeen = { path: string, seen: Seen, new_lines: Array<LineSpan>, };

export type ThreadSeenView = { head: string, files: Array<FileSeen>, };

export type MarkSeen = { path: string, seen: boolean, at: string, };

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

export type NewLink = { days: number, frozen: boolean, 
/**
 * A link to one artifact of the thread rather than its conversation.
 */
artifact: LinkedArtifact | null, };

export type ThreadLink = { id: number, created_ms: number, expires_ms: number, 
/**
 * Shows the thread as it was when made, not as it goes on.
 */
frozen: boolean, revoked: boolean, 
/**
 * Only when just made: the server keeps no copy of the token.
 */
url: string | null, 
/**
 * The one artifact it shows, rather than the conversation (ADR 0068).
 */
artifact: LinkedArtifact | null, };

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
 * On an `artifact`: what the agent showed the person (ADR 0068).
 */
artifact: ArtifactView | null, 
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

export type ArtifactView = { 
/**
 * The same for every version of one artifact.
 */
id: string, 
/**
 * Counts up from 1 along the thread's lineage.
 */
version: number, title: string, 
/**
 * `html`, `svg`, `markdown`, `mermaid` or `image`.
 */
kind: string, media_type: string, 
/**
 * The blob holding the content, read with the thread's artifact route.
 */
blob: string, size: number, 
/**
 * Shown in the conversation (`silt_show`) rather than as a card.
 */
inline: boolean, 
/**
 * Words under an image shown in the conversation.
 */
caption?: string | null, 
/**
 * The design system an HTML version was styled with (ADR 0071).
 */
design_system?: string | null, 
/**
 * The blob holding the CSS that system's tokens compiled to, which the
 * version's frame is given; read with the same route as the content.
 */
style?: string | null, 
/**
 * What this version changed (ADR 0073).
 */
note?: string | null, 
/**
 * What the artifact is, as its card says.
 */
description?: string | null, 
/**
 * The capabilities the version's page may use, each with its config
 * (ADR 0072).
 */
capabilities?: Record<string, Record<string, unknown>> | null, 
/**
 * The person this version was saved for, when the page saved itself
 * rather than an agent making it (ADR 0072).
 */
by?: string | null, 
/**
 * An HTML version's runtime contract, which its page's `silt` is
 * (ADR 0072).
 */
contract?: string | null, 
/**
 * The files beside an HTML version's page, by path, which its frame
 * fetches relative to itself (ADR 0074).
 */
files?: { [key in string]: ArtifactFile } | null, };

export type ArtifactFile = { blob: string, size: number, media_type: string, };

export type ArtifactListed = { thread: string, thread_title: string | null, repo: string, owner: string, 
/**
 * Whether the thread is the person's own.
 */
mine: boolean, artifact: ArtifactView, 
/**
 * When its latest version was made.
 */
time_ms: number, pinned: boolean, 
/**
 * Hidden by the thread's owner; shown only when asked for.
 */
archived: boolean, };

export type PageConnector = { name: string, 
/**
 * A stdio server's program, with `args`.
 */
command: string | null, args: Array<string>, 
/**
 * A Streamable HTTP server's address.
 */
url: string | null, 
/**
 * Headers sent to an HTTP server, each from the environment variable
 * it names, so no secret is kept in the file.
 */
header_env: { [key in string]: string }, 
/**
 * Headers sent to an HTTP server whose values this machine's keyring
 * keeps, set in Settings; the file names them only.
 */
header_secret: Array<string>, 
/**
 * The environment variables a stdio server gets beyond the few every
 * program needs ([`BASE_ENV`]), by name; nothing else of the daemon's
 * environment reaches it.
 */
pass_env: Array<string>, 
/**
 * The tools pages may call, by name; every one is off until ticked.
 */
tools: { [key in string]: boolean }, };

export type ConnectorTool = { name: string, description: string | null, 
/**
 * The server marks it as only reading; anything else is an action the
 * viewer confirms each time.
 */
read_only: boolean, input_schema: unknown, };

export type DesignSource = "project" | "personal";

export type ArtifactSettings = { 
/**
 * Through Silt's vendor route, which fetches and keeps each file, so
 * the viewer's browser never contacts a CDN; else from the CDNs.
 */
proxy: boolean, };

export type DesignSystemView = { 
/**
 * What `design_system` takes.
 */
name: string, 
/**
 * The name its tokens give it.
 */
title: string, source: DesignSource, readme?: string | null, 
/**
 * Its custom properties and type classes, by family.
 */
tokens?: { [key in string]: Array<string> }, 
/**
 * Its components, given with its README (ADR 0086).
 */
components?: Array<DesignComponentView>, 
/**
 * Its folder and its assets' paths under `assets/`, given with its
 * README (ADR 0088).
 */
dir?: string | null, assets?: Array<string>, };

export type DesignComponentView = { name: string, about: string, html: string, 
/**
 * Its guidelines, `components/<name>.md` (ADR 0088).
 */
guide?: string | null, };

export type DesignCatalog = { 
/**
 * Each project's own, then the person's.
 */
groups: Array<DesignGroup>, };

export type DesignGroup = { 
/**
 * The project whose repository keeps them; none for the person's own.
 */
project: DesignProject | null, systems: Array<DesignSummary>, };

export type DesignProject = { repo: string, name: string, clone: string, };

export type DesignSummary = { name: string, title: string, source: DesignSource, 
/**
 * Its folder.
 */
dir: string, 
/**
 * A few of its colours, in its first theme.
 */
swatches: Array<string>, colors: number, type_styles: number, components: number, 
/**
 * What picks it as a default: "repo's default", "picked for web on
 * this machine", "default for Work".
 */
uses: Array<string>, 
/**
 * The thread still making or changing it, when one is.
 */
thread?: string | null, 
/**
 * It's only in that thread's folder so far.
 */
unmerged: boolean, };

export type DesignDetail = { summary: DesignSummary, readme: string, 
/**
 * Where it was made from, as its tokens' `meta.source` says.
 */
made_from?: string | null, themes: Array<DesignTheme>, 
/**
 * Its tokens with their values per theme, colours first.
 */
tokens: Array<DesignToken>, type_styles: Array<DesignTypeStyle>, 
/**
 * The CSS its tokens and components compile to, as a frame gets it.
 */
css: string, components: Array<DesignComponentView>, cover?: string | null, assets: Array<DesignAssetView>, 
/**
 * Every file in its folder, by path.
 */
files: Array<string>, 
/**
 * What its files hold that Silt left out, and why.
 */
dropped: Array<string>, };

export type DesignTheme = { id: string, name: string, };

export type DesignToken = { family: string, name: string, 
/**
 * Its value by theme id; one value holds in every theme under `*`.
 */
values: { [key in string]: string }, usage: string, };

export type DesignTypeStyle = { name: string, group: string, 
/**
 * Its font stack, from its family.
 */
family: string, font_size: string, line_height: string | null, font_weight: string | null, letter_spacing: string | null, usage: string, };

export type DesignAssetView = { path: string, media_type: string, size: number, };

export type DesignAssetData = { media_type: string, data: string, };

export type DesignFrom = "code" | "website" | "description";

export type DesignKeep = "repo" | "personal";

export type DesignStart = { 
/**
 * The system's name as people read it, such as "Harbor"; its folder's
 * name comes from it.
 */
title: string, agent: string, keep: DesignKeep, 
/**
 * The project: the one built from, the one whose repository keeps it,
 * or both.
 */
clone: string | null, from: DesignFrom | null, 
/**
 * The website's address, for one made from a website.
 */
link: string | null, 
/**
 * What to make it from, for one made from a description, or anything
 * else the person adds.
 */
notes: string | null, 
/**
 * A brand guide, screenshots or anything else sent with the prompt.
 */
attachments: Array<PromptAttachment>, 
/**
 * What to change in the system `name`, which exists; a new one when
 * unset.
 */
change: string | null, name: string | null, };

export type ThreadDesign = { name: string, title: string, keep: DesignKeep, changing: boolean, 
/**
 * What's written so far; none until the folder has its tokens.
 */
detail: DesignDetail | null, };

export type ArtifactContent = { media_type: string, 
/**
 * Base64.
 */
data: string, };

export type LinkedArtifact = { id: string, version: number | null, };

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
seen_ms: number | null, 
/**
 * The turn they reviewed its changes through.
 */
reviewed_turn: number | null, };

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
server: string | null, 
/**
 * The workspace it's in on this machine, and why (ADR 0065); none from
 * a server.
 */
workspace: string | null, workspace_why: string | null, };

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
before_compaction: boolean, 
/**
 * The doc or sheet whose text it's in.
 */
artifact: string | null, };

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
attachments: Array<PromptAttachment>, 
/**
 * A new thread is a chat, in a folder of its own rather than a clone
 * (ADR 0059).
 */
chat: boolean, 
/**
 * A chat the new thread continues in a project: the agent starts from
 * its conversation.
 */
continues: string | null, 
/**
 * Where in the new thread's checkout the chat's files are copied, a
 * relative path; nowhere when unset.
 */
copy_to: string | null, 
/**
 * The workspace a new chat is in, by id; the default when unset
 * (ADR 0065).
 */
workspace: string | null, };

export type ChatSettings = { 
/**
 * Where chats' folders go; `$SILT_HOME/chats` when unset.
 */
dir: string | null, 
/**
 * The permissions a new chat starts with: full access, as a chat's
 * folder holds only what its agent wrote there.
 */
preset: Preset, };

export type WorkspacesView = { default: WorkspaceId, 
/**
 * Whether the person made any; until then there's the one Default,
 * syncing with the default server.
 */
saved: boolean, workspaces: Array<WorkspaceView>, };

export type WorkspaceView = { workspace: Workspace, 
/**
 * Its org's policy, as last fetched.
 */
org_policy: OrgPolicy | null, 
/**
 * It's bound to an org whose policy hasn't been fetched yet, so no agent
 * starts in it.
 */
waiting_for_policy: boolean, 
/**
 * Its own rules and its org's, together.
 */
effective: Policy, projects: Array<WorkspaceProject>, };

export type WorkspaceProject = { repo: string, why: Why, };

export type Workspace = { id: WorkspaceId, name: string, 
/**
 * `None` keeps its threads on this machine.
 */
server: ServerBinding | null, rules: Array<MatchRule>, defaults: Defaults, 
/**
 * Given to every agent started in it, such as its own
 * `CLAUDE_CONFIG_DIR`.
 */
env: { [key in string]: string }, policy: Policy, };

export type Workspaces = { 
/**
 * Where projects no rule matches go.
 */
default: WorkspaceId, workspaces: Array<Workspace>, };

export type ServerBinding = { url: string, org?: string | null, };

export type MatchRule = { "kind": "remote", "pattern": string } | { "kind": "folder", "pattern": string };

export type Defaults = { agent?: string | null, preset?: Preset | null, chats_preset?: Preset | null, 
/**
 * By agent id, as each agent's models and modes are its own.
 */
agents?: { [key in string]: AgentDefaults }, 
/**
 * The design system artifacts are styled with when the project names
 * none (ADR 0071).
 */
design_system?: string | null, };

export type AgentDefaults = { mode?: string | null, settings?: { [key in string]: string }, };

export type Policy = { 
/**
 * The agents allowed; any when unset.
 */
agents?: Array<AgentKey> | null, 
/**
 * Whether `agents.json`'s agents are allowed; they are when unset.
 */
custom_agents?: boolean | null, 
/**
 * The most an agent may do without asking.
 */
max_preset?: Preset | null, 
/**
 * The agent runs only with its own sandbox on.
 */
sandbox?: boolean, 
/**
 * "Hide other workspaces' files": the sandbox also keeps the agent out
 * of Silt's data and other workspaces' folders; implies `sandbox`.
 * History tools are scoped to their workspace without it.
 */
isolated?: boolean, 
/**
 * Projects keep the workspace's server: no other, and not this machine
 * only.
 */
home_locked?: boolean, 
/**
 * The page connectors its artifacts' pages may call, by name; any
 * when unset (ADR 0080).
 */
connectors?: Array<string> | null, };

export type OrgPolicy = { org: string, server: string, policy: Policy, };

export type Why = { "kind": "chosen" } | { "kind": "rule", "rule": MatchRule } | { "kind": "default" };

export type WorkspaceId = string;

export type AgentKey = string;

export type StateChange = { pinned: boolean | null, snooze: Snooze | null, settled: boolean | null, seen: boolean, 
/**
 * The turn the thread was reviewed through.
 */
reviewed_turn: number | null, };

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
 * Commands and subagents its agent left running, which go on after
 * its turn ends.
 */
background: number, 
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
 * The workspace it's in on this machine (ADR 0065); none from a server.
 */
workspace: string | null, 
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

export type VoiceDownload = { done: number, total: number, };

export type VoiceEvent = { "kind": "partial", text: string, } | { "kind": "segment", text: string, } | { "kind": "paused" } | { "kind": "done" } | { "kind": "failed", message: string, };

export type VoiceStatus = { 
/**
 * Whether `silt-voice` is installed.
 */
helper: boolean, 
/**
 * Whether the speech model has been downloaded.
 */
model: boolean, download: VoiceDownload | null, 
/**
 * Why the last download failed.
 */
failed: string | null, };
