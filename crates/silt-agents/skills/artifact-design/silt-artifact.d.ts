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
