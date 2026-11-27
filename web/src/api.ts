// Talks to the daemon that served this page (ADR 0011), or the server when
// it hosts the app (ADR 0026), with its session cookie. A daemon's token
// arrives once in the URL fragment from `silt open`, or is traded for the
// one-time code a browser opened by `silt open` carries, and is kept for the tab.

import { apiUrl, hosted } from './served'
import { askedServer } from './workspace'
import type {
  AskView,
  Judge,
  AppearanceMode,
  AppearanceSettings,
  TextSettings,
  ThemeCatalog,
  ThemeDetail,
  ThemeSaved,
  AgentChoices,
  AgentUsage,
  ArtifactContent,
  ArtifactListed,
  ConnectorTool,
  PageConnector,
  ArtifactSettings,
  NetworkSettings,
  NetworkView,
  MemoryOverview,
  MemoryView,
  ChatSettings,
  CloudLaunch,
  DesignSystemView,
  DesignCatalog,
  DesignDetail,
  DesignAssetData,
  DesignStart,
  DesignSummary,
  ThreadDesign,
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
  Subfolder,
  GraphQuery,
  GraphRow,
  HistoryPage,
  InboxEntry,
  InboxRead,
  FoundModel,
  KindSetting,
  LiveState,
  LocalModels,
  MarkSeen,
  Mine,
  ModelFiles,
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
  AccessView,
  ApprovalView,
  GrantScope,
  Installed,
  PluginView,
  PluginActivity,
  GitUpdate,
  Preview,
  Rule,
  VoiceStatus,
  UpdateView,
  Workspaces,
  WorkspacesView,
  Capabilities,
  LanView,
  BlobState,
} from './api.generated'

/** Who views a thread's pages: their id in page data, their level, and the names of the people the thread knows. */
/** A tool a page offers the agent answering its question; the page runs it (ADR 0072). */
export type SampleTool = { name: string; description: string; input_schema?: unknown }
export type PageViewer = { id: string; level: 'read' | 'comment' | 'collaborator' | 'owner'; people: Record<string, string> }

export type * from './api.generated'

/** The phone asking, on the network listener (ADR 0112). */
export type LanMe = {
  phone: string
  name: string | null
  machine: string
  capabilities: Capabilities
  notifies: boolean
  push_key: string | null
}

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

/** How long a presence beat waits for its answer. */
const PRESENCE_MS = 10_000

/** Reads under way, given up together once the page wakes or finds its connection again ([`abortReads`]). */
let reads = new AbortController()

/**
 * Gives up every read under way. One sent before a phone slept or lost its network can wait on a dead connection
 * long after the network is back, and what it reads can't refresh until it ends; while it waits it also holds one of
 * the few connections a browser opens to the machine.
 */
export function abortReads() {
  reads.abort()
  reads = new AbortController()
}

/** A signal that aborts once any of `signals` does. */
function either(signals: (AbortSignal | undefined)[]): AbortSignal {
  const given = signals.filter((signal) => signal !== undefined)
  if (given.length === 1) return given[0]
  const both = new AbortController()
  for (const signal of given) {
    if (signal.aborted) both.abort(signal.reason)
    else signal.addEventListener('abort', () => both.abort(signal.reason), { once: true })
  }
  return both.signal
}

