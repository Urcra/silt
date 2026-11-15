import { useQuery } from '@tanstack/react-query'
import { api } from './api'
import { permalink } from './permalink'
import { keys } from './query'
import { hosted } from './served'
import { toast } from './ui'

/** Copies a permalink to a path within the app, into the server `thread` syncs with, as `CopyLink` does. */
export function usePermalink(thread?: string): (path: string) => void {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads, enabled: !hosted() && thread !== undefined })
  const summary = thread === undefined ? undefined : threads.data?.find((candidate) => candidate.id.endsWith(thread))
  const server = summary ? summary.server : (status.data?.server ?? null)
  return (path) => {
    const url = permalink(path, server)
    void navigator.clipboard?.writeText(url).catch(() => undefined)
    toast(`Link copied: ${url}`, 'ok')
  }
}
