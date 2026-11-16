// Talks to the daemon that served this page (ADR 0011), or the server when
// it hosts the app (ADR 0026), with its session cookie. A daemon's token
// arrives once in the URL fragment from `silt open`, or is traded for the
// one-time code a browser opened by `silt open` carries, and is kept for the tab.

import { apiUrl } from './served'
import { askedServer } from './workspace'
import type {
  AgentChoices,
  ArtifactContent,
  ArtifactListed,
  ConnectorTool,
  PageConnector,
  ArtifactSettings,
  ChatSettings,
  CloudLaunch,
  DesignSystemView,
  CommentLocation,
  CommentOn,
  CommentView,
  CommitView,
  DaemonStatus,
  DeviceChange,
  DeviceView,
  DiffStat,
  FileAt,
  FileTurns,
  FoundFolder,
  GraphQuery,
  GraphRow,
  HistoryPage,
  InboxEntry,
  InboxRead,
  KindSetting,
  LiveState,
  MarkSeen,
  Mine,
  MrDetails,
  MrSummary,
  NewLink,
  OrgChange,
  OrgInfo,
  PendingAnchor,
  PhonePairing,
  PlaceAnchors,
  PresenceReply,
  Preset,
  Project,
  PromptAttachment,
  Queued,
  ReplicaDeltas,
  Resumed,
  Reverted,
  SearchResult,
  StartOptions,
  StateChange,
  ThreadLink,
  ThreadSeenView,
  ThreadSummary,
  TurnInfo,
  VoiceStatus,
  Workspaces,
  WorkspacesView,
} from './api.generated'

/** Who views a thread's pages: their id in page data, their level, and the names of the people the thread knows. */
/** A tool a page offers the agent answering its question; the page runs it (ADR 0072). */
export type SampleTool = { name: string; description: string; input_schema?: unknown }
export type PageViewer = { id: string; level: 'read' | 'comment' | 'collaborator' | 'owner'; people: Record<string, string> }

export type * from './api.generated'

const TOKEN_KEY = 'silt-token'

export async function takeTokenFromUrl(): Promise<void> {
  const token = window.location.hash.match(/token=([0-9a-f]+)/)
  const code = window.location.hash.match(/code=([0-9a-f]+)/)
  if (token) {
    sessionStorage.setItem(TOKEN_KEY, token[1])
  } else if (code) {
    const response = await fetch(apiUrl('/api/exchange'), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ code: code[1] }),
    })
    if (response.ok) {
      const { token } = (await response.json()) as { token: string }
      sessionStorage.setItem(TOKEN_KEY, token)
    }
  }
  if (token || code) window.history.replaceState(null, '', window.location.pathname + window.location.search)
}

export class ApiError extends Error {
  /** The code a page reads for a refusal, as the daemon or server said it (ADR 0072). */
  refusal: string | null = null
}

