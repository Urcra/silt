import type { ReactNode } from 'react'
import { cx } from '../ui'

/**
 * A view inside the shell (ADR 0035): its header with title, a mono line of
 * what it is and its actions, its body, and an inspector on the right
 * where it has one. `width` keeps forms and reading to a column.
 */
export function Page({
  title,
  meta,
  actions,
  children,
  inspector,
  inspectorLabel = 'details',
  width = 'full',
  label,
  flush = false,
}: {
  title?: ReactNode
  meta?: ReactNode
  actions?: ReactNode
  children: ReactNode
  inspector?: ReactNode
  inspectorLabel?: string
  width?: 'full' | 'wide' | 'narrow'
  /** What a screen reader calls the view, when its title isn't plain text. */
  label?: string
  /** The body sets its own padding, such as a list running edge to edge. */
  flush?: boolean
}) {
  const column = width === 'narrow' ? 'max-w-3xl' : width === 'wide' ? 'max-w-6xl' : ''
  return (
    <div className="flex min-h-full flex-1 flex-col lg:flex-row">
      <div className="flex min-w-0 flex-1 flex-col" aria-label={label}>
        {(title || actions) && (
          <header className="flex flex-wrap items-center gap-x-2.5 gap-y-1 border-b border-line px-4 py-2.5 md:px-6">
            {title && <h1 className="text-lg font-medium">{title}</h1>}
            {meta && <span className="min-w-0 font-mono text-2xs text-fg-2">{meta}</span>}
            {actions && <span className="ml-auto flex flex-wrap items-center gap-1.5">{actions}</span>}
          </header>
        )}
        <div className={cx('w-full flex-1', column, !flush && 'px-4 py-4 pb-8 md:px-6')}>{children}</div>
      </div>
      {inspector && (
        <aside
          aria-label={inspectorLabel}
          className="flex w-full shrink-0 flex-col border-t border-line lg:sticky lg:top-0 lg:h-[calc(100dvh-64px)] lg:w-[380px] lg:overflow-y-auto lg:border-t-0 lg:border-l"
        >
          {inspector}
        </aside>
      )}
    </div>
  )
}
