// The page side of an artifact's bridge in Silt (ADRs 0072 to 0082), as
// `window.silt`. These types are what Silt's frame serves, checked against
// its code; the `artifact-capabilities` skill says when to use each one.
//
// A page declares what it uses with `capabilities` on `silt_artifact`, such
// as `{"db": {}, "user": {}}`. `true` reads as `{}`. Leaving `capabilities`
// out of a new version keeps the last version's; `{}` clears them; an object
// replaces the whole set. Known names: artifact, assets, comments, db,
// dictation, downloads, files, mcp, room, sample, tts, user. `permissions`
// is built in and never declared. (`edit` is Silt's own, for doc and sheet
// artifacts.) A declaration is at most 16 KiB of JSON.
//
// Every call is one message to the page holding the frame, which checks it
// against the version's declaration and the viewer's grants before it acts.
// A call and its answer are each at most 1 MiB, except an asset upload
// (20 MiB) and a download (16 MiB). Each capability has its own rate,
// noted on it.

/** Why a call was refused. Every rejection from a capability is one of these. */
type SiltRefusalCode =
  /** The page didn't declare it, it isn't served here, the viewer said no, or their level is too low. */
  | 'not_granted'
  /** Over the capability's rate (per minute, noted on each), or too many watches. Back off; never retry in a loop. */
  | 'rate_limited'
  /** Someone saved first (`artifact.publish`). Every open view reloads to their version. */
  | 'conflict'
  /** The call or its answer is over its size. */
  | 'too_large'
  /** The call was malformed, or what it asked for was refused; `message` says why. */
  | 'refused'
  /** This view can't do it now (no speech model, a connector that isn't running, tools in the hosted app). */
  | 'unavailable'
  /** `tts`: the viewer is dictating, so nothing is spoken over them. */
  | 'busy'
  /** `tts`: the viewer stopped the reading. */
  | 'viewer_stopped'

interface SiltRefusal extends Error {
  code: SiltRefusalCode
  /** Words that say why, fit to show the viewer. */
  message: string
}

/** Names `silt.use` knows, with what each resolves to. */
interface SiltCapabilities {
  artifact: SiltArtifact
  assets: SiltAssets
  comments: SiltComments
  db: SiltDb
  dictation: SiltDictation
  downloads: SiltDownloads
  files: SiltFiles
  mcp: SiltMcp
  permissions: SiltPermissions
  room: SiltRoom
  sample: SiltSample
  tts: SiltTts
  user: SiltUser
}

interface Silt {
  /**
   * The runtime contract this version was made for, kept from version to
   * version until one passes `contract: "latest"` to `silt_artifact`. Only
   * `'1'` exists so far.
   */
  readonly version: string
  /**
   * A capability's functions, or `null` where this view can't run it: not
   * declared, not served here (the offline copy, a read-only link, the
   * hosted app for `dictation`), or it failed. The four can't be told
   * apart, so render the page without it and light the feature up once
   * this resolves. It resolves `null` after 10 s when nothing answers, as
   * in `silt_artifact_check`'s headless browser.
   *
   * It never asks the viewer anything: a consent prompt, a rate limit or a
   * refusal comes on the first call that needs it. Calling it again for
   * the same name returns the same promise. The object it resolves to is
   * frozen and Silt's: keep it and call it, never assign to it.
   */
  use<K extends keyof SiltCapabilities>(name: K): Promise<SiltCapabilities[K] | null>
  use(name: string): Promise<unknown | null>
  readonly hot: SiltHot
}

/**
 * State kept when a new version replaces the page while it's open. The
 * holding page asks for a snapshot every 2 s while one is registered, and
 * starts the next version's frame with the last one.
 */
interface SiltHot {
  /** The last version's snapshot when this frame replaced it; else null. */
  readonly data: unknown
  /** Registers what to keep: a function returning JSON-able state, at most 1 MiB. A larger one isn't kept. */
  snapshot(take: () => unknown): void
  /** Calls `start` at once with the kept state, or `{}`. Boot through it. */
  ready(start: (data: any) => void): void
}

