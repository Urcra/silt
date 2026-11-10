import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { type DiffStat, api } from '../api'
import { bytes } from '../diffParse'
import { appHref, filePath } from '../permalink'
import { keys, message } from '../query'
import { Button, Counts, Icon, Skeleton, cx } from '../ui'
import { type Commenting, DiffView } from './DiffView'

/** Which diff: one turn's, or the thread's since a base. */
export interface DiffOf {
  turn?: number
  base?: string
}

/** How many files are listed before More files. */
const LISTED = 200

/** A diff too large to read whole: its files from their stats, each read and drawn only once opened (ADR 0059). */
export function FilesByStats({
  thread,
  stats,
  of,
  commenting,
}: {
  thread: string
  stats: DiffStat[]
  of: DiffOf
  commenting?: Commenting
}) {
  const [listed, setListed] = useState(LISTED)
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set())
  const toggle = (path: string) => {
    const next = new Set(open)
    if (!next.delete(path)) next.add(path)
    setOpen(next)
  }
  const total = stats.reduce((sum, stat) => sum + stat.size, 0)
  return (
    <div data-testid="diff-by-file" className="flex flex-col gap-2">
      <p role="note" className="text-xs text-fg-2">
        {stats.length.toLocaleString()} files, {bytes(total)} of changes: too much to read at once, so each file is read as it's opened.
      </p>
      <ul className="flex flex-col gap-1">
        {stats.slice(0, listed).map((stat) => (
          <li key={stat.path} className="overflow-hidden rounded-md border border-line bg-elevated">
            <div className="flex items-center gap-2 bg-surface-1 px-2 py-1 font-mono text-2xs text-fg-2">
              <button
                type="button"
                aria-expanded={open.has(stat.path)}
                aria-label={`${open.has(stat.path) ? 'fold' : 'open'} ${stat.path}`}
                onClick={() => toggle(stat.path)}
                className="flex min-w-0 flex-1 items-center gap-2 text-left hover:text-fg-1 max-md:min-h-11"
              >
                <Icon name="chevronDown" className={cx('h-3 w-3 shrink-0 transition-transform', !open.has(stat.path) && '-rotate-90')} />
                <span className="min-w-0 truncate text-fg-1" title={stat.path}>
                  {stat.path}
                  {stat.old_path && <span className="ml-2 text-fg-2">renamed from {stat.old_path}</span>}
                </span>
              </button>
              <span className="whitespace-nowrap">{bytes(stat.size)}</span>
              <Counts added={stat.added} removed={stat.removed} binary={stat.binary} />
              <a aria-label="view file" className="font-sans text-accent-fg hover:text-fg-1" href={appHref(filePath(thread, stat.path))}>
                View file
              </a>
            </div>
            {open.has(stat.path) && <OneFile thread={thread} of={of} path={stat.path} commenting={commenting} />}
          </li>
        ))}
      </ul>
      {stats.length > listed && (
        <Button className="self-start" onClick={() => setListed(listed + LISTED)}>
          More files ({(stats.length - listed).toLocaleString()} left)
        </Button>
      )}
    </div>
  )
}

/** One file's lines, unified, as a file opened from a list has less room. */
function OneFile({ thread, of, path, commenting }: { thread: string; of: DiffOf; path: string; commenting?: Commenting }) {
  const diff = useQuery({ queryKey: keys.diffFile(thread, of, path), queryFn: () => api.diff(thread, { ...of, path }) })
  if (diff.error) return <p className="px-3 py-2 text-sm text-danger">{message(diff.error)}</p>
  if (diff.data === undefined) return <Skeleton className="m-2 h-12" />
  return <DiffView text={diff.data} commenting={commenting} bare />
}
