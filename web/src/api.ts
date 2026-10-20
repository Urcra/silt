// Talks to the daemon that served this page (ADR 0011). The token arrives
// once in the URL fragment from `silt open` and is kept for the tab.

const TOKEN_KEY = 'silt-token'

export function takeTokenFromUrl(): void {
  const match = window.location.hash.match(/token=([0-9a-f]+)/)
  if (match) {
    sessionStorage.setItem(TOKEN_KEY, match[1])
    window.location.hash = '#/'
  }
}

export class ApiError extends Error {}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method,
    headers: {
      Authorization: `Bearer ${sessionStorage.getItem(TOKEN_KEY) ?? ''}`,
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

export interface ThreadSummary {
  id: string
  repo: string
  title: string | null
  /** The command of the agent that recorded the latest session. */
  agent: string | null
  created_ms: number
  messages: number
  edits: number
  location: 'here' | 'remote' | 'moved' | 'diverged'
  forked_from: string | null
}

export interface HistoryEntry {
  index: number
  thread: string
  origin: 'own' | 'inherited' | 'upstream'
  kind: string
  text: string
  time_ms: number
}

export interface HistoryPage {
  thread: string
  total: number
  entries: HistoryEntry[]
}

export interface TurnInfo {
  number: number
  entry: number | null
  prompt: string
}

export interface SearchResult {
  thread: string
  kind: string
  snippet: string
  owner?: string | null
  status?: string | null
  repo?: string | null
}

export interface SearchFilters {
  repo?: string
  author?: string
  status?: string
  branch?: string
  file?: string
  since?: string
}

export interface MrSummary {
  id: string
  fork: string
  target: string
  title: string
  summary: string
  status: string
  opened_ms: number
}

export type ShowLine =
  | { line: 'entry'; kind: string; text: string }
  | { line: 'tool_call'; id: string; name: string; input: string }
  | { line: 'edits'; tool_use_id: string | null; files: { path: string; inserted: number; deleted: number }[] }
  | { line: 'compaction'; summary: string | null }
  | { line: 'turn'; mark: string }
  | { line: 'segment'; label: string }
  | { line: 'event'; text: string }

export interface MrDetails {
  mr: MrSummary
  diff: string
  conflicts: string[]
  conversation: ShowLine[]
}

export type CommentLocation =
  | { at: 'entry'; index: number; excerpt: string }
  | { at: 'lines'; path: string; start: number; end: number; outdated: boolean }
  | { at: 'unknown' }

export interface CommentView {
  id: string
  by: string
  time_ms: number
  text: string
  resolved: boolean
  location: CommentLocation
}

export type CommentOn =
  | { on: 'entry'; index: number }
  | { on: 'lines'; path: string; start: number; end: number }

const thread = (id: string) => `/api/threads/${encodeURIComponent(id)}`

export interface Notification {
  id: number
  kind: 'mr_opened' | 'mr_approved' | 'mr_declined' | 'comment' | 'mention'
  thread: string
  actor: string
  text: string
  created_ms: number
  read: boolean
}

export const api = {
  threads: () => request<ThreadSummary[]>('GET', '/api/threads'),
  history: (id: string) => request<HistoryPage>('GET', `${thread(id)}/history`),
  turns: (id: string) => request<TurnInfo[]>('GET', `${thread(id)}/turns`),
  diff: (id: string, options: { turn?: number; base?: string } = {}) => {
    const params = new URLSearchParams()
    if (options.turn !== undefined) params.set('turn', String(options.turn))
    if (options.base) params.set('base', options.base)
    return request<string>('GET', `${thread(id)}/diff?${params}`)
  },
  comments: (id: string) => request<CommentView[]>('GET', `${thread(id)}/comments`),
  addComment: (id: string, on: CommentOn, text: string) =>
    request<CommentView>('POST', `${thread(id)}/comments`, { on, text }),
  resolveComment: (id: string, comment: string) =>
    request<CommentView>('POST', `${thread(id)}/comments/${encodeURIComponent(comment)}/resolve`),
  search: (query: string, filters: SearchFilters) => {
    const params = new URLSearchParams({ q: query })
    for (const [name, value] of Object.entries(filters)) {
      if (!value) continue
      params.set(name, name === 'since' ? String(Date.parse(value)) : value)
    }
    return request<SearchResult[]>('GET', `/api/search?${params}`)
  },
  inbox: (all: boolean) =>
    request<Notification[]>('GET', `/api/inbox${all ? '?all=true' : ''}`),
  readNotifications: (read: { ids?: number[]; all?: boolean }) => request<string>('POST', '/api/inbox/read', read),
  mergeRequests: () => request<MrSummary[]>('GET', '/api/mrs'),
  mergeRequest: (id: string) => request<MrDetails>('GET', `/api/mrs/${encodeURIComponent(id)}`),
  approve: (id: string) => request<MrSummary>('POST', `/api/mrs/${encodeURIComponent(id)}/approve`),
  decline: (id: string, reason: string) =>
    request<MrSummary>('POST', `/api/mrs/${encodeURIComponent(id)}/decline`, { reason }),
}

export function short(id: string): string {
  return id.slice(-12)
}

/** Agent CLIs Silt can continue a thread with. */
export const AGENTS = ['claude', 'codex', 'gemini', 'opencode', 'copilot'] as const