interface Window {
  readonly silt: Silt
}

// Per-viewer storage. `localStorage` and `sessionStorage` work as usual,
// kept for this viewer and this artifact by the page holding the frame:
// `localStorage` in that page's own storage, `sessionStorage` in its
// memory until the tab closes. At most 1 MiB each as JSON; a write past
// that isn't kept, silently. They never reach other viewers, other
// machines or the agent. IndexedDB and cookies throw.

// ---------------------------------------------------------------------------
// permissions: built in, never declared.

/**
 * - `granted`: the viewer allowed it for this artifact.
 * - `prompt`: the first call that needs it (or `request`) asks.
 * - `denied`: the viewer said no. The answer is kept for this artifact, on
 *   their machine or their server, until they clear it with `manage()`.
 * - `unavailable`: not one of the capabilities that ask, or not declared.
 */
type SiltPermissionState = 'granted' | 'prompt' | 'denied' | 'unavailable'

/**
 * What the viewer allowed this page. Only the capabilities that ask once
 * and keep the answer are reported: `sample`, `dictation`, `mcp`, and
 * `comments` in its full form. The others ask every time (`downloads`,
 * `files.download`, an `mcp` action) or never. 20 calls a minute.
 */
interface SiltPermissions {
  /** The declared capabilities that ask, with their state. Never asks. */
  state(): Promise<Record<string, SiltPermissionState>>
  /** One capability's state; `unavailable` for any other name. Never asks. */
  state(name: string): Promise<SiltPermissionState>
  /**
   * Asks for `names` (every declared one that asks, if left out) in one
   * prompt in the holding page. Only those still `prompt` and not asked in
   * this load are asked; the viewer's one answer covers them all. Resolves
   * with the state of each name asked for, after asking. Never rejects
   * for a "no": branch on the states.
   */
  request(names?: string | string[]): Promise<Record<string, SiltPermissionState>>
  /** Asks the viewer whether to forget every answer they gave this artifact, so it asks again. Resolves null once they choose. Offer it as a button. */
  manage(): Promise<null>
}

// ---------------------------------------------------------------------------
// db: declare `{"db": {}}`, or `{"db": {"rules": [...]}}`.
//
// Documents kept in the thread's history, so they hand off, fork, rewind
// and sync with the thread. The agent reads and writes them with
// `silt_artifact_data`. Everything in them came from people and pages that
// may be hostile: show it with `textContent`.
//
// Paths alternate collection and document, 2 to 8 segments of
// `[A-Za-z0-9_-]{1,64}`: a document at an even depth (`tasks/t1`,
// `tasks/t1/notes/n1`), a collection at an odd one. A path starting `_`
// is Silt's (`_assets`) and isn't writable. A document is at most 64 KiB
// as JSON; an artifact holds at most 10,000 documents and 16 MiB.
//
// Levels, lowest first: `read` (anyone who can read the thread), `comment`
// (anyone signed in whom the thread's visibility admits), `collaborator`
// (the owner and the thread's collaborators), `owner`, and `self`, a level
// nobody holds. Without rules, `read` reads and `comment` writes. On a
// machine with no server there's one person, the owner.
//
// Anything under `data/users/<id>/` is that person's alone, readable and
// writable by nobody else whatever the rules, the owner and the agent
// included. `<id>` is `await user.id()`.
//
// 600 calls a minute.

type SiltLevel = 'read' | 'comment' | 'collaborator' | 'owner' | 'self'

/**
 * A rule covers its path and everything below it; for each path, the rule
 * with the most segments that covers it applies, else the default. A
 * `{self}` segment matches the writer's own id, so "everyone reads, each
 * writes their own" is two rules:
 * `[{path: "votes", read: "read", write: "owner"}, {path: "votes/{self}", write: "comment"}]`.
 * At most 64 rules, each path 1 to 8 segments.
 */
interface SiltDbRule {
  path: string
  /** Defaults to `read`. */
  read?: SiltLevel
  /** Defaults to `comment`. */
  write?: SiltLevel
}

