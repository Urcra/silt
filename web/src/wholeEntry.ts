// Entries a page of history carried only in part (`cut`: an image without its data, a long output from its end), read
// whole once they're shown.

import { useQuery } from '@tanstack/react-query'
import { useContext } from 'react'
import { api, type HistoryEntry } from './api'
import { ArtifactActions } from './artifactActions'

/** `entry` whole once `wanted`, when its page left part of it out; until then, and otherwise, as it came. */
export function useWhole<T extends HistoryEntry | null | undefined>(entry: T, wanted: boolean): T {
  const thread = useContext(ArtifactActions)?.thread
  const cut = Boolean(entry?.cut)
  const whole = useQuery({
    // Not under the thread: what an entry was recorded as doesn't change with the thread's events.
    queryKey: ['whole-entry', thread ?? '', entry?.index ?? -1, entry?.kind ?? '', entry?.time_ms ?? 0],
    queryFn: async () => {
      const page = await api.history(thread!, entry!.index, 1, { whole: true })
      const found = page.entries[0]
      return found && found.kind === entry!.kind && found.time_ms === entry!.time_ms ? found : entry!
    },
    enabled: wanted && cut && thread !== undefined,
    staleTime: Infinity,
    gcTime: 60_000,
  })
  return cut && whole.data ? (whole.data as T) : entry
}

/** How many lines the whole of `entry`'s text has, for one its page cut. */
export function linesOf(entry: HistoryEntry): number {
  return entry.cut ? entry.cut.lines : entry.text.trim().split('\n').length
}
