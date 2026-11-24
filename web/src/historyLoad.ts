// A thread's history as the page holds it: its last page first, earlier pages only when the person scrolls up to
// them, and, as the thread changes, only from where it changed (ADR 0117). Reading a long thread from its start drew
// it a page at a time from the top, its tasks starting and ending as each page came, before the page could land at
// its end.

import { useCallback, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, type HistoryEntry, type HistoryPage } from './api'
import { keys } from './query'

/** Entries the history opens with, from the start of the turn the first of them is in. */
export const FIRST_PAGE = 50
/** Entries in each page read after the first: updates, and earlier pages. */
export const PAGE = 500

export interface Loaded {
  thread: string
  total: number
  /** The daemon's version of what's held, to ask what changed since; none from a server, whose pages are read whole. */
  version: number | null
  /** The index of the first entry held: those before it are read only when scrolled up to. */
  start: number
  /** The entries from `start` on, each at index `start` plus its place. */
  entries: HistoryEntry[]
  /** The entries before `start` with news of a task, so tasks started before what's held show as they are. */
  tasks: HistoryEntry[]
}

/** What a page asks for: a range from `offset`, or the last `tail` entries before `before`. */
export type Ask = { offset: number; limit: number; known: number | null } | { tail: number; before?: number }

export type Fetch = (ask: Ask) => Promise<HistoryPage>

function fromPage(page: HistoryPage): Loaded {
  return {
    thread: page.thread,
    total: page.total,
    version: page.version ?? null,
    start: page.from ?? 0,
    entries: page.entries,
    tasks: page.tasks ?? [],
  }
}

/** The history's last page, as a thread opens, or as what's held can't be brought up to date. */
async function readTail(fetch: Fetch, size: number): Promise<Loaded> {
  return fromPage(await fetch({ tail: Math.max(FIRST_PAGE, size) }))
}

/**
 * Brings `before` up to date, or reads the history's last page without it. Unchanged entries keep their objects, so
 * what was drawn from them isn't drawn again, and an unchanged history is `before` itself. `show` hears each page of
 * an update but the last as it comes.
 */
export async function loadHistory(
  fetch: Fetch,
  before: Loaded | undefined,
  show: (loaded: Loaded) => void,
  signal?: AbortSignal,
): Promise<Loaded> {
  if (!before) return readTail(fetch, FIRST_PAGE)
  if (before.version === null) {
    // A server's history says nothing of what changed: what's held is read again whole.
    const page = await fetch({ offset: before.start, limit: Number.MAX_SAFE_INTEGER, known: null })
    signal?.throwIfAborted()
    return fromPage(page)
  }
  const { start } = before
  let entries = before.entries
  let known: number = before.version
  let changed = false
  for (;;) {
    const offset = start + entries.length
    const page = await fetch({ offset, limit: PAGE, known })
    // A read given up, as one a newer read replaced, stops rather than reading on beside it.
    signal?.throwIfAborted()
    const version = page.version ?? null
    const from = Math.min(page.from ?? offset, offset)
    // A history that can't say what changed, or changed before what's held, is read again from its end.
    if (version === null || from < start) return readTail(fetch, entries.length)
    if (from < offset || page.entries.length > 0) {
      entries = entries.slice(0, from - start).concat(page.entries)
      changed = true
    }
    if (start + entries.length > page.total) {
      entries = entries.slice(0, Math.max(0, page.total - start))
      changed = true
    }
    known = version
    const loaded = { ...before, total: page.total, version, entries }
    if (start + entries.length >= page.total || page.entries.length === 0) {
      if (!changed && before.total === page.total && before.version === version) return before
      return loaded
    }
    show(loaded)
  }
}

/** `loaded` with the page before it, read as the person scrolls up to it. */
export async function loadEarlier(fetch: Fetch, loaded: Loaded): Promise<Loaded> {
  if (loaded.start === 0) return loaded
  const page = await fetch({ tail: PAGE, before: loaded.start })
  const earlier = page.entries.filter((entry) => entry.index < loaded.start)
  return { ...loaded, start: page.from ?? 0, entries: earlier.concat(loaded.entries), tasks: page.tasks ?? [] }
}

/**
 * What an update read, with the earlier pages read meanwhile: an update starts from what was held when it began, and
 * would otherwise drop them.
 */
export function withEarlier(updated: Loaded, held: Loaded | undefined): Loaded {
  if (!held || held.start >= updated.start || held.thread !== updated.thread) return updated
  const earlier = held.entries.filter((entry) => entry.index < updated.start)
  if (earlier.length === 0 || earlier[earlier.length - 1].index !== updated.start - 1) return updated
  return { ...updated, start: held.start, entries: earlier.concat(updated.entries), tasks: held.tasks }
}

function fetcher(id: string): Fetch {
  return (ask) =>
    'tail' in ask
      ? api.history(id, undefined, undefined, { tail: ask.tail, before: ask.before })
      : api.history(id, ask.offset, ask.limit, { known: ask.known })
}

/**
 * A thread's history from its last page, kept up to date, and `earlier` to read the page before what's held, once the
 * person scrolls up to it.
 */
export function useHistory(id: string | null) {
  const queries = useQueryClient()
  const queryKey = [...keys.history(id ?? ''), 'loaded']
  const history = useQuery({
    queryKey,
    enabled: id !== null,
    queryFn: async ({ signal }) => {
      const before = queries.getQueryData<Loaded>(queryKey)
      const updated = await loadHistory(fetcher(id!), before, (loaded) => queries.setQueryData(queryKey, loaded), signal)
      // Read again from its end, what changed before what was held: earlier pages held meanwhile are stale too.
      return before && updated.start === before.start ? withEarlier(updated, queries.getQueryData<Loaded>(queryKey)) : updated
    },
    // What's unchanged is kept by `loadHistory` itself; comparing tens of megabytes on each change would cost more.
    structuralSharing: false,
  })
  const [loadingEarlier, setLoadingEarlier] = useState(false)
  const reading = useRef(false)
  const earlier = useCallback(async () => {
    const held = queries.getQueryData<Loaded>(queryKey)
    if (!id || !held || held.start === 0 || reading.current) return
    reading.current = true
    setLoadingEarlier(true)
    try {
      const read = await loadEarlier(fetcher(id), held)
      // What an update brought meanwhile stays; the earlier page goes before it.
      const now = queries.getQueryData<Loaded>(queryKey)
      if (now && now.start === held.start) queries.setQueryData(queryKey, withEarlier(now, read))
    } finally {
      reading.current = false
      setLoadingEarlier(false)
    }
    // The key is the id's; it changes with it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [queries, id])
  return { error: history.error, data: history.data, earlier, loadingEarlier }
}