interface SiltDbConfig {
  rules?: SiltDbRule[]
}

interface SiltDoc<T = any> {
  /** The last segment of its path. */
  id: string
  path: string
  data: T
}

/** What a write resolves to: how many writes were made. */
interface SiltWritten {
  written: number
}

interface SiltDocRef<T = any> {
  readonly id: string
  readonly path: string
  /** The document, or null when there's none or the rules hide it. */
  get(): Promise<SiltDoc<T> | null>
  /** Replaces the whole document. */
  set(value: T): Promise<SiltWritten>
  /** Merges `fields` in, nested objects too; a `null` removes a field. A missing document starts as `{}`. */
  update(fields: Partial<T>): Promise<SiltWritten>
  delete(): Promise<SiltWritten>
  /**
   * Replaces `old` with `now` in the text field `field`, sending only the
   * change. `old` must be non-empty and occur exactly once in the field;
   * otherwise it rejects `invalid` and nothing is written. Under the same
   * write rules as `update`.
   */
  strReplace(field: keyof T & string, old: string, now: string): Promise<SiltWritten>
  /**
   * Calls `fn` with the document now and again whenever it changes (null
   * while there's none). Returns a function that stops it. A refused watch
   * calls `onError` instead, with `rate_limited` past 32 watches a page.
   */
  onSnapshot(fn: (doc: SiltDoc<T> | null) => void, onError?: (refusal: SiltRefusal) => void): () => void
}

/** Ops for `where`: equality, order, `in` (the field is one of a list), `contains` (an array field holds the value). */
type SiltWhereOp = '==' | '!=' | '<' | '<=' | '>' | '>=' | 'in' | 'contains'

/**
 * A query of the documents directly in a collection, not deeper. Values
 * order as: missing, null, booleans, numbers, strings, then the rest. Each
 * call returns a new query; `orderBy` takes one field, the last given.
 */
interface SiltCollection<T = any> {
  readonly path: string
  doc(id: string): SiltDocRef<T>
  /** A new document with a fresh id made from the time and a random part. */
  add(value: T): Promise<SiltDocRef<T>>
  where(field: string, op: SiltWhereOp, value: unknown): SiltCollection<T>
  orderBy(field: string, direction?: 'asc' | 'desc'): SiltCollection<T>
  /** At most 1,000, the default. Keep it low: an answer over 1 MiB is refused `too_large`. */
  limit(n: number): SiltCollection<T>
  /** The documents the viewer may read, in order. */
  get(): Promise<SiltDoc<T>[]>
  /** As a document's `onSnapshot`, for the query's documents. */
  onSnapshot(fn: (docs: SiltDoc<T>[]) => void, onError?: (refusal: SiltRefusal) => void): () => void
}

type SiltDbWrite =
  | { op: 'set'; path: string; value: unknown }
  | { op: 'update'; path: string; fields: object }
  | { op: 'delete'; path: string }

interface SiltDb {
  doc<T = any>(path: string): SiltDocRef<T>
  collection<T = any>(path: string): SiltCollection<T>
  /**
   * At most 100 writes, made in order, all or none: one that the rules or
   * the limits refuse refuses the batch. A refused write rejects `refused`
   * with a message such as "you may not write votes/bob".
   */
  batch(writes: SiltDbWrite[]): Promise<SiltWritten>
}

// ---------------------------------------------------------------------------
// artifact: declare `{"artifact": {}}`. There's no `self` alias.

/**
 * A page that saves itself as the next version. Nothing a viewer does is
 * kept unless the page publishes it, so embed the shared state as data in
 * the document, render from it, and publish a whole regenerated document
 * after a viewer acts, never on load. 12 calls a minute.
 */
interface SiltArtifact {
  /**
   * Saves `html`, a whole document with its doctype first, as a new version
   * over the one this view shows, and every open view reloads to it.
   * Rejects `conflict` when that's no longer the latest (don't retry; this
   * view reloads to the winner), `not_granted` for anyone but the thread's
   * owner and collaborators, `refused` without a doctype, and `too_large`
   * past the bridge's 1 MiB.
   */
  publish(html: string): Promise<{ id: string; version: number }>
  /** The version this view shows. */
  version(): Promise<number>
}

