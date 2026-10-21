// Talks to the daemon that served this page (ADR 0011). The token arrives
// once in the URL fragment from `silt open`, or is traded for the one-time
// code a browser opened by `silt open` carries, and is kept for the tab.

import type {
  AgentChoices,
  CommentOn,
  CommentView,
  HistoryPage,
  InboxEntry,
  InboxRead,
  MrDetails,
  MrSummary,
  PresenceReply,
  ReplicaDeltas,
  SearchResult,
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
    const response = await fetch('/api/exchange', {
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
  liveUrl: (id: string) => `${thread(id)}/live?token=${token()}`,
  agents: () => request<AgentChoices>('GET', '/api/agents'),
  startAgent: (agent: string, where: { thread: string } | { clone: string }) =>
    request<string>('POST', '/api/agents/start', { agent, ...where }),
  prompt: (id: string, text: string) => request<string>('POST', `${thread(id)}/agent/prompt`, { text }),
  cancelTurn: (id: string) => request<string>('POST', `${thread(id)}/agent/cancel`),
  answer: (id: string, permission: number, option: string | null) =>
    request<string>('POST', `${thread(id)}/agent/answer`, { id: permission, option }),
  stopAgent: (id: string) => request<string>('POST', `${thread(id)}/agent/stop`),
  inbox: (all: boolean) => request<InboxEntry[]>('GET', `/api/inbox${all ? '?all=true' : ''}`),
  readNotifications: (read: Partial<InboxRead>) => request<string>('POST', '/api/inbox/read', read),
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
