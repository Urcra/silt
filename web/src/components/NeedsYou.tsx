import { useQuery } from '@tanstack/react-query'
import { api } from '../api'
import { useNow } from '../now'
import { keys, message } from '../query'
import { sections } from '../sections'
import { EmptyState, Skeleton } from '../ui'
import { useInWorkspace } from '../workspaceFilter'
import { NeedsYouItem } from './Sidebar'

/** What waits on the person, each answerable in place: a phone's first screen (ADR 0028). */
export function NeedsYou() {
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const shown = useInWorkspace()
  const now = useNow()
  const waiting = sections((threads.data ?? []).filter(shown), now).needs_you
  return (
    <section aria-label="waiting on you" className="mx-auto max-w-2xl space-y-3">
      <h2 className="text-lg font-semibold">Needs you</h2>
      {threads.isPending && <Skeleton className="h-16" />}
      {threads.error && <p className="text-danger">{message(threads.error)}</p>}
      {threads.isSuccess && waiting.length === 0 && <EmptyState title="Nothing waits on you">Agents that ask for something show here.</EmptyState>}
      <ul className="divide-y divide-line rounded-md border border-line text-sm">
        {waiting.map((thread) => (
          <NeedsYouItem key={thread.id} thread={thread} />
        ))}
      </ul>
    </section>
  )
}
