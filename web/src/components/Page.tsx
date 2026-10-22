import type { ReactNode } from 'react'
import { cx } from '../ui'

/**
 * A view inside the shell (ADR 0039): its header with title, a mono line of
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
  inspectorWide = false,
  sheet,
}: {
  title?: ReactNode
  meta?: ReactNode
  actions?: ReactNode
  children: ReactNode
  inspector?: ReactNode
  inspectorLabel?: string
  width?: 'full' | 'wide' | 'narrow'
  /** Which view this is, as `data-page`, for tests and styles to find it. */
  label?: string
  /** The body sets its own padding, such as a list running edge to edge. */
  flush?: boolean
  /** The inspector takes more room, such as for a diff. */
  inspectorWide?: boolean
  /** Below the desktop width, the inspector opens over the page as a sheet, only while `open`. */
  sheet?: { open: boolean; onClose: () => void }
}) {
  const column = width === 'narrow' ? 'max-w-3xl' : width === 'wide' ? 'max-w-6xl' : ''
  return (
    <div className="flex min-h-full flex-1 flex-col lg:flex-row">
      <div className="flex min-w-0 flex-1 flex-col" data-page={label}>
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
          className={cx(
            'flex w-full shrink-0 flex-col border-line lg:sticky lg:top-0 lg:h-[calc(100dvh-64px)] lg:overflow-y-auto lg:border-l',
            inspectorWide ? 'lg:w-[min(42vw,640px)]' : 'lg:w-[360px]',
            sheet
              ? sheet.open
                ? 'fixed inset-x-0 top-10 bottom-0 z-30 overflow-y-auto bg-bg lg:static lg:z-auto'
                : 'hidden lg:flex'
              : 'border-t lg:border-t-0',
          )}
        >
          {sheet?.open && (
            <div className="flex justify-end border-b border-line px-2 lg:hidden">
              <button type="button" onClick={sheet.onClose} className="inline-flex min-h-11 items-center gap-1.5 px-3 text-sm text-fg-2">
                Close
              </button>
            </div>
          )}
          {inspector}
        </aside>
      )}
    </div>
  )
}