// ---------------------------------------------------------------------------
// user: declare `{"user": {}}`, or `{"user": {"scopes": ["profile"]}}` for names and `search`.

/**
 * Who views the page. An id is 16 hex characters, the same for one person
 * on every machine and the server, and shows no name; with no server it's
 * `local`. Store ids, never names. Names are known for the thread's owner,
 * its collaborators and the viewer, and only under the `profile` scope:
 * without it every name is ''. Silt keeps no email addresses. Resolve
 * names each time you render.
 * Reads never reject for a missing name. 120 calls a minute.
 */
interface SiltUser {
  id(): Promise<string>
  /** `name` is '' when unknown here. */
  me(): Promise<{ id: string; name: string }>
  isOwner(): Promise<boolean>
  /** The owner or a collaborator. */
  canEdit(): Promise<boolean>
  /** `data.write` (any level above `read`) or `artifact.publish` (as `canEdit`); null for any other name. */
  can(name: string): Promise<boolean | null>
  /** Names by id, at most 500 ids; `name` is '' for someone unknown, so fall back with `||`. */
  profiles(ids: string[]): Promise<Record<string, { id: string; name: string }>>
  /** Up to 20 of the thread's people with a word of their name starting with `query`; everyone (up to 20) when it's empty. */
  search(query: string): Promise<{ id: string; name: string }[]>
}

// ---------------------------------------------------------------------------
// downloads: declare `{"downloads": {}}`.

/**
 * A file the page made, saved once the viewer confirms it in the holding
 * page, every time. 10 calls a minute.
 */
interface SiltDownloads {
  /**
   * At most 16 MiB. The name loses folder separators, control characters
   * and leading dots, and is cut to 128 characters; runnable types (`.exe`,
   * `.sh`, `.js`, `.bat`, `.ps1`, `.jar`, `.dmg` and the like) are refused.
   * Rejects `not_granted` when the viewer declines, `refused` for a bad name
   * or data, `too_large` past 16 MiB.
   */
  save(file: { filename: string; data: string | Blob | ArrayBuffer | Uint8Array; type?: string }): Promise<{ saved: true }>
}

// ---------------------------------------------------------------------------
// comments: declare `{"comments": {"composer_only": true}}` or `{"comments": {}}`.

/**
 * An entry point to Silt's own comments, which Silt shows and sends to the
 * agent like a person's. Never build a list of them. `composer_only`
 * serves `openComposer` alone and asks nothing. The full form adds `add`,
 * `reply` and `resolve`, written as the viewer once they agree for this
 * artifact (kept; `not_granted` if they said no). 20 calls a minute.
 */
interface SiltComments {
  /**
   * Opens the composer, quoting `text` (500 characters at most). With
   * `element` (or an `anchor` of the page's own), the comment is on that
   * element, quoting its text when no `text` is given; without, on this version.
   */
  openComposer(at: { text?: string; element?: Element; anchor?: string }): Promise<null>
  /**
   * An element's anchor: the `data-comment-anchor` of it or an ancestor,
   * else `#id`, else a CSS path. Give elements people comment on a
   * `data-comment-anchor` (or an id) so comments keep to them across versions.
   */
  anchorFor(element: Element): string
  /**
   * For a page that draws what it's commented on itself (a canvas, a chart):
   * `onReveal` hears the anchor of a comment the viewer asks to see, in place
   * of Silt scrolling to the element and outlining it. Returns what stops it.
   */
  customAnchors(onReveal: (comment: { anchor: string }) => void): () => void
  /** A comment as the viewer, on the version shown or on `element`/`anchor`: `text` 1 to 4,000 characters, `quote` at most 500. */
  add(comment: { text: string; quote?: string; element?: Element; anchor?: string }): Promise<{ id: string }>
  /** `id` must be a comment on this artifact, else `refused`. */
  reply(to: { id: string; text: string }): Promise<{ id: string }>
  resolve(comment: { id: string }): Promise<{ id: string }>
}

