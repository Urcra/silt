import { usePermalink } from '../usePermalink'

/**
 * Copies a permalink to `path` within the app (ADR 0031): into the server
 * `thread` syncs with, or this machine's for one kept here (ADR 0032).
 */
export function CopyLink({
  path,
  thread,
  label = 'copy link',
  name,
  className,
}: {
  path: string
  thread?: string
  label?: string
  /** What a screen reader calls it, when `label` alone doesn't say what it links. */
  name?: string
  className?: string
}) {
  const link = usePermalink(thread)
  const copy = () => link(path)
  return (
    <button type="button" aria-label={name} className={className ?? 'text-xs text-accent-fg'} onClick={copy}>
      {label}
    </button>
  )
}
