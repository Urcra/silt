import { useQuery } from '@tanstack/react-query'
import { api } from '../api'
import { permalink } from '../permalink'
import { keys } from '../query'
import { hosted } from '../served'
import { toast } from '../ui'

/** Copies a permalink to `path` within the app (ADR 0031). */
export function CopyLink({ path, label = 'copy link' }: { path: string; label?: string }) {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const copy = () => {
    const url = permalink(path, status.data?.server ?? null)
    void navigator.clipboard?.writeText(url).catch(() => undefined)
    toast(`Link copied: ${url}`, 'ok')
  }
  return (
    <button type="button" className="text-xs text-accent-fg" onClick={copy}>
      {label}
    </button>
  )
}
