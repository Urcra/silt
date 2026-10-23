import { type ReactNode, useContext } from 'react'
import { InSettings } from '../settingsContext'
import { Drawer } from '../shell'
import { IconButton, cx } from '../ui'

/** On a phone, the header's way to the sidebar, as the page's first control. */
export function DrawerButton() {
  const drawer = useContext(Drawer)
  if (!drawer) return null
  return (
    <IconButton
      icon="list"
      label={drawer.open ? 'hide projects' : 'show projects'}
      aria-expanded={drawer.open}
      onClick={drawer.toggle}
      className="-ml-2 shrink-0 md:hidden"
    />
  )
}

/**
 * A view inside the shell (ADR 0044): its 48 px header with title, a mono line of
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
  inspectorTitle,
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
  /** The inspector's own 48 px row, level with the page's header. */
  inspectorTitle?: ReactNode
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
  const embedded = useContext(InSettings)
  if (embedded)
    return (
      <div className="flex min-w-0 flex-col gap-3" data-page={label}>
        {(title || actions) && (
          <div className="flex flex-wrap items-center gap-2.5">
            {title && <h2 className="text-sm font-medium">{title}</h2>}
            {meta && <span className="min-w-0 font-mono text-2xs text-fg-2">{meta}</span>}
            {actions && <span className="ml-auto flex flex-wrap items-center gap-1.5">{actions}</span>}
          </div>
        )}
        {children}
      </div>
    )
  return (
    <div className="flex min-h-full flex-1 flex-col lg:flex-row">
      <div className="flex min-w-0 flex-1 flex-col" data-page={label}>
        {(title || actions) && (
          <header className="flex min-h-12 shrink-0 items-center gap-x-2.5 gap-y-0.5 border-b border-line-subtle px-4 max-md:flex-wrap max-md:py-1.5 md:h-12 md:pr-3 md:pl-5">
            <DrawerButton />
            {title && <h1 className="max-w-[60%] min-w-0 shrink-0 truncate text-[15px] font-medium">{title}</h1>}
            {meta && (
              <span className="min-w-0 truncate font-mono text-2xs whitespace-nowrap text-fg-2 max-md:order-last max-md:w-full md:flex-1">{meta}</span>
            )}
            {actions && <span className="ml-auto flex shrink-0 items-center gap-1.5">{actions}</span>}
          </header>
        )}
        <div className={cx('w-full flex-1', column, flush ? 'flex flex-col' : 'px-4 py-4 pb-8 md:px-6')}>{children}</div>
      </div>
      {inspector && (
        <aside
          aria-label={inspectorLabel}
          className={cx(
            'flex w-full shrink-0 flex-col border-line lg:sticky lg:top-0 lg:h-dvh lg:overflow-y-auto lg:border-l',
            inspectorWide ? 'lg:w-[min(42vw,640px)]' : 'lg:w-[360px]',
            sheet
              ? sheet.open
                ? 'fixed inset-x-0 top-0 bottom-0 z-30 overflow-y-auto bg-bg lg:static lg:z-auto'
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
          {inspectorTitle && (
            <div className="flex h-12 shrink-0 items-center border-b border-line-subtle px-4 text-[13px] font-medium max-lg:hidden">{inspectorTitle}</div>
          )}
          {inspector}
        </aside>
      )}
    </div>
  )
}
