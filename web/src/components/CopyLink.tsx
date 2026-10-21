import { useQuery } from '@tanstack/react-query'
import { api } from '../api'
import { permalink } from '../permalink'
import { keys } from '../query'
import { hosted } from '../served'
import { toast } from '../ui'

/**
 * Copies a permalink to `path` within the app (ADR 0031): into the server
 * `thread` syncs with, or this machine's for one kept here (ADR 0032).
 */
export function CopyLink({ path, thread, label = 'copy link' }: { path: string; thread?: string; label?: string }) {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads, enabled: !hosted() && thread !== undefined })
  const summary = thread === undefined ? undefined : threads.data?.find((candidate) => candidate.id.endsWith(thread))
  const server = summary ? summary.server : (status.data?.server ?? null)
  const copy = () => {
    const url = permalink(path, server)
    void navigator.clipboard?.writeText(url).catch(() => undefined)
    toast(`Link copied: ${url}`, 'ok')
  }
  return (
    <button type="button" className="text-xs text-accent-fg" onClick={copy}>
      {label}
    </button>
  )
}