async function request<T>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const response = await fetch(apiUrl(path), {
    method,
    // A change isn't given up with the reads: it may have been made, and sending it again could make it twice.
    signal: method === 'GET' ? either([reads.signal, signal]) : signal,
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

/**
 * A blob of the thread's, raw, for an `<img>` or a `<video>` to read by range (ADR 0121); `preview` asks for an
 * image's preview of that many px, `download` saves it under that name. The daemon takes its token in the query, as
 * an element can't send a header; the server and a phone go by their cookie.
 */
export function blobUrl(id: string, blob: string, options: { preview?: number; download?: string } = {}): string {
  const query = new URLSearchParams()
  if (options.preview) query.set('preview', String(options.preview))
  if (options.download) query.set('download', options.download)
  const given = token()
  if (given) query.set('token', given)
  const search = query.toString()
  return apiUrl(`${thread(id)}/blobs/${blob}`) + (search ? `?${search}` : '')
}

/** What the daemon reads of each file a prompt carries, and nothing the page keeps beside it. */
function sentFiles(files: PromptAttachment[]): PromptAttachment[] {
  return files.map(({ name, mime, data, upload }) => ({ name, mime, data, upload: upload ?? null }))
}

/** What an upload answered: the file, by the name a prompt then gives it. */
export interface Uploaded {
  upload: string
  size: number
}

/**
 * Sends a file a prompt will carry to the machine ahead of the prompt, streaming it with no size limit (ADR 0121),
 * telling `progress` how much has gone.
 */
export function uploadFile(file: Blob, progress: (sent: number) => void, signal?: AbortSignal): Promise<Uploaded> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest()
    xhr.open('POST', apiUrl('/api/uploads'))
    xhr.setRequestHeader('Authorization', `Bearer ${token()}`)
    xhr.setRequestHeader('X-Silt-Request', '1')
    xhr.setRequestHeader('Content-Type', 'application/octet-stream')
    xhr.upload.onprogress = (event) => progress(event.loaded)
    xhr.onload = () => {
      let answer: { upload?: string; size?: number; error?: string } = {}
      try {
        answer = JSON.parse(xhr.responseText) as typeof answer
      } catch {
        // Not JSON: the status says what went wrong.
      }
      if (xhr.status >= 200 && xhr.status < 300 && answer.upload) resolve({ upload: answer.upload, size: answer.size ?? 0 })
      else reject(new ApiError(answer.error ?? `upload failed (${xhr.status})`))
    }
    xhr.onerror = () => reject(new ApiError('the upload was cut off'))
    xhr.onabort = () => reject(new DOMException('the upload was stopped', 'AbortError'))
    signal?.addEventListener('abort', () => xhr.abort(), { once: true })
    xhr.send(file)
  })
}

/** Where a design system is: a project's clone, the person's own, or the folder of the thread making it. */
export interface DesignWhere {
  source: 'project' | 'personal'
  name: string
  clone?: string
  thread?: string
}

function designQuery(where: DesignWhere): string {
  const query = new URLSearchParams({ source: where.source, name: where.name })
  if (where.clone) query.set('clone', where.clone)
  if (where.thread) query.set('thread', where.thread)
  return query.toString()
}

/** What a diff leaves out: with `hideGit`, what git itself rewrote (ADR 0069); with `hideOthers`, the edits of other
 * agents working in the same clone (ADR 0089). */
export interface Hidden {
  hideGit?: boolean
  hideOthers?: boolean
}

const hiddenParams = (params: URLSearchParams, hidden: Hidden) => {
  if (hidden.hideGit) params.set('git', 'hide')
  if (hidden.hideOthers) params.set('others', 'hide')
  return params
}

/** The same, as a comment's or its anchors' lines say how they're numbered. */
export const hiddenLines = (hidden: Hidden) => ({ hide_git: hidden.hideGit ?? false, hide_others: hidden.hideOthers ?? false })

/** Which of a thread's changes a diff shows: those of `turn`, or of turns `turn` to `toTurn`, else since `base`. */
export interface DiffSpec extends Hidden {
  turn?: number
  toTurn?: number
  base?: string
}