// ---------------------------------------------------------------------------
// sample: declare `{"sample": {}}`, or `{"sample": {"images": true}}`.

/**
 * One question to an agent, Claude Code's print mode on the viewer's own
 * machine, paid by the viewer. It has no tools but the page's, no session
 * and no memory: send the instructions, the data and the answer's format
 * each time. The first question asks the viewer, once per artifact (kept).
 * At most 4 questions a minute per artifact (`rate_limited`).
 *
 * In the hosted app the question goes to one of the person's own online
 * computers, whose Devices settings must allow "Ask an agent a page's
 * question" (phones don't by default); with none, it rejects. There, tools are refused
 * (`unavailable`) and the answer arrives a few hundred milliseconds at a
 * time.
 *
 * There's no abort, no model or effort choice and no cache.
 */
interface SiltSampleTool {
  /** 1 to 64 letters, digits, `_` and `-`, unique in the question. */
  name: string
  /** At most 1,000 characters. */
  description?: string
  /** A JSON Schema object for `execute`'s input; any object when left out. */
  inputSchema?: object
  /**
   * Runs in the page when the agent calls it, with the agent's input.
   * Return small JSON-able data, or throw; the error's message (2,000
   * characters at most) is what the agent hears. The agent waits about a
   * minute for each call.
   */
  execute(input: any): unknown | Promise<unknown>
}

interface SiltSampleOptions {
  /** As the answer streams: `text` is the whole answer so far (assign it), `delta` what's new. */
  onText?: (said: { text: string; delta: string }) => void
  /** Up to 16 of the page's functions the agent may call while it answers. In the hosted app one of the person's computers answers and its calls come back through the server. */
  tools?: SiltSampleTool[]
  /** `quick` for a short, cheap answer, `complex` for a hard one. */
  modelTier?: 'quick' | 'default' | 'complex'
  /** Only when the person names one, such as `claude-sonnet-5-5`; wins over `modelTier`. */
  model?: string
  /** Up to `limits().images` PNG, JPEG, GIF or WebP images, each at most 5 MiB as base64 (about 3.7 MB). Needs `{"images": true}`, else `refused`. */
  images?: Blob[]
  /** The same question, with no tools or images, asked again within five minutes gets the answer it had, unless this is `false`, as a chat's turns should be. */
  cache?: boolean
  /** Aborting it rejects the call with `aborted` (an `AbortError`) and stops `onText`. Use a new `AbortController` per call. */
  signal?: AbortSignal
}

/** Text, or a conversation that ends on the user's turn. At most 256 KiB, and not empty. */
type SiltSampleInput = string | { role: 'user' | 'assistant'; content: string }[]

interface SiltSample {
  /** `truncated` when the answer was cut off. Rejects `not_granted` (the viewer said no: hide the feature), `rate_limited`, `refused`, `unavailable`, `aborted`. */
  (input: SiltSampleInput, opts?: SiltSampleOptions): Promise<{ text: string; truncated: boolean; modelApplied?: string }>
  /** The answer parsed as JSON, a ```json fence around it allowed. Throws a SyntaxError (no `code`) when it isn't JSON. */
  json(input: SiltSampleInput, opts?: SiltSampleOptions): Promise<any>
  /** `images`: how many one question may show (4), or false without `{"images": true}`. `tools` is true everywhere. One computer answers at most three pages' questions at once. */
  limits(): Promise<{ images: number | false; perMinute: number; tools: true }>
}

// ---------------------------------------------------------------------------
// assets: declare `{"assets": {}}`, or `{"assets": {"upload": "comment"}}`.

