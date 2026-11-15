import { type ReactNode, useEffect, useRef } from 'react'
import { Counts, cx } from '../ui'

/** Whether a file was ticked off: `changed` once it changed again after. */
export type Seen = 'yes' | 'no' | 'changed'

export interface SeenFile {
  path: string
  added?: number
  removed?: number
  binary?: boolean
  seen: Seen
  /** A line under it, such as how much is new since it was seen. */
  note?: ReactNode
}

function SeenBox({ file, onSeen }: { file: SeenFile; onSeen: (path: string, seen: boolean) => void }) {
  const box = useRef<HTMLInputElement>(null)
  // Half-ticked once it changed again: seen before, with something new since.
  useEffect(() => {
    if (box.current) box.current.indeterminate = file.seen === 'changed'
  }, [file.seen])
  return (
    <input
      ref={box}
      type="checkbox"
      aria-label={`seen ${file.path}`}
      title={file.seen === 'changed' ? 'Seen, then changed again' : undefined}
      checked={file.seen === 'yes'}
      onChange={(event) => onSeen(file.path, event.target.checked)}
      className="accent-[var(--accent)] max-md:h-5 max-md:w-5"
    />
  )
}

/** Files to tick off as they're looked at, with how many are: a merge request's, or a thread's in its review. */
export function SeenFiles({
  files,
  picked,
  link,
  onSeen,
  hint,
  className,
}: {
  files: SeenFile[]
  picked?: string
  /** A file's name, as a link to it in the diff. */
  link: (path: string) => ReactNode
  /** Unset once there's nothing left to tick off, as for a decided merge request. */
  onSeen?: (path: string, seen: boolean) => void
  hint?: ReactNode
  className?: string
}) {
  const seen = files.filter((file) => file.seen === 'yes').length
  return (
    <nav aria-label="files" className={cx('flex shrink-0 flex-col gap-0.5', className)}>
      {onSeen && (
        <div className="flex items-center gap-2 px-2 pb-2 text-xs text-fg-2">
          <span className="h-1 flex-1 overflow-hidden rounded-sm bg-surface-2" aria-hidden="true">
            <span className="block h-full bg-accent" style={{ width: `${files.length === 0 ? 0 : (100 * seen) / files.length}%` }} />
          </span>
          <span data-testid="seen-count" className="font-mono text-2xs">
            {seen}/{files.length} seen
          </span>
        </div>
      )}
      {files.map((file) => (
        <div
          key={file.path}
          className={cx(
            'flex flex-wrap items-center gap-x-2 rounded-sm px-2 py-1 font-mono text-2xs hover:bg-surface-2 max-md:min-h-11',
            file.path === picked ? 'bg-surface-2 text-fg-1' : 'text-fg-2',
          )}
        >
          {onSeen && <SeenBox file={file} onSeen={onSeen} />}
          {link(file.path)}
          {file.added !== undefined && <Counts added={file.added} removed={file.removed ?? 0} binary={file.binary} />}
          {file.note && <span className={cx('basis-full font-sans', onSeen && 'pl-5')}>{file.note}</span>}
        </div>
      ))}
      {onSeen && hint && <p className="px-2 pt-2 text-2xs leading-normal text-fg-2">{hint}</p>}
    </nav>
  )
}
