// Primitives that open, move focus or come and go: tabs, menus, popovers,
// dialogs, tooltips and toasts, each operable from the keyboard.

import {
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { createPortal } from 'react-dom'
import { cx } from './cx'
import { Icon, type IconName } from './icons'
import { currentToasts, hearToasts, setToasts } from './toasts'

/** Arrow keys, Home and End move between tabs, as in a toolbar. */
export function Tabs<T extends string>({
  label,
  tabs,
  value,
  onChange,
}: {
  label: string
  tabs: { id: T; label: ReactNode }[]
  value: T
  onChange: (id: T) => void
}) {
  const refs = useRef(new Map<T, HTMLButtonElement>())
  const move = (event: KeyboardEvent, index: number) => {
    const last = tabs.length - 1
    const to = { ArrowRight: index + 1, ArrowLeft: index - 1, Home: 0, End: last }[event.key]
    if (to === undefined) return
    event.preventDefault()
    const target = tabs[(to + tabs.length) % tabs.length]
    onChange(target.id)
    refs.current.get(target.id)?.focus()
  }
  return (
    <div role="tablist" aria-label={label} className="flex gap-0.5">
      {tabs.map((tab, index) => (
        <button
          key={tab.id}
          ref={(button) => {
            if (button) refs.current.set(tab.id, button)
          }}
          type="button"
          role="tab"
          aria-selected={tab.id === value}
          tabIndex={tab.id === value ? 0 : -1}
          onClick={() => onChange(tab.id)}
          onKeyDown={(event) => move(event, index)}
          className={cx(
            'inline-flex items-center gap-1.5 rounded-sm px-2.5 py-1 text-xs max-md:min-h-11',
            // Hovering one only lights its text, so only the selected tab is ever filled.
            tab.id === value ? 'bg-surface-2 font-medium text-fg-1' : 'text-fg-2 hover:text-fg-1',
          )}
        >
          {tab.label}
        </button>
      ))}
    </div>
  )
}

/** Closes on Escape or a click outside `box`, giving focus back to `trigger`. */
function useDismiss(open: boolean, close: () => void, box: React.RefObject<HTMLElement | null>, trigger: React.RefObject<HTMLElement | null>) {
  useEffect(() => {
    if (!open) return
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return
      close()
      trigger.current?.focus()
    }
    const onPointer = (event: PointerEvent) => {
      const target = event.target as Node
      if (!box.current?.contains(target) && !trigger.current?.contains(target)) close()
    }
    document.addEventListener('keydown', onKey)
    document.addEventListener('pointerdown', onPointer)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('pointerdown', onPointer)
    }
  }, [open, close, box, trigger])
}

const PANEL = 'absolute z-20 rounded-md border border-line bg-elevated shadow-overlay'

export interface MenuItem {
  label: string
  onSelect: () => void
  disabled?: boolean
  icon?: IconName
  /** Its shortcut, or a word on what it does, after the label. */
  hint?: string
  /** A line above it, starting a group of its own. */
  divided?: boolean
}

