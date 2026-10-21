// Primitives that open, move focus or come and go: tabs, menus, popovers,
// dialogs, tooltips and toasts, each operable from the keyboard.

import {
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { cx } from './cx'
import { Icon } from './icons'
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
            'rounded-sm px-2.5 py-1 text-xs',
            tab.id === value ? 'bg-surface-2 text-fg-1' : 'text-fg-2 hover:bg-surface-2 hover:text-fg-1',
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

const PANEL = 'absolute z-20 mt-1 rounded-md border border-line bg-elevated shadow-overlay'

/** A button opening a list of actions; arrow keys move between them. */
export function Menu({
  label,
  trigger,
  items,
}: {
  /** What the menu is for, for a screen reader. */
  label: string
  trigger: ReactNode
  items: { label: string; onSelect: () => void; disabled?: boolean }[]
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
        className="inline-flex items-center rounded-sm p-1 text-fg-2 hover:bg-surface-2 hover:text-fg-1"
      >
        {trigger}
      </button>
      {open && (
        <ul ref={list} role="menu" aria-label={label} onKeyDown={move} className={cx(PANEL, 'right-0 min-w-40 p-1')}>
          {items.map((item) => (
            <li key={item.label} role="none">
              <button
                type="button"
                role="menuitem"
                disabled={item.disabled}
                onClick={() => {
                  close()
                  button.current?.focus()
                  item.onSelect()
                }}
                className="block w-full rounded-sm px-2.5 py-1 text-left text-sm text-fg-1 hover:bg-surface-2 focus:bg-surface-2 disabled:opacity-40"
              >
                {item.label}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

/** A button showing a panel next to it, such as a form or details. */
export function Popover({ trigger, children, label }: { trigger: ReactNode; children: ReactNode; label: string }) {
  const [open, setOpen] = useState(false)
  const button = useRef<HTMLButtonElement>(null)
  const panel = useRef<HTMLDivElement>(null)
  const id = useId()
  useDismiss(open, () => setOpen(false), panel, button)
  return (
    <div className="relative inline-block">
      <button
        ref={button}
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen(!open)}
        className="inline-flex items-center gap-1 rounded-sm px-2 py-1 text-xs text-fg-2 hover:bg-surface-2"
      >
        {trigger}
      </button>
      {open && (
        <div ref={panel} id={id} role="dialog" aria-label={label} className={cx(PANEL, 'w-72 p-3')}>
          {children}
        </div>
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
}: {
  open: boolean
  onClose: () => void
  title: string
  children: ReactNode
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
      className="m-auto w-[min(32rem,calc(100vw-2rem))] rounded-lg border border-line bg-elevated p-0 text-fg-1 shadow-overlay backdrop:bg-black/40"
    >
      {open && (
        <div className="p-4">
          <div className="mb-3 flex items-center">
            <h2 id={heading} className="text-base font-medium">
              {title}
            </h2>
            <button type="button" aria-label="close" onClick={onClose} className="ml-auto rounded-sm p-1 text-fg-2 hover:bg-surface-2">
              <Icon name="x" />
            </button>
          </div>
          {children}
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