const diffParams = (options: DiffSpec) => {
  const params = new URLSearchParams()
  if (options.turn !== undefined) params.set('turn', String(options.turn))
  if (options.toTurn !== undefined) params.set('to_turn', String(options.toTurn))
  if (options.base) params.set('base', options.base)
  return hiddenParams(params, options)
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
  /** The local models here, and the llama-server that runs them (ADR 0092); a daemon's only. */
  models: () => request<LocalModels>('GET', '/api/models'),
  /** Starts a download: `owner/repo:file.gguf`, or a GGUF file's path. Answers with the model's name. */
  pullModel: (source: string) => request<string>('POST', '/api/models/pull', { source }),
  removeModel: (id: string) => request<string>('DELETE', `/api/models/${encodeURIComponent(id)}`),
  /** GGUF models on Hugging Face, the most downloaded first; with no words, the most downloaded of all. */
  searchModels: (words: string) => request<FoundModel[]>('GET', `/api/models/search?q=${encodeURIComponent(words)}`),
  /** A Hugging Face repository's quantizations, as this machine would run each. */
  modelFiles: (repo: string) => request<ModelFiles>('GET', `/api/models/files?repo=${encodeURIComponent(repo)}`),
  /** Starts downloading llama.cpp's latest build for this machine. */
  pullRuntime: () => request<string>('POST', '/api/models/runtime'),
  /** Whether a newer Silt is out and how an update goes (ADR 0100), for the desktop app built from `desktop`; a daemon's only. */
  update: (desktop?: string) =>
    request<UpdateView>('GET', `/api/update${desktop ? `?desktop=${encodeURIComponent(desktop)}` : ''}`),
  checkUpdate: () => request<string>('POST', '/api/update/check'),
  startUpdate: () => request<string>('POST', '/api/update/start'),
  /** Restarts the daemon on the installed update now, ending agents' turns. */
  restartForUpdate: () => request<string>('POST', '/api/update/restart'),
  /** Erases a Local only thread and its forks (ADR 0092); unconfirmed, says what would go. */
  eraseThread: (id: string, confirm: boolean) => request<string>('POST', `${thread(id)}/erase`, { confirm }),
  threads: () => request<ThreadSummary[]>('GET', '/api/threads'),
  /** How much of a blob of the thread's has arrived, while another machine uploads it (ADR 0121). */
  blobState: (id: string, blob: string) => request<BlobState>('GET', `${thread(id)}/blobs/${blob}/state`),
  /** An artifact's content, by the blob a version of it names (ADR 0068). */
  artifact: (id: string, blob: string) => request<ArtifactContent>('GET', `${thread(id)}/artifacts/${blob}`),
  /** A read or write of an artifact's page data (ADR 0072). */
  pageData: (id: string, artifact: string, asked: Record<string, unknown>) =>
    request<unknown>('POST', `${thread(id)}/artifacts/${artifact}/data`, asked),
  /** What the person allowed or refused an artifact's page, by capability (ADR 0072). */
  artifactGrants: (id: string, artifact: string) =>
    request<Record<string, 'granted' | 'refused'>>('GET', `${thread(id)}/artifacts/${artifact}/grants`),
  /** Kept for what `version`, the one the person saw, declares of `capability`. */
  setArtifactGrant: (id: string, artifact: string, capability: string, state: 'granted' | 'refused' | null, version?: number) =>
    request<unknown>('PUT', `${thread(id)}/artifacts/${artifact}/grants`, { capability, state, version: version ?? null }),
  /** A version an HTML artifact's page saves itself as; refused as a conflict unless `base` is the latest (ADR 0072). */
  publishArtifact: (id: string, artifact: string, html: string, base: number) =>
    request<{ id: string; version: number }>('POST', `${thread(id)}/artifacts/${artifact}/publish`, { html, base }),
  /** The files people added to an artifact's page (ADR 0074). */
  pageAssets: (
    id: string,
    artifact: string,
    asked: { op: 'add'; name: string; media_type: string; data: string } | { op: 'list' } | { op: 'delete'; id: string },
  ) => request<unknown>('POST', `${thread(id)}/artifacts/${artifact}/assets`, asked),
  /** The thread's files, for a page that declares `files` (ADR 0078). */
  pageFiles: (id: string, artifact: string, asked: Record<string, unknown>) =>
    request<unknown>('POST', `${thread(id)}/artifacts/${artifact}/files`, asked),
  /** Every artifact in the threads the person can read, for the Artifacts page (ADR 0075). */
  artifacts: () => request<ArtifactListed[]>('GET', '/api/artifacts'),
  /** An artifact's thumbnail, drawn the first time it's asked for where a headless browser is installed (ADR 0075). */
  artifactThumbnail: (id: string, artifact: string) =>
    request<ArtifactContent>('GET', `${thread(id)}/artifacts/${artifact}/thumbnail`),
  pinArtifact: (id: string, artifact: string, pinned: boolean) =>
    request<unknown>('PUT', `${thread(id)}/artifacts/${artifact}/pin`, { pinned }),
  /** Hides an artifact for everyone, or shows it again; the thread's owner's to do. */
  archiveArtifact: (id: string, artifact: string, archived: boolean) =>
    request<unknown>('PUT', `${thread(id)}/artifacts/${artifact}/archive`, { archived }),
  /** A page's call of the viewer's page connectors (ADR 0080). */
  pageMcp: (id: string, artifact: string, asked: Record<string, unknown>) =>
    request<unknown>('POST', `${thread(id)}/artifacts/${artifact}/mcp`, asked),
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
  sheetData: (id: string, artifact: string, asked: Record<string, unknown>) =>
    request<unknown>('POST', `${thread(id)}/artifacts/${artifact}/sheet`, asked),
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
  artifactPdf: (id: string, artifact: string, html: string) =>
    request<ArtifactContent>('POST', `${thread(id)}/artifacts/${artifact}/pdf`, { html }),
  /** A page's answer to the agent's call of a tool it gave its question. */
  /** Stops a page's question while this machine's agent answers it. */
  sampleStop: (id: string, artifact: string, ask: string) =>
    request<null>('POST', `${thread(id)}/artifacts/${artifact}/sample/${encodeURIComponent(ask)}/stop`),
  sampleToolResult: (
    id: string,
    artifact: string,
    ask: string,
    call: number,
    result: { ok: boolean; value?: unknown; error?: string },
  ) => request<null>('POST', `${thread(id)}/artifacts/${artifact}/sample/${encodeURIComponent(ask)}/tools/${call}`, result),
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
  artifactFrame: (
    id: string,
    artifact: string,
    ask: { version: number; theme: 'light' | 'dark'; parent: string; boot?: unknown },
  ) => request<{ path: string; expires_in_ms: number }>('POST', `${thread(id)}/artifacts/${artifact}/frame`, ask),
  /** A thread's history; with `limit`, only that many entries from the `offset`th, or from where it changed since
   * the version `known`; with `tail`, the last that many before entry `before` (else the end), from the start of
   * their turn. A page leaves out images and the start of long outputs unless asked for `whole`. */
  history: (
    id: string,
    offset?: number,
    limit?: number,
    options: { known?: number | null; whole?: boolean; tail?: number; before?: number; signal?: AbortSignal } = {},
  ) => {
    const query = new URLSearchParams()
    if (limit !== undefined) query.set('offset', String(offset ?? 0))
    if (limit !== undefined) query.set('limit', String(limit))
    if (options.known !== undefined && options.known !== null) query.set('known', String(options.known))
    if (options.whole) query.set('whole', 'true')
    if (options.tail !== undefined) query.set('tail', String(options.tail))
    if (options.before !== undefined) query.set('before', String(options.before))
    const asked = query.toString()
    return request<HistoryPage>('GET', `${thread(id)}/history${asked ? `?${asked}` : ''}`, undefined, options.signal)
  },
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
  diffStats: (id: string, options: DiffSpec = {}) =>
    request<DiffStat[]>('GET', `${thread(id)}/diff/stats?${diffParams(options)}`),
  /** The diff's files this person marked seen: each as they saw it, or changed since, with the lines new since (ADR 0067). */
  threadSeen: (id: string, options: DiffSpec = {}) => request<ThreadSeenView>('GET', `${thread(id)}/seen?${diffParams(options)}`),
  markThreadSeen: (id: string, mark: MarkSeen) => request<string>('PUT', `${thread(id)}/seen`, mark),
  /** For the diff's files, the turns that last wrote their new lines. */
  diffTurns: (id: string, options: DiffSpec = {}) =>
    request<FileTurns[]>('GET', `${thread(id)}/diff/turns?${diffParams(options)}`),
  /** With `base`, a diff base, code comments say where they are there too. */
  comments: (id: string, base?: string, hidden: Hidden = {}) => {
    const params = new URLSearchParams()
    if (base) params.set('base', base)
    const query = hiddenParams(params, hidden).toString()
    return request<CommentView[]>('GET', `${thread(id)}/comments${query ? `?${query}` : ''}`)
  },
  commentsSent: (id: string, comments: string[]) => request<CommentView[]>('POST', `${thread(id)}/comments/sent`, { comments }),
  addComment: (id: string, on: CommentOn, text: string) => request<CommentView>('POST', `${thread(id)}/comments`, { on, text }),
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
    // The heartbeat sends again every few seconds, so one that hangs is given up rather than holding a connection.
    request<PresenceReply>('POST', `${thread(id)}/presence`, state, AbortSignal.timeout(PRESENCE_MS)),
  live: (id: string) => request<LiveState>('GET', `${thread(id)}/live`),
  agents: () => request<AgentChoices>('GET', '/api/agents'),
  /** How much of its plan `agent` has used, with the sign-in it runs with in `thread` (ADR 0095); a daemon's only. */
  usage: (agent: string, thread?: string) =>
    request<AgentUsage>(
      'GET',
      `/api/agents/${encodeURIComponent(agent)}/usage${thread ? `?thread=${encodeURIComponent(thread)}` : ''}`,
    ),
  /** Starts `agent` in a thread, a clone, or with neither a new chat (`options.chat`, ADR 0059). */
  startAgent: (
    agent: string,
    where: { thread: string } | { clone: string } | Record<string, never>,
    options: Partial<StartOptions> = {},
  ) =>
    request<string>('POST', '/api/agents/start', {
      agent,
      ...where,
      ...options,
      ...(options.attachments ? { attachments: sentFiles(options.attachments) } : {}),
    }),
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
  revokeDevice: (id: string) => request<null>('POST', `/api/devices/${encodeURIComponent(id)}/revoke${onServer(askedServer())}`),
  continueOn: (id: string, device: string, agent: string) => request<string>('POST', `${thread(id)}/continue`, { device, agent }),
  setThreadState: (id: string, change: StateChange) => request<Mine>('PATCH', `${thread(id)}/state`, change),
  addClone: (path: string) => request<string>('POST', '/api/projects', { path }),
  found: () => request<FoundFolder[]>('GET', '/api/found'),
  subfolders: (dir: string) => request<Subfolder[]>('GET', `/api/folders?dir=${encodeURIComponent(dir)}`),
  importFound: (paths: string[] = []) => request<number>('POST', '/api/found/import', { paths }),
  cloneProject: (url: string, path: string) => request<string>('POST', '/api/projects/clone', { url, path }),
  newProject: (path: string) => request<string>('POST', '/api/projects/new', { path }),
  projectHome: (repo: string, home: string) => request<string>('PUT', '/api/projects/home', { repo, home }),
  // This machine's workspaces (ADR 0065).
  workspaces: () => request<WorkspacesView>('GET', '/api/workspaces'),
  // The person's design systems and Silt's own, for a workspace's default (ADR 0071).
  designSystems: (clone?: string) =>
    request<DesignSystemView[]>('GET', clone ? `/api/design-systems?clone=${encodeURIComponent(clone)}` : '/api/design-systems'),
  // Settings › Design systems: every system, one system's page, its assets, and the threads that make one (ADR 0088).
  designCatalog: () => request<DesignCatalog>('GET', '/api/design-systems/catalog'),
  designSystem: (where: DesignWhere) => request<DesignDetail>('GET', `/api/design-systems/system?${designQuery(where)}`),
  designAsset: (where: DesignWhere, path: string) =>
    request<DesignAssetData>('GET', `/api/design-systems/asset?${designQuery(where)}&path=${encodeURIComponent(path)}`),
  designStart: (start: DesignStart) => request<string>('POST', '/api/design-systems/start', start),
  threadDesign: (id: string) => request<ThreadDesign | null>('GET', `${thread(id)}/design-system`),
  designKeep: (thread: string, name: string) => request<DesignSummary>('POST', '/api/design-systems/keep', { thread, name }),
  themes: () => request<ThemeCatalog>('GET', '/api/themes'),
  themeDetail: (id: string) => request<ThemeDetail>('GET', `/api/themes/detail?id=${encodeURIComponent(id)}`),
  themeCss: (pick: { light?: string; dark?: string; text?: TextSettings } = {}) => {
    const { text, ...themes } = pick
    const query = new URLSearchParams(Object.entries(themes).filter((entry): entry is [string, string] => Boolean(entry[1])))
    if (text) query.set('text', JSON.stringify(text))
    return request<{ css: string; mode: AppearanceMode }>('GET', `/api/themes/css${query.size ? `?${query}` : ''}`)
  },
  setAppearance: (settings: AppearanceSettings) => request<string>('PUT', '/api/appearance', settings),
  importTheme: (text: string) => request<ThemeSaved>('POST', '/api/themes/import', { text }),
  duplicateTheme: (from: string) => request<ThemeSaved>('POST', '/api/themes/duplicate', { from }),
  memory: () => request<MemoryOverview>('GET', '/api/memory'),
  /** What's remembered in the project whose clone is `clone`, or everywhere without one. */
  memoryOf: (clone: string | null) =>
    request<MemoryView>('GET', `/api/memory/store${clone ? `?clone=${encodeURIComponent(clone)}` : ''}`),
  setMemory: (change: { clone: string | null; store: string; file: string | null; text: string }) =>
    request<string>('PUT', '/api/memory/store', change),
  forgetMemory: (change: { clone: string | null; store: string; file: string }) =>
    request<string>('DELETE', '/api/memory/store', change),
  /** Moves a memory to the memory everywhere, or to the project whose clone is `to`. */
  moveMemory: (change: { clone: string | null; store: string; file: string; to: string | null }) =>
    request<string>('POST', '/api/memory/move', change),
  networkSettings: () => request<NetworkView>('GET', '/api/network-settings'),
  setNetworkSettings: (settings: NetworkSettings) => request<string>('PUT', '/api/network-settings', settings),
  artifactSettings: () => request<ArtifactSettings>('GET', '/api/artifact-settings'),
  setArtifactSettings: (settings: ArtifactSettings) => request<string>('PUT', '/api/artifact-settings', settings),
  projectDesignSystems: () => request<Record<string, string>>('GET', '/api/projects/design-systems'),
  setProjectDesignSystem: (repo: string, design_system: string | null) =>
    request<string>('PUT', '/api/projects/design-systems', { repo, design_system }),
  setWorkspaces: (workspaces: Workspaces) => request<string>('PUT', '/api/workspaces', workspaces),
  /** Puts a project in a workspace, or with none back under the match rules. */
  assignProject: (repo: string, workspace: string | null) =>
    request<string>('POST', '/api/workspaces/assign', { repo, workspace }),
  // Plugin calls waiting for the person, and this machine's plugins (ADR 0093).
  approvals: (thread?: string) =>
    request<ApprovalView[]>('GET', thread ? `/api/approvals?thread=${encodeURIComponent(thread)}` : '/api/approvals'),
  /** Answers an approval, naming the hash of the request shown, so it answers only that. */
  answerApproval: (id: number, approve: boolean, hash: string, grant: GrantScope | null = null) =>
    request<string>('POST', `/api/approvals/${id}`, { approve, hash, grant }),
  revokeGrant: (id: number) => request<string>('DELETE', `/api/grants/${id}`),
  access: (thread: string) => request<AccessView>('GET', `/api/threads/${encodeURIComponent(thread)}/access`),
  unfolder: (thread: string, path: string) =>
    request<string>('DELETE', `/api/threads/${encodeURIComponent(thread)}/folders`, { path }),
  releaseDevice: (thread: string, device: string) =>
    request<string>('POST', `/api/threads/${encodeURIComponent(thread)}/devices/release`, { device }),
  plugins: () => request<PluginView[]>('GET', '/api/plugins'),
  setPluginRule: (id: string, action: string, rule: Rule | null, matching: { field: string; pattern: string } | null) =>
    request<string>('PUT', `/api/plugins/${encodeURIComponent(id)}/rules`, { action, rule, ...matching }),
  setPluginSetting: (id: string, key: string, value: unknown, environment: string | null, workspace: string | null) =>
    request<string>('PUT', `/api/plugins/${encodeURIComponent(id)}/settings`, { key, value, environment, workspace }),
  setPluginEnvironment: (id: string, name: string, patterns: string[]) =>
    request<string>('PUT', `/api/plugins/${encodeURIComponent(id)}/environments/${encodeURIComponent(name)}`, { patterns }),
  trustPublisher: (publisher: string, trusted: boolean) =>
    request<string>('PUT', `/api/publishers/${encodeURIComponent(publisher)}`, { trusted }),
  previewPlugin: (url: string) => request<Preview>('POST', '/api/plugins/preview', { url }),
  addPlugin: (url: string, commit: string) => request<Installed>('POST', '/api/plugins/add', { url, commit }),
  checkPluginUpdates: () => request<GitUpdate[]>('POST', '/api/plugins/check'),
  updatePlugin: (id: string, consented: boolean) =>
    request<Installed>('POST', `/api/plugins/${encodeURIComponent(id)}/update`, { consented }),
  pluginActivity: (id: string) => request<PluginActivity>('GET', `/api/plugins/${encodeURIComponent(id)}/activity`),
  removePlugin: (id: string) => request<string>('DELETE', `/api/plugins/${encodeURIComponent(id)}`),
  turnPlugin: (id: string, workspace: string, on: boolean) =>
    request<string>('PUT', `/api/plugins/${encodeURIComponent(id)}/workspaces/${encodeURIComponent(workspace)}`, { on }),
  status: () => request<DaemonStatus>('GET', '/api/status'),
  prompt: (id: string, text: string, plan = false, attachments: PromptAttachment[] = []) =>
    request<string>('POST', `${thread(id)}/agent/prompt`, { text, plan, attachments: sentFiles(attachments) }),
  setMode: (id: string, mode: string) => request<string>('POST', `${thread(id)}/agent/mode`, { mode }),
  setSetting: (id: string, setting: string, value: string) =>
    request<string>('POST', `${thread(id)}/agent/setting`, { id: setting, value }),
  cancelTurn: (id: string) => request<string>('POST', `${thread(id)}/agent/cancel`),
  /** Ends a command the agent left running in the background, by the call that started it. */
  stopTask: (id: string, call: string) => request<string>('POST', `${thread(id)}/agent/tasks/${encodeURIComponent(call)}/stop`),
  answer: (id: string, permission: number, option: string | null) =>
    request<string>('POST', `${thread(id)}/agent/answer`, { id: permission, option }),
  /** The person's answer to a call Auto refused that the agent asks them to allow, with words for it (ADR 0132). */
  answerEscalation: (id: string, escalation: number, allow: boolean, words: string | null) =>
    request<string>('POST', `${thread(id)}/agent/escalation`, { id: escalation, allow, words }),
  /** Who checks the thread's commands on Auto; `null` for its workspace's default (ADR 0132). */
  setJudge: (id: string, judge: Judge | null) => request<string>('POST', `${thread(id)}/agent/judge`, { judge }),
  /** Answers a question the agent waits on, each field by its chosen values; none declines it. */
  answerQuestion: (id: string, question: number, answers: Record<string, string[]> | null) =>
    request<string>('POST', `${thread(id)}/agent/reply`, { id: question, answers }),
  /** Signs the thread's agent in by one of the ways it lists; the sign-in runs on the agent's machine (ADR 0056). */
  signIn: (id: string, method: string) => request<string>('POST', `${thread(id)}/agent/sign-in`, { method }),
  /** Passes a code from the sign-in's page on to the sign-in. */
  signInCode: (id: string, code: string) => request<string>('POST', `${thread(id)}/agent/sign-in/code`, { code }),
  cancelSignIn: (id: string) => request<string>('POST', `${thread(id)}/agent/sign-in/cancel`),
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
  // Phones on the local network (ADR 0112).
  lan: () => request<LanView>('GET', '/api/lan'),
  lanTurn: (on: boolean) => request<LanView>('PUT', '/api/lan', { on }),
  lanPair: (capabilities: Capabilities) => request<LanView>('POST', '/api/lan/pair', { capabilities }),
  lanAnswer: (ask: string, allow: boolean) => request<LanView>('POST', `/api/lan/asks/${encodeURIComponent(ask)}`, { allow }),
  lanPhoneChange: (phone: string, change: { name?: string; capabilities?: Capabilities }) =>
    request<LanView>('PATCH', `/api/lan/phones/${encodeURIComponent(phone)}`, change),
  lanPhoneRevoke: (phone: string) => request<LanView>('POST', `/api/lan/phones/${encodeURIComponent(phone)}/revoke`),
  lanForget: () => request<LanView>('POST', '/api/lan/forget'),
  lanMe: () => request<LanMe>('GET', '/api/lan/me'),
  lanPush: (subscription: PushSubscriptionJSON) => request<string>('POST', '/api/lan/push', subscription),
  lanUnpush: () => request<string>('DELETE', '/api/lan/push'),
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
  /** Agents' asks of other agents on this machine (ADR 0120); the hosted app runs no agents, so none. */
  asks: () => (hosted() ? Promise.resolve([]) : request<AskView[]>('GET', '/api/asks')),
  askStop: (id: string) => request<AskView>('POST', `/api/asks/${encodeURIComponent(id)}/stop`),
  askTake: (id: string) => request<string>('POST', `/api/asks/${encodeURIComponent(id)}/take`),
  /** A merge request; `brief` leaves out its diff and the fork's conversation. */
  mergeRequest: (id: string, hidden: Hidden = {}, brief = false) => {
    const params = hiddenParams(new URLSearchParams(), hidden)
    if (brief) params.set('brief', 'true')
    const query = params.toString()
    return request<MrDetails>('GET', `/api/mrs/${encodeURIComponent(id)}${query ? `?${query}` : ''}`)
  },
  approve: (id: string) => request<MrSummary>('POST', `/api/mrs/${encodeURIComponent(id)}/approve`),
  writeGuide: (id: string, text?: string) =>
    request<string>('POST', `/api/mrs/${encodeURIComponent(id)}/guide`, text === undefined ? {} : { text }),
  markSeen: (id: string, path: string, seen: boolean) =>
    request<string>('PUT', `/api/mrs/${encodeURIComponent(id)}/seen`, { path, seen }),
  requestChanges: (id: string, reason: string) =>
    request<MrSummary>('POST', `/api/mrs/${encodeURIComponent(id)}/request-changes`, { reason }),
  decline: (id: string, reason: string) => request<MrSummary>('POST', `/api/mrs/${encodeURIComponent(id)}/decline`, { reason }),
}

export function short(id: string): string {
  return id.slice(-12)
}

/** Agent CLIs Silt can continue a thread with. */
export const AGENTS = ['claude', 'codex', 'qudiks', 'gemini', 'agy', 'opencode', 'copilot'] as const