/**
 * Files people add to the page, kept with the thread. By default only the
 * owner and collaborators add them; `upload: "comment"` lets anyone who may
 * comment add one. Only the owner and collaborators delete. Everyone with
 * the page lists them all. 30 calls a minute.
 *
 * Types: PNG, JPEG, GIF, WebP, AVIF, SVG, MP4, WebM, MP3, WAV, Ogg, PDF,
 * WOFF, WOFF2, CSV, Markdown, plain text and JSON; never a page or a script.
 * An SVG is kept without scripts, handlers or outside links; show it with
 * `<img>` all the same, never inline.
 */
interface SiltAsset {
  /** 12 lowercase hex characters. */
  id: string
  name: string
  media_type: string
  size: number
  /** `_assets/<id>`, relative to the page: use it as an `<img>`, `<video>`, `<audio>` or `<a>` source. */
  url: string
}

interface SiltAssets {
  /** At most 20 MiB. `name` defaults to a File's own name. Rejects `not_granted` below the declared level, `refused` for a type that isn't allowed, `too_large`. */
  upload(file: Blob, name?: string): Promise<{ id: string; url: string; sizeBytes: number; contentType: string }>
  list(): Promise<{ assets: SiltAsset[]; usage: { bytes: number } }>
  /** For good. Only on a deliberate action, and update the documents that pointed at it. */
  delete(id: string): Promise<{ deleted: string }>
}

// ---------------------------------------------------------------------------
// files: declare `{"files": {}}`, or `{"files": {"paths": ["docs/**", "bench/*.json"]}}`.

/**
 * The thread's files, read from its history as the viewer, never from a
 * checkout, alike on every machine. `paths` globs (`*` within a segment,
 * `**` across them) name what the page may read; without them, the whole
 * tree. Folders the thread's workspace denies are refused. In the hosted
 * app, reads go through one of the person's computers that holds the
 * thread, and fail when none is online. 120 calls a minute.
 *
 * Insert what you read with `textContent`, and don't copy it into `db`,
 * assets or a version unless the person asks: anyone who can read the
 * thread can then read it.
 */
interface SiltFiles {
  /** The files directly under `path` (the whole tree when empty) and its folders, or every file below it when `recursive`; 500 a page, then pass `cursor`. */
  list(opts?: { path?: string; recursive?: boolean; cursor?: string }): Promise<{ entries: { path: string; kind: 'file' | 'folder' }[]; cursor: string | null }>
  /**
   * A text file at the thread's head, or `at` an earlier revision. Rejects
   * `refused` for a path the page may not read or a binary file, and
   * `too_large` for one over 4 MiB.
   */
  read(path: string, opts?: { at?: string }): Promise<{ path: string; text: string; size: number; revision: string }>
  /**
   * The turns that changed `path`, oldest first: the prompt each answered
   * (null for edits outside a turn), whether the agent made it, and the
   * characters it put in and took out. Refused like `read` for a path the
   * page may not read.
   */
  history(path: string): Promise<{ path: string; changes: { prompt: string | null; by_agent: boolean; inserted: number; deleted: number; time_ms: number }[] }>
  /** Saves the file as text once the viewer confirms it, every time; runnable types are refused. */
  download(path: string): Promise<{ saved: true }>
  /**
   * Reads `path` now and again every `every` ms (at least 2,000; 5,000 by
   * default), calling `fn` each time its revision changes. A failed read is
   * skipped silently and tried again at the next tick. Returns a function
   * that stops it; stop it when the page no longer shows the file.
   */
  watch(path: string, fn: (file: { path: string; text: string; size: number; revision: string }) => void, opts?: { every?: number }): () => void
  /**
   * Two tools for `sample`: `list_files {path?, recursive?}` and
   * `read_file {path}` (its text cut at 60,000 characters), so the agent
   * answering a question reads the thread's files itself. Pass them as
   * `tools`; the page must declare `sample` too, and tools only run on the
   * daemon, not in the hosted app.
   */
  asSampleTools(): Promise<SiltSampleTool[]>
}

// ---------------------------------------------------------------------------
// dictation: declare `{"dictation": {}}`.

/**
 * The viewer's speech as text, turned into words on their own machine by
 * Silt's speech helper. `null` in the hosted app and wherever the browser
 * has no microphone access. The first `start()` asks the viewer, once per
 * artifact (kept). A take stops when the holding page's tab is hidden.
 * 30 calls a minute.
 */