/** A button opening a list of actions; arrow keys move between them. */
export function Menu({
  label,
  trigger,
  items,
  above = false,
  align = 'right',
  triggerClassName,
}: {
  /** What the menu is for, for a screen reader. */
  label: string
  trigger: ReactNode
  items: MenuItem[]
  /** Opens upwards, for a trigger near the bottom of the page, such as a docked composer's. */
  above?: boolean
  /** Which of the trigger's edges the menu lines up with. */
  align?: 'left' | 'right'
  triggerClassName?: string
}) {
  const [open, setOpen] = useState(false)
  const button = useRef<HTMLButtonElement>(null)
  const list = useRef<HTMLUListElement>(null)
  const close = () => setOpen(false)
  useDismiss(open, close, list, button)
  useEffect(() => {
    if (open) list.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
  }, [open])
  const move = (event: KeyboardEvent) => {
    const step = { ArrowDown: 1, ArrowUp: -1 }[event.key]
    if (!step || !list.current) return
    event.preventDefault()
    const buttons = [...list.current.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')]
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement)
    buttons[(at + step + buttons.length) % buttons.length]?.focus()
  }
  return (
    <div className="relative inline-block">
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={label}
        onClick={() => setOpen(!open)}
        className={triggerClassName ?? 'inline-flex items-center rounded-sm p-1 text-fg-2 hover:bg-surface-2 hover:text-fg-1'}
      >
        {trigger}
      </button>
      {open && (
        <ul
          ref={list}
          role="menu"
          aria-label={label}
          onKeyDown={move}
          className={cx(PANEL, above ? 'bottom-full mb-1' : 'mt-1', align === 'left' ? 'left-0' : 'right-0', 'min-w-40 p-1')}
        >
          {items.map((item) => (
            <li key={item.label} role="none" className={cx(item.divided && 'mt-1 border-t border-line pt-1')}>
              <button
                type="button"
                role="menuitem"
                disabled={item.disabled}
                onClick={() => {
                  close()
                  button.current?.focus()
                  item.onSelect()
                }}
                className="flex w-full items-center gap-2.5 rounded-sm px-2.5 py-1 text-left text-sm whitespace-nowrap text-fg-1 hover:bg-surface-2 focus:bg-surface-2 disabled:opacity-40 max-md:min-h-11"
              >
                {item.icon && <Icon name={item.icon} className="h-4 w-4 shrink-0 text-fg-2" />}
                <span className="flex-1">{item.label}</span>
                {item.hint && <span className="ml-4 font-mono text-2xs text-fg-3">{item.hint}</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

/** A button showing a panel next to it, such as a form or a list to pick from; `children` may take a way to close it. */
export function Popover({
  trigger,
  children,
  label,
  triggerLabel,
  triggerClassName,
  panelClassName,
  align = 'left',
  className,
  openOnMount = false,
}: {
  trigger: ReactNode
  children: ReactNode | ((close: () => void) => ReactNode)
  label: string
  triggerLabel?: string
  triggerClassName?: string
  panelClassName?: string
  align?: 'left' | 'right'
  /** The box around the trigger and panel; an inline block unless given. */
  className?: string
  /** Opens as soon as it's shown, such as when a link asked for it. */
  openOnMount?: boolean
}) {
  // Where the panel goes, measured from the trigger as it opens; it floats
  // over the page so no scrolling box around the trigger clips it.
  const [at, setAt] = useState<{ top: number; left?: number; right?: number } | null>(null)
  const button = useRef<HTMLButtonElement>(null)
  const panel = useRef<HTMLDivElement>(null)
  const id = useId()
  const open = at !== null
  const close = () => setAt(null)
  useDismiss(open, close, panel, button)
  useEffect(() => {
    if (!open) return
    window.addEventListener('resize', close)
    return () => window.removeEventListener('resize', close)
  }, [open])
  const place = useCallback(() => {
    if (!button.current) return
    const box = button.current.getBoundingClientRect()
    // A phone's popover spans the screen's width from its left edge, never past its right.
    if (window.innerWidth < 768) setAt({ top: box.bottom + 4, left: 8 })
    else setAt(align === 'right' ? { top: box.bottom + 4, right: window.innerWidth - box.right } : { top: box.bottom + 4, left: box.left })
  }, [align])
  const toggle = () => {
    if (open) close()
    else place()
  }
  const opened = useRef(false)
  useEffect(() => {
    if (!openOnMount || opened.current) return
    opened.current = true
    place()
  }, [openOnMount, place])
  // A panel past the window's edge, as on a phone, moves back inside it; one past its bottom,
  // as from a composer docked there, opens above its trigger instead.
  useLayoutEffect(() => {
    if (!at || !panel.current || !button.current) return
    const box = panel.current.getBoundingClientRect()
    const edge = 8
    const below = box.bottom > window.innerHeight - edge + 1
    const across = box.right > window.innerWidth - edge + 1 || box.left < edge - 1
    if (!below && !across) return
    const top = below ? Math.round(Math.max(edge, button.current.getBoundingClientRect().top - 4 - box.height)) : at.top
    const left = across ? Math.round(Math.max(edge, Math.min(box.left, window.innerWidth - edge - box.width))) : at.left
    if (top !== at.top || left !== at.left) setAt(across ? { top, left } : { ...at, top })
  }, [at])
  // The trigger moves when what's around it settles, as when fonts load or
  // a centred page's content arrives under a picker opened with the page.
  useEffect(() => {
    if (!open || !button.current || typeof ResizeObserver === 'undefined') return
    let live = true
    const again = () => live && place()
    const observer = new ResizeObserver(again)
    for (let around: Element | null = button.current.parentElement; around; around = around.parentElement) observer.observe(around)
    void document.fonts?.ready.then(again)
    return () => {
      live = false
      observer.disconnect()
    }
  }, [open, place])
  return (
    <div className={className ?? 'relative inline-block'}>
      <button
        ref={button}
        type="button"
        aria-expanded={open}
        aria-controls={id}
        aria-label={triggerLabel}
        onClick={toggle}
        className={triggerClassName ?? 'inline-flex items-center gap-1 rounded-sm px-2 py-1 text-xs text-fg-2 hover:bg-surface-2'}
      >
        {trigger}
      </button>
      {at &&
        createPortal(
          <div
            ref={panel}
            id={id}
            role="dialog"
            aria-label={label}
            style={{ top: at.top, left: at.left, right: at.right }}
            className={cx(
              'fixed z-40 max-h-[calc(100dvh-60px)] max-w-[calc(100vw-16px)] overflow-y-auto rounded-md border border-line bg-elevated text-fg-1 shadow-overlay',
              panelClassName ?? 'w-72 p-3',
            )}
          >
            {typeof children === 'function' ? children(close) : children}
          </div>,
          document.body,
        )}
    </div>
  )
}

/** A modal dialog; the browser keeps focus in it and closes it on Escape. */
export function Dialog({
  open,
  onClose,
  title,
  children,
  bare = false,
  wide = false,
}: {
  open: boolean
  onClose: () => void
  title: string
  children: ReactNode
  /** No heading or padding, near the top, as the palette is. */
  bare?: boolean
  wide?: boolean
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const heading = useId()
  useEffect(() => {
    const element = dialog.current
    if (!element) return
    if (open && !element.open) element.showModal?.()
    if (!open && element.open) element.close()
  }, [open])
  return (
    <dialog
      ref={dialog}
      aria-labelledby={heading}
      onClose={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
      className={cx(
        'rounded-lg border border-line bg-elevated p-0 text-fg-1 shadow-overlay backdrop:bg-backdrop',
        bare ? 'mx-auto mt-[14vh] w-[min(620px,calc(100vw-2rem))]' : 'm-auto',
        !bare && (wide ? 'w-[min(44rem,calc(100vw-2rem))]' : 'w-[min(35rem,calc(100vw-2rem))]'),
      )}
    >
      {open && bare && (
        <>
          <h2 id={heading} className="sr-only">
            {title}
          </h2>
          {children}
        </>
      )}
      {open && !bare && (
        <div>
          <div className="flex items-center gap-2 border-b border-line px-5 py-3">
            <h2 id={heading} className="min-w-0 truncate text-[16px] font-medium">
              {title}
            </h2>
            <button
              type="button"
              aria-label="close"
              onClick={onClose}
              className="ml-auto inline-flex items-center justify-center rounded-sm p-1 text-fg-2 hover:bg-surface-2 max-md:min-h-11 max-md:min-w-11"
            >
              <Icon name="x" />
            </button>
          </div>
          <div className="px-5 py-4">{children}</div>
        </div>
      )}
    </dialog>
  )
}

/** A hint shown while its element is hovered or focused; `children` gets the props that link the two. */
export function Tooltip({
  text,
  children,
}: {
  text: string
  children: (describedBy: { 'aria-describedby': string }) => ReactNode
}) {
  const [shown, setShown] = useState(false)
  const id = useId()
  return (
    <span
      className="relative inline-flex"
      onMouseEnter={() => setShown(true)}
      onMouseLeave={() => setShown(false)}
      onFocus={() => setShown(true)}
      onBlur={() => setShown(false)}
      onKeyDown={(event) => event.key === 'Escape' && setShown(false)}
    >
      {children({ 'aria-describedby': id })}
      <span
        id={id}
        role="tooltip"
        className={cx(
          'pointer-events-none absolute bottom-full left-1/2 z-30 mb-1 -translate-x-1/2 rounded-sm bg-fg-1 px-2 py-0.5 text-xs whitespace-nowrap text-bg',
          !shown && 'hidden',
        )}
      >
        {text}
      </span>
    </span>
  )
}

/** Where toasts show; mounted once. */
export function Toaster() {
  const shown = useSyncExternalStore(hearToasts, currentToasts)
  return (
    <div aria-live="polite" className="fixed right-4 bottom-4 z-40 flex w-80 flex-col gap-2">
      {shown.map((item) => (
        <div
          key={item.id}
          role={item.tone === 'danger' ? 'alert' : 'status'}
          className={cx(
            'flex items-start gap-2 rounded-md border border-line bg-elevated px-3 py-2 text-sm shadow-overlay',
            item.tone === 'danger' && 'text-danger',
          )}
        >
          <span className="min-w-0 flex-1">{item.text}</span>
          <button
            type="button"
            aria-label="dismiss"
            onClick={() => setToasts(currentToasts().filter((other) => other.id !== item.id))}
            className="rounded-sm text-fg-2 hover:text-fg-1"
          >
            <Icon name="x" />
          </button>
        </div>
      ))}
    </div>
  )
}
