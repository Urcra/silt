import { type ReactNode, type RefObject, useContext, useRef } from 'react'
import { useDragWidth } from '../dragWidth'
import { WIDE, useMedia } from '../media'
import { MainScroll } from '../scroll'
import { InSettings, SETTINGS_HEADING } from '../settingsContext'
import { Drawer } from '../shell'
import { IconButton, cx } from '../ui'

const INSPECTOR = { min: 320, max: 960, start: 480 }

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

/** A list page's column: 960 px at most, starting where the page's title does. */
export function ListColumn({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cx('flex w-full max-w-[960px] flex-col gap-4 px-4 py-4 md:pr-6 md:pl-5', className)}>{children}</div>
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
  sheet,
  inspectorWidth = 360,
  columnRef,
  full = false,
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
  /** Below the desktop width, the inspector opens over the page as a sheet, only while `open`; `bare`, with a close of its own. */
  sheet?: { open: boolean; onClose: () => void; bare?: boolean }
  /** On a desktop, the inspector takes the body's space too; the body stays mounted, hidden. */
  full?: boolean
  /** The inspector's width until the person drags it. */
  inspectorWidth?: number
  /** The body's own column, which scrolls beside an inspector: for a view that renders only what's in view, above where `MainScroll` says so. */
  columnRef?: RefObject<HTMLDivElement | null>
}) {
  const column = width === 'narrow' ? 'max-w-3xl' : width === 'wide' ? 'max-w-6xl' : ''
  const embedded = useContext(InSettings)
  // A width per kind of page, kept across its tabs; a diff wanted wide on a thread mustn't squeeze a review's.
  const aside = useRef<HTMLElement>(null)
  const panel = useDragWidth(
    `silt-inspector-width:${label ?? 'page'}`,
    { ...INSPECTOR, start: inspectorWidth },
    'left',
    '--inspector',
    aside,
  )
  const desktop = useMedia(WIDE)
  const outer = useContext(MainScroll)
  const fallback = useRef<HTMLDivElement>(null)
  const own = columnRef ?? fallback
  // Beside an inspector the body scrolls in its own column, so each side has its own scroll bar by it.
  const split = Boolean(inspector) && desktop
  if (embedded)
    return (
      <div className="flex min-w-0 flex-col gap-3" data-page={label}>
        {(title || actions) && (
          <div className="flex flex-wrap items-center gap-2.5">
            {title && <h2 className={SETTINGS_HEADING}>{title}</h2>}
            {meta && <span className="min-w-0 font-mono text-2xs text-fg-2">{meta}</span>}
            {actions && <span className="ml-auto flex flex-wrap items-center gap-1.5">{actions}</span>}
          </div>
        )}
        {children}
      </div>
    )
  return (
    // Grows with its content, never under it, so the header stays stuck however long the body is.
    <MainScroll.Provider value={split ? own : outer}>
      <div className={cx('flex shrink-0 grow flex-col lg:flex-row', split ? 'h-full min-h-0 overflow-hidden' : 'min-h-full')}>
        <div
          ref={own}
          className={cx('flex min-w-0 flex-1 flex-col', split && 'relative overflow-y-auto', split && full && 'lg:hidden')}
          data-page={label}
        >
          {(title || actions) && (
            <header className="sticky top-0 z-10 flex min-h-12 shrink-0 items-center bg-bg gap-x-2.5 gap-y-0.5 border-b border-line-subtle px-4 max-md:flex-wrap max-md:py-1.5 md:h-12 md:pr-3 md:pl-5">
              <DrawerButton />
              {title && <h1 className="max-w-[60%] min-w-0 shrink-0 truncate text-[15px] font-medium">{title}</h1>}
              {meta && (
                <span className="min-w-0 truncate font-mono text-2xs whitespace-nowrap text-fg-2 max-md:order-last max-md:w-full md:flex-1">
                  {meta}
                </span>
              )}
              {actions && <span className="ml-auto flex shrink-0 items-center gap-1.5">{actions}</span>}
            </header>
          )}
          <div className={cx('w-full flex-1', column, flush ? 'flex flex-col' : 'px-4 py-4 pb-8 md:px-6')}>{children}</div>
        </div>
        {inspector && !(split && full) && (
          <div
            {...panel.handle}
            aria-label={`resize ${inspectorLabel}`}
            className="relative z-10 -mr-1 hidden w-1 shrink-0 cursor-col-resize hover:bg-accent-tint focus-visible:bg-accent lg:block"
          />
        )}
        {inspector && (
          <aside
            ref={aside}
            aria-label={inspectorLabel}
            style={panel.style}
            className={cx(
              'flex w-full shrink-0 flex-col border-line lg:h-full lg:overflow-y-auto',
              split && full ? 'lg:flex-1 lg:shrink' : 'lg:w-(--inspector) lg:max-w-[50vw] lg:border-l',
              sheet
                ? sheet.open
                  ? 'fixed inset-x-0 top-0 bottom-0 z-30 overflow-y-auto bg-bg lg:sticky lg:z-auto'
                  : 'hidden lg:flex'
                : 'border-t lg:border-t-0',
            )}
          >
            {sheet?.open && !sheet.bare && (
              <div className="flex justify-end border-b border-line px-2 lg:hidden">
                <button
                  type="button"
                  onClick={sheet.onClose}
                  className="inline-flex min-h-11 items-center gap-1.5 px-3 text-sm text-fg-2"
                >
                  Close
                </button>
              </div>
            )}
            {inspectorTitle && (
              <div className="flex h-12 shrink-0 items-center border-b border-line-subtle px-4 text-[13px] font-medium max-lg:hidden">
                {inspectorTitle}
              </div>
            )}
            {inspector}
          </aside>
        )}
      </div>
    </MainScroll.Provider>
  )
}