interface SiltDictation {
  /**
   * Starts a take. Call it from the viewer's click. Rejects `not_granted`
   * when the viewer said no (keep typing, and don't ask again), `refused`
   * while a take runs, `unavailable` when the speech model isn't
   * downloaded yet or the microphone fails.
   */
  start(): Promise<null>
  /** Ends the take, waiting up to 10 s for the last words. */
  stop(): Promise<null>
  /** A live guess at the words being said, replacing the last guess. */
  onPartial(fn: (said: { text: string }) => void): () => void
  /** A settled phrase, to append. */
  onFinal(fn: (said: { text: string }) => void): () => void
  /** `listening` once a take starts; `idle` when it ends, with `stopped` or `failed`. */
  onState(fn: (said: { state: 'listening' | 'idle'; reason?: 'stopped' | 'failed' }) => void): () => void
}

// ---------------------------------------------------------------------------
// tts: declare `{"tts": {}}`.

/**
 * Text read aloud in the browser's own voices, so they differ between
 * machines. `null` where the browser has no speech synthesis. No microphone
 * and no consent. 60 calls a minute.
 */
interface SiltTtsLine {
  /** 1 to 2,000 characters of plain text. */
  text: string
  /** `buttery` (the default), `airy`, `glassy` or `rounded`, each mapped to one of the browser's voices. */
  voice?: string
  /** 0.7 to 1.5; 1 is normal. */
  rate?: number
  /** Such as `en` or `pt-BR`: picks a voice for it. Give it for text you wrote. */
  language?: string
  /** Passed back by `onLine`. */
  id?: string
}

interface SiltTts {
  /**
   * Reads `text` (with `opts`), or each line in turn, replacing any reading
   * under way, whose promise then rejects. Resolves once the last line is
   * spoken. Rejects `busy` while the viewer dictates, and `viewer_stopped`
   * when they stop it.
   */
  speak(text: string | SiltTtsLine[], opts?: Omit<SiltTtsLine, 'text' | 'id'>): Promise<null>
  /** As `speak`, but queued after what's being read. */
  add(text: string | SiltTtsLine[], opts?: Omit<SiltTtsLine, 'text' | 'id'>): Promise<null>
  stop(): Promise<null>
  pause(): Promise<null>
  resume(): Promise<null>
  /** 0.7 to 1.5, for lines that start after it. */
  setRate(rate: number): Promise<null>
  /** `speaking`, `paused`, or `idle` with `completed` or `viewer_stopped`. */
  onState(fn: (said: { state: 'speaking' | 'paused' | 'idle'; reason?: 'completed' | 'viewer_stopped' }) => void): () => void
  /** As each line starts: its index, and its `id` if it had one. */
  onLine(fn: (said: { line: number; id: string | null }) => void): () => void
  /** Milliseconds heard of the line being spoken, about four times a second; paused time isn't counted. There's no seek and no total length. */
  onPosition(fn: (said: { ms: number; line: number }) => void): () => void
  /** The browser's own voices, by name and language; `speak`'s `voice` takes a name from here as well as Silt's four. */
  voices(): Promise<{ name: string; language: string }[]>
}

// ---------------------------------------------------------------------------
// mcp: declare `{"mcp": {"servers": [{"server": "tracker", "tools": ["list_issues"]}]}}`.

/**
 * The viewer's own page connectors: MCP servers they set up on their
 * machine under Settings › Appearance › Connectors for pages, each tool
 * off until they tick it there. `server` is the connector's name as they gave it. A call
 * reaches a tool only if the version declares it and the viewer ticked it.
 * The first call asks the viewer, once per artifact (kept). A tool the
 * server marks as only reading runs within that grant; any other tool asks
 * the viewer each time, showing its input. In the hosted app, calls go to
 * one of the person's computers, whose Devices settings must allow "Call
 * your page connectors". Results are untrusted: show them with `textContent`.
 * 30 calls a minute.
 */