export function token(): string {
  return sessionStorage.getItem(TOKEN_KEY) ?? ''
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(apiUrl(path), {
    method,
    headers: {
      Authorization: `Bearer ${sessionStorage.getItem(TOKEN_KEY) ?? ''}`,
      // The server takes changes only with it, which other sites can't add (ADR 0026).
      'X-Silt-Request': '1',
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const data = await response.json().catch(() => ({}))
  if (!response.ok) {
    const said = data as { error?: string; message?: string; refusal?: string | null }
    const failure = new ApiError(said.error ?? said.message ?? `request failed (${response.status})`)
    failure.refusal = typeof said.refusal === 'string' ? said.refusal : null
    throw failure
  }
  return data as T
}

/** A file the API answers with, as bytes. */
async function requestBlob(path: string): Promise<Blob> {
  const response = await fetch(apiUrl(path), {
    headers: { Authorization: `Bearer ${sessionStorage.getItem(TOKEN_KEY) ?? ''}`, 'X-Silt-Request': '1' },
  })
  if (!response.ok) throw new ApiError(`request failed (${response.status})`)
  return response.blob()
}

/** A sheet as its viewer shows it (ADR 0077). */
export type SheetView = {
  rows: string[]
  columns: string[]
  raw: string[][]
  values: string[][]
  sheets?: number
  names?: string[]
  widths?: (number | null)[]
  formats?: (string | null)[]
  /** Cells' own number formats by address, `B3`, which go before their column's. */
  cell_formats?: Record<string, string>
}

/** The search form's fields, as typed; `since` is a date. */
export interface SearchForm {
  repo?: string
  author?: string
  status?: string
  branch?: string
  file?: string
  since?: string
}

const thread = (id: string) => `/api/threads/${encodeURIComponent(id)}`
/** Which of a thread's changes a diff shows: those of `turn`, or of turns `turn` to `toTurn`, else since `base`; with
 * `hideGit`, without what git itself rewrote (ADR 0069). */
export interface DiffSpec {
  turn?: number
  toTurn?: number
  base?: string
  hideGit?: boolean
}

const diffParams = (options: DiffSpec) => {
  const params = new URLSearchParams()
  if (options.turn !== undefined) params.set('turn', String(options.turn))
  if (options.toTurn !== undefined) params.set('to_turn', String(options.toTurn))
  if (options.base) params.set('base', options.base)
  if (options.hideGit) params.set('git', 'hide')
  return params
}
/** Names which server to ask, on a machine signed in to several (ADR 0032). */
const onServer = (server?: string | null) => (server ? `?server=${encodeURIComponent(server)}` : '')

/** A socket for one dictation: audio frames out, `VoiceEvent`s back (ADR 0061). */
export function dictationSocket(): WebSocket {
  const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws'
  return new WebSocket(`${scheme}://${window.location.host}${apiUrl('/api/voice/dictate')}?token=${token()}`)
}

export const api = {
  voiceStatus: () => request<VoiceStatus>('GET', '/api/voice'),
  voiceModel: () => request<VoiceStatus>('POST', '/api/voice/model'),
  threads: () => request<ThreadSummary[]>('GET', '/api/threads'),
  /** An artifact's content, by the blob a version of it names (ADR 0068). */
  artifact: (id: string, blob: string) => request<ArtifactContent>('GET', `${thread(id)}/artifacts/${blob}`),
  /** A read or write of an artifact's page data (ADR 0072). */
  pageData: (id: string, artifact: string, asked: Record<string, unknown>) => request<unknown>('POST', `${thread(id)}/artifacts/${artifact}/data`, asked),
  /** What the person allowed or refused an artifact's page, by capability (ADR 0072). */
  artifactGrants: (id: string, artifact: string) => request<Record<string, 'granted' | 'refused'>>('GET', `${thread(id)}/artifacts/${artifact}/grants`),
  /** Kept for what `version`, the one the person saw, declares of `capability`. */
  setArtifactGrant: (id: string, artifact: string, capability: string, state: 'granted' | 'refused' | null, version?: number) =>
    request<unknown>('PUT', `${thread(id)}/artifacts/${artifact}/grants`, { capability, state, version: version ?? null }),
  /** A version an HTML artifact's page saves itself as; refused as a conflict unless `base` is the latest (ADR 0072). */
  publishArtifact: (id: string, artifact: string, html: string, base: number) =>
    request<{ id: string; version: number }>('POST', `${thread(id)}/artifacts/${artifact}/publish`, { html, base }),
  /** The files people added to an artifact's page (ADR 0074). */
  pageAssets: (id: string, artifact: string, asked: { op: 'add'; name: string; media_type: string; data: string } | { op: 'list' } | { op: 'delete'; id: string }) =>
    request<unknown>('POST', `${thread(id)}/artifacts/${artifact}/assets`, asked),
  /** The thread's files, for a page that declares `files` (ADR 0078). */
  pageFiles: (id: string, artifact: string, asked: Record<string, unknown>) => request<unknown>('POST', `${thread(id)}/artifacts/${artifact}/files`, asked),
  /** Every artifact in the threads the person can read, for the Artifacts page (ADR 0075). */
  artifacts: () => request<ArtifactListed[]>('GET', '/api/artifacts'),
  /** An artifact's thumbnail, drawn the first time it's asked for where a headless browser is installed (ADR 0075). */
  artifactThumbnail: (id: string, artifact: string) => request<ArtifactContent>('GET', `${thread(id)}/artifacts/${artifact}/thumbnail`),
  pinArtifact: (id: string, artifact: string, pinned: boolean) => request<unknown>('PUT', `${thread(id)}/artifacts/${artifact}/pin`, { pinned }),
  /** Hides an artifact for everyone, or shows it again; the thread's owner's to do. */
  archiveArtifact: (id: string, artifact: string, archived: boolean) =>
    request<unknown>('PUT', `${thread(id)}/artifacts/${artifact}/archive`, { archived }),
  /** A page's call of the viewer's page connectors (ADR 0080). */
  pageMcp: (id: string, artifact: string, asked: Record<string, unknown>) => request<unknown>('POST', `${thread(id)}/artifacts/${artifact}/mcp`, asked),
  /** The MCP servers this machine lists for pages (ADR 0080). */
  pageConnectors: () => request<PageConnector[]>('GET', '/api/page-connectors'),
  setPageConnectors: (connectors: PageConnector[]) => request<unknown>('PUT', '/api/page-connectors', connectors),
  importPageConnectors: () => request<PageConnector[]>('POST', '/api/page-connectors/import'),
  connectorTools: (name: string) => request<ConnectorTool[]>('GET', `/api/page-connectors/${encodeURIComponent(name)}/tools`),
  /** Keeps a header's value for an HTTP connector in this computer's keyring, or forgets it with `null`. */
  setConnectorSecret: (name: string, header: string, value: string | null) =>
    request<unknown>('PUT', `/api/page-connectors/${encodeURIComponent(name)}/secret`, { header, value }),
  /** A doc's text, or edits of it (ADR 0077). */
  docData: (
    id: string,
    artifact: string,
    asked: { op: 'read' } | { op: 'edit'; edits: { pos: number; delete: number; insert: string }[]; base?: string | null },
  ) => request<{ text: string; base?: string }>('POST', `${thread(id)}/artifacts/${artifact}/doc`, asked),
  /** A sheet's cells and values, or changes to it (ADR 0077). */
  sheetData: (id: string, artifact: string, asked: Record<string, unknown>) => request<unknown>('POST', `${thread(id)}/artifacts/${artifact}/sheet`, asked),
  sheetXlsx: (id: string, artifact: string) => requestBlob(`${thread(id)}/artifacts/${artifact}/sheet.xlsx`),
  /** Who views the thread's pages, for their `user` capability. */
  pageViewer: (id: string) => request<PageViewer>('GET', `${thread(id)}/viewer`),
  /** A page's question for an agent, its answer streamed as `sample` events named `ask` where the bus carries them. */
  /** One of the files people added to a page, for a view Silt draws itself, such as a whiteboard's images. */
  pageAsset: (id: string, artifact: string, asset: string) =>
    request<ArtifactContent>('GET', `${thread(id)}/artifacts/${artifact}/assets/${encodeURIComponent(asset)}`),
  /** Keeps a doc, a sheet or a whiteboard as it stands as its next version (ADR 0077). */
  saveVersion: (id: string, artifact: string, note: string) =>
    request<{ id: string; version: number }>('POST', `${thread(id)}/artifacts/${artifact}/versions`, { note }),
  /** A doc or a deck printed to a PDF by this computer's headless browser, from the page drawn of it. */
  artifactPdf: (id: string, artifact: string, html: string) => request<ArtifactContent>('POST', `${thread(id)}/artifacts/${artifact}/pdf`, { html }),
  /** A page's answer to the agent's call of a tool it gave its question. */
  sampleToolResult: (id: string, artifact: string, ask: string, call: number, result: { ok: boolean; value?: unknown; error?: string }) =>
    request<null>('POST', `${thread(id)}/artifacts/${artifact}/sample/${encodeURIComponent(ask)}/tools/${call}`, result),
  /** What the agent has said so far of a page's question in the hosted app, which a device answers. */
  sampleSoFar: (id: string, artifact: string, ask: string) =>
    request<{ text: string; calls?: { call: number; name: string; input: unknown }[] }>(
      'GET',
      `${thread(id)}/artifacts/${artifact}/sample/${encodeURIComponent(ask)}`,
    ),
  sample: (
    id: string,
    artifact: string,
    ask: string,
    input: unknown,
    images: { media_type: string; data: string }[] = [],
    tools: SampleTool[] = [],
    model: { model_tier?: string; model?: string } = {},
  ) =>
    request<{ text: string; truncated: boolean; model_applied?: string }>('POST', `${thread(id)}/artifacts/${artifact}/sample`, {
      ask,
      input,
      images,
      tools,
      model,
    }),
  /** A key that opens one version of an HTML artifact in a frame, at the path it answers (ADR 0074). */
  artifactFrame: (id: string, artifact: string, ask: { version: number; theme: 'light' | 'dark'; parent: string; boot?: unknown }) =>
    request<{ path: string; expires_in_ms: number }>('POST', `${thread(id)}/artifacts/${artifact}/frame`, ask),
  /** A thread's history; with `limit`, only that many entries from the `offset`th. */
  history: (id: string, offset?: number, limit?: number) =>
    request<HistoryPage>('GET', `${thread(id)}/history${limit === undefined ? '' : `?offset=${offset ?? 0}&limit=${limit}`}`),
  turns: (id: string) => request<TurnInfo[]>('GET', `${thread(id)}/turns`),
  commits: (id: string) => request<CommitView[]>('GET', `${thread(id)}/commits`),
  /** The thread's files this machine's checkouts don't write (ADR 0045); a daemon's only. */
  unwritten: (id: string) => request<string[]>('GET', `${thread(id)}/unwritten`),
  fileAt: (id: string, path: string, at?: string) =>
    request<FileAt>('GET', `${thread(id)}/files/at?path=${encodeURIComponent(path)}${at ? `&at=${encodeURIComponent(at)}` : ''}`),
  graph: (query: GraphQuery) => {
    const params = new URLSearchParams()
    for (const [key, value] of Object.entries(query)) {
      if (value !== null && value !== false && value !== '') params.set(key, String(value))
    }
    return request<GraphRow[]>('GET', `/api/graph?${params}`)
  },
  /** What the thread changed; with `limit`, only that many files from the `offset`th, in path order; with `path`, only that file. */
  diff: (id: string, options: DiffSpec & { offset?: number; limit?: number; path?: string } = {}) => {
    const params = diffParams(options)
    if (options.limit !== undefined) {
      params.set('offset', String(options.offset ?? 0))
      params.set('limit', String(options.limit))
    }
    if (options.path) params.set('path', options.path)
    return request<string>('GET', `${thread(id)}/diff?${params}`)
  },
  /** The diff's files with their counts and sizes but not their lines, in the diff's order. */
  diffStats: (id: string, options: DiffSpec = {}) => request<DiffStat[]>('GET', `${thread(id)}/diff/stats?${diffParams(options)}`),
  /** The diff's files this person marked seen: each as they saw it, or changed since, with the lines new since (ADR 0067). */
  threadSeen: (id: string, options: DiffSpec = {}) => request<ThreadSeenView>('GET', `${thread(id)}/seen?${diffParams(options)}`),
  markThreadSeen: (id: string, mark: MarkSeen) => request<string>('PUT', `${thread(id)}/seen`, mark),
  /** For the diff's files, the turns that last wrote their new lines. */
  diffTurns: (id: string, options: DiffSpec = {}) => request<FileTurns[]>('GET', `${thread(id)}/diff/turns?${diffParams(options)}`),
  /** With `base`, a diff base, code comments say where they are there too. */
  comments: (id: string, base?: string, hideGit = false) => {
    const params = new URLSearchParams()
    if (base) params.set('base', base)
    if (hideGit) params.set('git', 'hide')
    const query = params.toString()
    return request<CommentView[]>('GET', `${thread(id)}/comments${query ? `?${query}` : ''}`)
  },
  commentsSent: (id: string, comments: string[]) => request<CommentView[]>('POST', `${thread(id)}/comments/sent`, { comments }),
  addComment: (id: string, on: CommentOn, text: string) =>
    request<CommentView>('POST', `${thread(id)}/comments`, { on, text }),
  /** Anchors lines for a pending review comment, recording nothing, so it follows their text (ADR 0066). */
  anchorLines: (id: string, on: CommentOn) => request<PendingAnchor>('POST', `${thread(id)}/comments/anchor`, on),
  /** Where pending review comments' anchors are now, and at `base`, as comments are placed. */
  placeAnchors: (id: string, place: PlaceAnchors) => request<CommentLocation[]>('POST', `${thread(id)}/comments/place`, place),
  reply: (id: string, comment: string, text: string) =>
    request<CommentView>('POST', `${thread(id)}/comments/${encodeURIComponent(comment)}/replies`, { text }),
  resolveComment: (id: string, comment: string) =>
    request<CommentView>('POST', `${thread(id)}/comments/${encodeURIComponent(comment)}/resolve`),
  search: (query: string, filters: SearchForm) => {
    const params = new URLSearchParams({ q: query })
    for (const [name, value] of Object.entries(filters)) {
      if (!value) continue
      params.set(name, name === 'since' ? String(Date.parse(value)) : value)
    }
    return request<SearchResult[]>('GET', `/api/search?${params}`)
  },
  replica: (id: string, after: number) => request<ReplicaDeltas>('GET', `${thread(id)}/replica?after=${after}`),
  gitText: (id: string, oid: string) => request<string>('GET', `${thread(id)}/git/${oid}`),
  sendDeltas: (id: string, deltas: string) => request<number>('POST', `${thread(id)}/replica`, { deltas }),
  presence: (id: string, state: { file: string | null; line: number | null; draft: string; leaving?: boolean }) =>
    request<PresenceReply>('POST', `${thread(id)}/presence`, state),
  live: (id: string) => request<LiveState>('GET', `${thread(id)}/live`),
  agents: () => request<AgentChoices>('GET', '/api/agents'),
  /** Starts `agent` in a thread, a clone, or with neither a new chat (`options.chat`, ADR 0059). */
  startAgent: (agent: string, where: { thread: string } | { clone: string } | Record<string, never>, options: Partial<StartOptions> = {}) =>
    request<string>('POST', '/api/agents/start', { agent, ...where, ...options }),
  chatSettings: () => request<ChatSettings>('GET', '/api/chats/settings'),
  setChatSettings: (settings: ChatSettings) => request<string>('PUT', '/api/chats/settings', settings),
  /** Makes a chat's folder a project of its own; answers with the project. */
  makeProject: (id: string) => request<string>('POST', `${thread(id)}/make-project`),
  // Claude Code cloud sessions, through the person's own `claude` (ADR 0040).
  cloudStart: (clone: string, task: string) => request<CloudLaunch>('POST', '/api/cloud', { clone, task }),
  cloudLaunches: () => request<CloudLaunch[]>('GET', '/api/cloud'),
  cloudSend: (thread: string, text: string) => request<null>('POST', `/api/threads/${thread}/cloud/send`, { text }),
  branches: (clone: string) => request<string[]>('GET', `/api/clones/branches?path=${encodeURIComponent(clone)}`),
  projects: () => request<Project[]>('GET', '/api/projects'),
  // Orgs, devices and notification settings are each server's own: the
  // chosen workspace's server answers (ADR 0032).
  orgs: (server: string | null = askedServer()) => request<OrgInfo[]>('GET', `/api/orgs${onServer(server)}`),
  orgChange: (change: OrgChange) => request<string>('POST', `/api/orgs/change${onServer(askedServer())}`, change),
  devices: () => request<DeviceView[]>('GET', `/api/devices${onServer(askedServer())}`),
  pairPhone: () => request<PhonePairing>('POST', `/api/pair${onServer(askedServer())}`),
  links: (id: string) => request<ThreadLink[]>('GET', `${thread(id)}/links`),
  makeLink: (id: string, link: NewLink) => request<ThreadLink>('POST', `${thread(id)}/links`, link),
  revokeLink: (id: string, link: number) => request<null>('POST', `${thread(id)}/links/${link}/revoke`),
  changeDevice: (id: string, change: DeviceChange) =>
    request<DeviceView>('PATCH', `/api/devices/${encodeURIComponent(id)}${onServer(askedServer())}`, change),
  revokeDevice: (id: string) =>
    request<null>('POST', `/api/devices/${encodeURIComponent(id)}/revoke${onServer(askedServer())}`),
  continueOn: (id: string, device: string, agent: string) =>
    request<string>('POST', `${thread(id)}/continue`, { device, agent }),
  setThreadState: (id: string, change: StateChange) => request<Mine>('PATCH', `${thread(id)}/state`, change),
  addClone: (path: string) => request<string>('POST', '/api/projects', { path }),
  found: () => request<FoundFolder[]>('GET', '/api/found'),
  importFound: (paths: string[] = []) => request<number>('POST', '/api/found/import', { paths }),
  cloneProject: (url: string, path: string) => request<string>('POST', '/api/projects/clone', { url, path }),
  newProject: (path: string) => request<string>('POST', '/api/projects/new', { path }),
  projectHome: (repo: string, home: string) => request<string>('PUT', '/api/projects/home', { repo, home }),
  // This machine's workspaces (ADR 0065).
  workspaces: () => request<WorkspacesView>('GET', '/api/workspaces'),
  // The person's design systems and Silt's own, for a workspace's default (ADR 0071).
  designSystems: (clone?: string) => request<DesignSystemView[]>('GET', clone ? `/api/design-systems?clone=${encodeURIComponent(clone)}` : '/api/design-systems'),
  artifactSettings: () => request<ArtifactSettings>('GET', '/api/artifact-settings'),
  setArtifactSettings: (settings: ArtifactSettings) => request<string>('PUT', '/api/artifact-settings', settings),
  projectDesignSystems: () => request<Record<string, string>>('GET', '/api/projects/design-systems'),
  setProjectDesignSystem: (repo: string, design_system: string | null) => request<string>('PUT', '/api/projects/design-systems', { repo, design_system }),
  setWorkspaces: (workspaces: Workspaces) => request<string>('PUT', '/api/workspaces', workspaces),
  /** Puts a project in a workspace, or with none back under the match rules. */
  assignProject: (repo: string, workspace: string | null) =>
    request<string>('POST', '/api/workspaces/assign', { repo, workspace }),
  status: () => request<DaemonStatus>('GET', '/api/status'),
  prompt: (id: string, text: string, plan = false, attachments: PromptAttachment[] = []) =>
    request<string>('POST', `${thread(id)}/agent/prompt`, { text, plan, attachments }),
  setMode: (id: string, mode: string) => request<string>('POST', `${thread(id)}/agent/mode`, { mode }),
  setSetting: (id: string, setting: string, value: string) =>
    request<string>('POST', `${thread(id)}/agent/setting`, { id: setting, value }),
  cancelTurn: (id: string) => request<string>('POST', `${thread(id)}/agent/cancel`),
  /** Ends a command the agent left running in the background, by the call that started it. */
  stopTask: (id: string, call: string) => request<string>('POST', `${thread(id)}/agent/tasks/${encodeURIComponent(call)}/stop`),
  answer: (id: string, permission: number, option: string | null) =>
    request<string>('POST', `${thread(id)}/agent/answer`, { id: permission, option }),
  /** Answers a question the agent waits on, each field by its chosen values; none declines it. */
  answerQuestion: (id: string, question: number, answers: Record<string, string[]> | null) =>
    request<string>('POST', `${thread(id)}/agent/reply`, { id: question, answers }),
  /** Signs the thread's agent in by one of the ways it lists; the sign-in runs on the agent's machine (ADR 0056). */
  signIn: (id: string, method: string) => request<string>('POST', `${thread(id)}/agent/sign-in`, { method }),
  /** Passes a code from the sign-in's page on to the sign-in. */
  signInCode: (id: string, code: string) => request<string>('POST', `${thread(id)}/agent/sign-in/code`, { code }),
  cancelSignIn: (id: string) => request<string>('POST', `${thread(id)}/agent/sign-in/cancel`),
  /** Pushes the thread's branch; answers the forge's page for its new pull request. */
  openPullRequest: (id: string) => request<string>('POST', `${thread(id)}/pull-request`),
  /** Why the thread can't open a pull request now, or `null` when it can. */
  pullRequestBlocker: (id: string) => request<string | null>('GET', `${thread(id)}/pull-request`),
  stopAgent: (id: string) => request<string>('POST', `${thread(id)}/agent/stop`),
  steer: (id: string, text: string) => request<string>('POST', `${thread(id)}/agent/steer`, { text }),
  setPreset: (id: string, preset: Preset) => request<string>('POST', `${thread(id)}/agent/preset`, { preset }),
  queue: (id: string) => request<Queued[]>('GET', `${thread(id)}/queue`),
  enqueue: (id: string, text: string) => request<number>('POST', `${thread(id)}/queue`, { text }),
  editQueued: (id: string, item: number, text: string) => request<string>('PUT', `${thread(id)}/queue/${item}`, { text }),
  moveQueued: (id: string, item: number, to: number) => request<string>('POST', `${thread(id)}/queue/${item}/move`, { to }),
  removeQueued: (id: string, item: number) => request<string>('DELETE', `${thread(id)}/queue/${item}`),
  sendQueued: (id: string, item: number) => request<string>('POST', `${thread(id)}/queue/${item}/send`),
  inbox: (all: boolean) => request<InboxEntry[]>('GET', `/api/inbox${all ? '?all=true' : ''}`),
  readNotifications: (read: Partial<InboxRead>, server?: string | null) =>
    request<string>('POST', `/api/inbox/read${onServer(server)}`, read),
  pushKey: () => request<string>('GET', '/api/push/key'),
  subscribePush: (subscription: PushSubscriptionJSON) => request<null>('POST', '/api/push/subscriptions', subscription),
  notificationSettings: () => request<KindSetting[]>('GET', `/api/me/notifications${onServer(askedServer())}`),
  setNotification: (setting: KindSetting) =>
    request<KindSetting[]>('PUT', `/api/me/notifications${onServer(askedServer())}`, setting),
  revert: (id: string, turn: number) => request<Reverted>('POST', `${thread(id)}/revert`, { turn }),
  fork: (id: string, turn?: number) => request<Resumed>('POST', `${thread(id)}/fork`, turn === undefined ? {} : { turn }),
  setVisibility: (id: string, visibility: string) => request<string>('POST', `${thread(id)}/visibility`, { visibility }),
  share: (id: string, user: string, remove: boolean) => request<string>('POST', `${thread(id)}/share`, { user, remove }),
  offerMergeRequest: (id: string, title: string) => request<MrSummary>('POST', `${thread(id)}/mrs`, { title }),
  mergeRequests: () => request<MrSummary[]>('GET', '/api/mrs'),
  mergeRequest: (id: string, hideGit = false) =>
    request<MrDetails>('GET', `/api/mrs/${encodeURIComponent(id)}${hideGit ? '?git=hide' : ''}`),
  approve: (id: string) => request<MrSummary>('POST', `/api/mrs/${encodeURIComponent(id)}/approve`),
  writeGuide: (id: string, text?: string) =>
    request<string>('POST', `/api/mrs/${encodeURIComponent(id)}/guide`, text === undefined ? {} : { text }),
  markSeen: (id: string, path: string, seen: boolean) =>
    request<string>('PUT', `/api/mrs/${encodeURIComponent(id)}/seen`, { path, seen }),
  requestChanges: (id: string, reason: string) =>
    request<MrSummary>('POST', `/api/mrs/${encodeURIComponent(id)}/request-changes`, { reason }),
  decline: (id: string, reason: string) =>
    request<MrSummary>('POST', `/api/mrs/${encodeURIComponent(id)}/decline`, { reason }),
}

export function short(id: string): string {
  return id.slice(-12)
}

/** Agent CLIs Silt can continue a thread with. */
export const AGENTS = ['claude', 'codex', 'gemini', 'agy', 'opencode', 'copilot'] as const
