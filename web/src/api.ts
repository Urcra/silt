// Talks to the daemon that served this page (ADR 0011), or the server when
// it hosts the app (ADR 0026), with its session cookie. A daemon's token
// arrives once in the URL fragment from `silt open`, or is traded for the
// one-time code a browser opened by `silt open` carries, and is kept for the tab.

import { apiUrl } from './served'
import { askedServer } from './workspace'
import type {
  AgentChoices,
  CloudLaunch,
  CommentOn,
  CommentView,
  CommitView,
  DaemonStatus,
  FileAt,
  FoundFolder,
  DeviceChange,
  DeviceView,
  GraphQuery,
  GraphRow,
  HistoryPage,
  InboxEntry,
  InboxRead,
  KindSetting,
  LiveState,
  Mine,
  MrDetails,
  MrSummary,
  NewLink,
  OrgChange,
  OrgInfo,
  PhonePairing,
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
  ThreadSummary,
  TurnInfo,
} from './api.generated'

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

export class ApiError extends Error {}

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
    throw new ApiError((data as { error?: string }).error ?? `request failed (${response.status})`)
  }
  return data as T
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
/** Names which server to ask, on a machine signed in to several (ADR 0032). */
const onServer = (server?: string | null) => (server ? `?server=${encodeURIComponent(server)}` : '')

export const api = {
  threads: () => request<ThreadSummary[]>('GET', '/api/threads'),
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
  /** What the thread changed; with `limit`, only that many files from the `offset`th, in path order. */
  diff: (id: string, options: { turn?: number; base?: string; offset?: number; limit?: number } = {}) => {
    const params = new URLSearchParams()
    if (options.turn !== undefined) params.set('turn', String(options.turn))
    if (options.base) params.set('base', options.base)
    if (options.limit !== undefined) {
      params.set('offset', String(options.offset ?? 0))
      params.set('limit', String(options.limit))
    }
    return request<string>('GET', `${thread(id)}/diff?${params}`)
  },
  comments: (id: string) => request<CommentView[]>('GET', `${thread(id)}/comments`),
  addComment: (id: string, on: CommentOn, text: string) =>
    request<CommentView>('POST', `${thread(id)}/comments`, { on, text }),
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
  startAgent: (agent: string, where: { thread: string } | { clone: string }, options: Partial<StartOptions> = {}) =>
    request<string>('POST', '/api/agents/start', { agent, ...where, ...options }),
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
  status: () => request<DaemonStatus>('GET', '/api/status'),
  prompt: (id: string, text: string, plan = false, attachments: PromptAttachment[] = []) =>
    request<string>('POST', `${thread(id)}/agent/prompt`, { text, plan, attachments }),
  setMode: (id: string, mode: string) => request<string>('POST', `${thread(id)}/agent/mode`, { mode }),
  setSetting: (id: string, setting: string, value: string) =>
    request<string>('POST', `${thread(id)}/agent/setting`, { id: setting, value }),
  cancelTurn: (id: string) => request<string>('POST', `${thread(id)}/agent/cancel`),
  answer: (id: string, permission: number, option: string | null) =>
    request<string>('POST', `${thread(id)}/agent/answer`, { id: permission, option }),
  /** Answers a question the agent waits on, each field by its chosen values; none declines it. */
  answerQuestion: (id: string, question: number, answers: Record<string, string[]> | null) =>
    request<string>('POST', `${thread(id)}/agent/reply`, { id: question, answers }),
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
  mergeRequest: (id: string) => request<MrDetails>('GET', `/api/mrs/${encodeURIComponent(id)}`),
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
export const AGENTS = ['claude', 'codex', 'gemini', 'opencode', 'copilot'] as const