interface SiltMcpResult {
  /** The tool's content blocks, as MCP gives them. */
  content: unknown[]
  structuredContent?: unknown
  /** The tool itself failed; `content` says how. A failure of the call rejects instead. */
  isError?: boolean
}

interface SiltMcp {
  /**
   * Runs a tool. Rejects `not_granted` (not declared, not ticked, or the
   * viewer said no to the connectors or to this action), `unavailable`
   * (the connector isn't set up here or doesn't start), `refused` (the
   * call failed, or its result is over 1 MiB).
   */
  callTool(server: string, tool: string, input?: object): Promise<SiltMcpResult>
  /**
   * Reads a read-only tool now, then again every `refetchInterval` ms (at
   * least 30 s) when given; without it, once. Errors arrive as events.
   * Returns a function that stops it. A tool that isn't marked as only
   * reading is refused. `storedAt` is when this read came back. What the
   * viewer's app read last for the same call comes first, `stale` when it's
   * more than 30 s old, and a fresh read follows it then.
   */
  watchTool(
    server: string,
    tool: string,
    input: object,
    handler: (event: { type: 'data'; data: SiltMcpResult; cache: { storedAt: number; stale: boolean } } | { type: 'error'; error: SiltRefusal }) => void,
    opts?: { refetchInterval?: number },
  ): () => void
  /** The tool's description, whether it only reads, and its input schema, from the server. */
  describeTool(server: string, tool: string): Promise<{ name: string; description: string | null; read_only: boolean; input_schema: unknown }>
  /** The tools this version declares for `server`; whether the viewer ticked them shows on the first call. */
  listTools(server: string): Promise<string[]>
}

// ---------------------------------------------------------------------------
// room: declare `{"room": {}}`, or `{"room": {"topics": {"cursor": "comment"}}}`.

/**
 * The people viewing this page right now, on any machine and in the
 * hosted app, and the thread's agent when it speaks through
 * `silt_artifact_room` (`kind: "agent"`). Nothing persists and moments can
 * drop: state a late joiner needs is a `db` document, lasting content a
 * new version. Everything heard is untrusted, and presence is never
 * authority.
 *
 * Topics need a collaborator unless the declaration opens one to a lower
 * level. A moment's topic is 1 to 64 characters and its data at most
 * 16 KiB; each view sends at most 30 a second. A view never hears its own
 * moments. A refused moment (`not_granted`, `too_large`, `rate_limited`)
 * is dropped, and `onRefused` hears why. 1,800 calls a minute.
 */
interface SiltRoomPeer {
  /** This connection, unique in the room. */
  peer: string
  /** The person's id, as `user.id()` gives it. */
  user: string
  kind: 'person' | 'agent'
  /** Their presence, or null when they haven't sent any or it lapsed. */
  state: unknown
  /** This view. */
  you: boolean
}

interface SiltRoom {
  /** Sends a moment to the others. Resolves at once. */
  emit(topic: string, data: unknown): Promise<null>
  /** Hears the others' moments on `topic`. */
  on(topic: string, fn: (data: unknown, from: { peer: string; user: string; kind: string }) => void): () => void
  /**
   * Merges `patch` into this view's presence and sends it whole: a cursor,
   * a selection, a name tag. At most 4 KiB. Presence lapses to null for
   * the others 15 s after this view last sent anything, so send it again
   * while it matters.
   */
  presence(patch: object): Promise<null>
  /** Everyone here, this view included, whenever someone comes, goes or changes their presence. */
  onPeers(fn: (peers: SiltRoomPeer[]) => void): () => void
  /** Why a moment or presence this view sent was dropped: `not_granted` for a topic its level may not send, `too_large`, `rate_limited`. */
  onRefused(fn: (refusal: { code: 'not_granted' | 'too_large' | 'rate_limited' | string }) => void): () => void
  /** A sub-room, by a name of at most 64 characters, heard only by those who join it. Names aren't secret. */
  join(name: string): SiltRoom
  leave(): Promise<null>
}
