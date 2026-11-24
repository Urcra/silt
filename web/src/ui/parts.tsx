// The pieces the boards' screens are drawn with (ADR 0039): status dots,
// pills, section labels, cards, key/value lists, diff blocks and the status bar.

import { type HTMLAttributes, type ReactNode, useState } from 'react'
import { cx } from './cx'
import { Icon } from './icons'

const DOT = {
  ok: 'bg-ok',
  warn: 'bg-warn',
  danger: 'bg-danger',
  info: 'bg-info',
  accent: 'bg-accent',
  idle: 'bg-fg-3',
}

export type DotTone = keyof typeof DOT

/** A 6 px status dot, where a badge would otherwise go; `label` when it says something alone. */
export function Dot({ tone, label, className }: { tone: DotTone; label?: string; className?: string }) {
  return (
    <span
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      title={label}
      className={cx('inline-block h-1.5 w-1.5 shrink-0 rounded-full', DOT[tone], className)}
    />
  )
}

/** A thread whose agent is working: a turning amber ring, still under reduced motion. */
export function Spinner({ label, className }: { label?: string; className?: string }) {
  return (
    <span
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      title={label}
      className={cx(
        'inline-block h-[9px] w-[9px] shrink-0 animate-spin rounded-full border-[1.5px] border-accent border-r-transparent motion-reduce:animate-none',
        className,
      )}
    />
  )
}

/** A small mono label in a hairline box, such as the device in the top bar. */
export function Pill({ children, className, ...props }: HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      className={cx(
        'inline-flex items-center gap-1.5 rounded-sm border border-line-subtle bg-surface-1 px-2 py-0.5 font-mono text-2xs text-fg-2',
        className,
      )}
      {...props}
    >
      {children}
    </span>
  )
}

/** The tiny upper-case heading over a group, with an optional count or action on its right. */
export function SectionLabel({ children, aside, className }: { children: ReactNode; aside?: ReactNode; className?: string }) {
  return (
    <div
      className={cx(
        'flex items-center justify-between gap-2 px-3 pt-2.5 pb-1 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase',
        className,
      )}
    >
      <span className="min-w-0 truncate">{children}</span>
      {aside}
    </div>
  )
}

/** A bordered box on the raised surface. */
export function Card({ children, className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cx('rounded-md border border-line bg-elevated', className)} {...props}>
      {children}
    </div>
  )
}

/** Pairs of a mono key and its value, as an inspector lists them. */
export function KeyValue({ rows, className }: { rows: [ReactNode, ReactNode][]; className?: string }) {
  return (
    <dl className={cx('grid grid-cols-[90px_1fr] gap-x-3 gap-y-1 text-xs', className)}>
      {rows.map(([key, value], index) => (
        <div key={index} className="contents">
          <dt className="font-mono text-fg-2">{key}</dt>
          <dd className="min-w-0 font-mono break-words text-fg-1">{value}</dd>
        </div>
      ))}
    </dl>
  )
}

/** A file's diff: its path and counts in a header, its lines below. */
export function DiffBlock({
  path,
  added,
  removed,
  actions,
  children,
  className,
  id,
}: {
  path: ReactNode
  added?: number
  removed?: number
  actions?: ReactNode
  children?: ReactNode
  className?: string
  id?: string
}) {
  const [folded, setFolded] = useState(false)
  return (
    <section id={id} className={cx('overflow-hidden rounded-md border border-line bg-elevated', className)}>
      <header
        className={cx(
          'flex items-center gap-2 bg-surface-1 px-3 py-1.5 font-mono text-2xs text-fg-2',
          !folded && 'border-b border-line',
        )}
      >
        {children !== undefined && (
          <button
            type="button"
            aria-expanded={!folded}
            aria-label={folded ? 'show this file' : 'fold this file'}
            onClick={() => setFolded(!folded)}
            className="-ml-1 shrink-0 rounded-sm p-0.5 text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11"
          >
            <Icon name="chevronDown" className={cx('h-3 w-3 transition-transform', folded && '-rotate-90')} />
          </button>
        )}
        <span className="min-w-0 truncate text-fg-1">{path}</span>
        {(added !== undefined || removed !== undefined) && (
          <Counts added={added ?? 0} removed={removed ?? 0} className="ml-auto" />
        )}
        {actions && <span className={cx('flex items-center gap-3', added === undefined && 'ml-auto')}>{actions}</span>}
      </header>
      {!folded && children}
    </section>
  )
}

/** Lines added and removed, as `+57 −9`. */
export function Counts({
  added,
  removed,
  binary = false,
  className,
}: {
  added: number
  removed: number
  /** A binary file, or what kind of one, such as `image` (ADR 0120). */
  binary?: boolean | string
  className?: string
}) {
  if (binary)
    return (
      <span className={cx('font-mono text-2xs whitespace-nowrap text-fg-2', className)}>
        {typeof binary === 'string' ? binary : 'binary'}
      </span>
    )
  return (
    <span className={cx('font-mono text-2xs whitespace-nowrap', className)}>
      <span className="text-ok">+{added}</span> <span className="text-danger">−{removed}</span>
    </span>
  )
}

/** The 24 px bar along the bottom of the app or a pane. */
export function StatusBar({ children, className, label }: { children: ReactNode; className?: string; label: string }) {
  return (
    <footer
      aria-label={label}
      className={cx(
        'flex h-6 shrink-0 items-center gap-3 border-t border-line bg-bg px-2.5 font-mono text-2xs text-fg-2',
        className,
      )}
    >
      {children}
    </footer>
  )
}
