// A thread's history as the page holds it: read a page at a time the first time, then, as the thread changes, only
// from where it changed. Every event of a long thread used to read all of it again: tens of megabytes a second while
// an agent worked.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, type HistoryEntry, type HistoryPage } from './api'
import { keys } from './query'

/** Entries in a thread's first page of history, which shows on its own while the rest loads. */
export const FIRST_PAGE = 50
/** Entries in each page after the first. */
export const PAGE = 500

export interface Loaded {
  thread: string
  total: number
  /** The daemon's version of what's held, to ask what changed since; none from a server, whose pages are read whole. */
  version: number | null
  entries: HistoryEntry[]
}

type Fetch = (offset: number, limit: number, known: number | null) => Promise<HistoryPage>

/**
 * Brings `before` up to date, or reads the history from its start without it. Unchanged entries keep their objects, so
 * what was drawn from them isn't drawn again, and an unchanged history is `before` itself. `show` hears each page but
 * the last as it comes.
 */
export async function loadHistory(
  fetch: Fetch,
  before: Loaded | undefined,
  show: (loaded: Loaded) => void,
  signal?: AbortSignal,
): Promise<Loaded> {
  let known = before?.version ?? null
  let entries = known === null || !before ? [] : before.entries
  let changed = known === null
  for (;;) {
    const offset = entries.length
    const page = await fetch(offset, offset === 0 ? FIRST_PAGE : PAGE, known)
    // A read given up, as one a newer read replaced, stops rather than reading on beside it.
    signal?.throwIfAborted()
    const version = page.version ?? null
    if (known !== null && version === null) {
      // A history that can't say what changed is read again whole.
      known = null
      entries = []
      changed = true
      continue
    }
    const from = Math.min(page.from ?? offset, offset)
    if (from < offset || page.entries.length > 0) {
      entries = from === offset ? entries.concat(page.entries) : entries.slice(0, from).concat(page.entries)
      changed = true
    }
    if (entries.length > page.total) {
      entries = entries.slice(0, page.total)
      changed = true
    }
    known = version
    const loaded = { thread: page.thread, total: page.total, version, entries }
    if (entries.length >= page.total || page.entries.length === 0) {
      if (!changed && before && before.total === page.total && before.version === version) return before
      return loaded
    }
    show(loaded)
  }
}

/** A thread's whole history, kept up to date: the first page is drawn as soon as it comes, and the others follow it. */
export function useHistory(id: string | null) {
  const queries = useQueryClient()
  const queryKey = [...keys.history(id ?? ''), 'loaded']
  const history = useQuery({
    queryKey,
    enabled: id !== null,
    queryFn: ({ signal }) =>
      loadHistory(
        (offset, limit, known) => api.history(id!, offset, limit, { known }),
        queries.getQueryData<Loaded>(queryKey),
        (loaded) => queries.setQueryData(queryKey, loaded),
        signal,
      ),
    // What's unchanged is kept by `loadHistory` itself; comparing tens of megabytes on each change would cost more.
    structuralSharing: false,
  })
  return { error: history.error, data: history.data }
}
