// The page side of an artifact's bridge in Silt (ADR 0072), as `window.silt`.
// Declare what a page uses with `capabilities` on `silt_artifact`; each one
// resolves to `null` where this view can't run it (not declared, not
// granted, not served here, or failed), so render without it first.

interface SiltRefusal extends Error {
  /** `not_granted`, `rate_limited`, `conflict`, `too_large`, `refused` or `unavailable`. */
  code: string
}

interface SiltHot {
  /** What the last version's snapshot held, when a new version arrived while the page was open; else null. */
  readonly data: unknown
  /** Registers what to keep when a new version arrives: a function returning JSON-able state, at most 1 MiB. */
  snapshot(take: () => unknown): void
  /** Starts the page with the kept state, or `{}`. */
  ready(start: (data: any) => void): void
}

interface Silt {
  readonly version: '1'
  /** A capability's functions, or null. Resolves within 10 s. */
  use(name: string): Promise<any | null>
  readonly hot: SiltHot
}

interface Window {
  readonly silt: Silt
}

// `db` (declare `{"db": {}}`, or `{"db": {"rules": [...]}}`): documents kept
// in the thread's page data, which hand off, fork and sync with the thread.
// Paths alternate collection and document: `tasks/t1`, `tasks/t1/notes/n1`.
// Rules: `[{path, read, write}]` with levels `read`, `comment`,
// `collaborator`, `owner`; `{self}` in a path is the writer's own id. Without
// rules, anyone who can read the thread reads, and anyone who may comment
// writes. A path starting `_` is Silt's.
interface SiltDoc<T = any> {
  id: string
  path: string
  data: T
}

interface SiltDocRef<T = any> {
  readonly id: string
  readonly path: string
  get(): Promise<SiltDoc<T> | null>
  set(value: T): Promise<unknown>
  /** Merges top-level fields; a null field removes it. */
  update(fields: Partial<T>): Promise<unknown>
  delete(): Promise<unknown>
  /** Calls `fn` now and whenever the document changes; returns a function that stops it. */
  onSnapshot(fn: (doc: SiltDoc<T> | null) => void): () => void
}

interface SiltCollection<T = any> {
  readonly path: string
  doc(id: string): SiltDocRef<T>
  /** A new document with a fresh id. */
  add(value: T): Promise<SiltDocRef<T>>
  /** `==`, `!=`, `<`, `<=`, `>`, `>=`, `in` or `contains`. */
  where(field: string, op: string, value: unknown): SiltCollection<T>
  orderBy(field: string, direction?: 'asc' | 'desc'): SiltCollection<T>
  limit(n: number): SiltCollection<T>
  get(): Promise<SiltDoc<T>[]>
  onSnapshot(fn: (docs: SiltDoc<T>[]) => void): () => void
}

interface SiltDb {
  doc<T = any>(path: string): SiltDocRef<T>
  collection<T = any>(path: string): SiltCollection<T>
  /** At most 100 writes, all or none: `{op: 'set'|'update'|'delete', path, value|fields}`. */
  batch(writes: { op: 'set' | 'update' | 'delete'; path: string; value?: unknown; fields?: unknown }[]): Promise<unknown>
}

// `localStorage` and `sessionStorage` work as usual, kept for this viewer
// and this artifact by the page showing it. They never reach other viewers,
// other machines or the agent.

// `artifact` (declare `{"artifact": {}}`): the page saves itself as the next
// version, for the person viewing it, and every open view reloads to it.
// Nothing a viewer does is kept unless the page publishes it, so keep the
// shared state as data in the document, render from it, and publish a whole
// regenerated document (doctype first) after a viewer acts, never on load.
// A save over a version that's no longer the latest rejects `conflict`:
// don't retry, the view reloads to the winner. Only the thread's owner and
// collaborators save; others get `not_granted`, so render read-only.
interface SiltArtifact {
  publish(html: string): Promise<{ id: string; version: number }>
  /** The version this view shows. */
  version(): Promise<number>
}

// `user` (declare `{"user": {}}`): who views the page. `id()` is the same
// on every machine and the server for one person; store ids, never names,
// and resolve names with `profiles` each time you render.
interface SiltUser {
  id(): Promise<string>
  me(): Promise<{ id: string; name: string }>
  isOwner(): Promise<boolean>
  /** The owner or a collaborator. */
  canEdit(): Promise<boolean>
  /** `data.write` or `artifact.publish`; null where this view isn't told. */
  can(name: string): Promise<boolean | null>
  /** Names by id; `name` is '' for someone unknown here, so use `||`. */
  profiles(ids: string[]): Promise<Record<string, { id: string; name: string }>>
}

// `downloads` (declare `{"downloads": {}}`): hands the viewer a file the page
// made, after they confirm it. At most 16 MiB; runnable types are refused.
interface SiltDownloads {
  save(file: { filename: string; data: string | Blob | ArrayBuffer | Uint8Array; type?: string }): Promise<{ saved: true }>
}

// `comments` (declare `{"comments": {}}`): opens the artifact's own comment
// composer, quoting `text`. Silt shows the comments; never list them.
interface SiltComments {
  openComposer(at: { text?: string }): Promise<null>
}

// `sample` (declare `{"sample": {}}`): asks an agent a question, with no
// tools and no memory: send the instructions, the data and the format each
// time. The person agrees once per artifact; `not_granted` means they said
// no, so hide the feature. At most 4 a minute (`rate_limited`). Call it on a
// click, never in a loop.
interface SiltSample {
  (input: string | { role: 'user' | 'assistant'; content: string }[], opts?: { onText?: (said: { text: string; delta: string }) => void }): Promise<{
    text: string
    truncated: boolean
  }>
  /** The answer parsed as JSON, a ```json fence around it allowed. */
  json(input: string | { role: 'user' | 'assistant'; content: string }[], opts?: { onText?: (said: { text: string; delta: string }) => void }): Promise<any>
  limits(): Promise<{ images: boolean; perMinute: number }>
}

// `assets` (declare `{"assets": {}}`, or `{"assets": {"upload": "comment"}}`
// to let anyone who may comment add one): files people add to the page,
// kept with the thread. Images, video, audio, PDFs, fonts and text; never a
// page or a script. At most 20 MiB each. Keep an asset's `id` in `db` as the
// pointer, and use `url` (relative to the page) as an <img> or <a> source.
interface SiltAssets {
  upload(file: Blob, name?: string): Promise<{ id: string; url: string; sizeBytes: number; contentType: string }>
  list(): Promise<{ assets: { id: string; name: string; media_type: string; size: number; url: string }[]; usage: { bytes: number } }>
  /** Only on a deliberate action; the owner and collaborators only. */
  delete(id: string): Promise<unknown>
}

// `files` (declare `{"files": {}}`, or `{"files": {"paths": ["docs/**"]}}`):
// the thread's files, read from its history as the viewer, never from a
// checkout. Folders the workspace denies are refused. Insert what you read
// with `textContent`, and don't copy it into `db` unless the person asks.
interface SiltFiles {
  list(opts?: { path?: string; recursive?: boolean; cursor?: string }): Promise<{ entries: { path: string; kind: 'file' | 'folder' }[]; cursor: string | null }>
  /** Text files, at most 4 MiB; `at` a version for an older one. */
  read(path: string, opts?: { at?: string }): Promise<{ path: string; text: string; size: number; revision: string }>
  /** Asks the viewer to save it. */
  download(path: string): Promise<{ saved: true }>
}
