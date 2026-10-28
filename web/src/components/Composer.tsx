import { type ClipboardEvent, type DragEvent, type KeyboardEvent, type ReactNode, useLayoutEffect, useRef, useState } from 'react'
import { filesFrom } from '../attachments'
import { Icon, type IconName, cx } from '../ui'
import { ComposerFolded } from './composerFolded'

/**
 * Folds the row whose tools `content` holds when they're wider than `room`, and unfolds it once they'd fit
 * again. Folded, a change in what the row holds, such as another agent's settings, measures it unfolded anew.
 */
function useFolded() {
  const room = useRef<HTMLSpanElement>(null)
  const content = useRef<HTMLSpanElement>(null)
  const [folded, setFolded] = useState(false)
  const unfolded = useRef(0)
  const foldedWidth = useRef<number | undefined>(undefined)
  useLayoutEffect(() => {
    const [outer, inner] = [room.current, content.current]
    if (!outer || !inner || typeof ResizeObserver === 'undefined') return undefined
    const fit = () => {
      const width = inner.offsetWidth
      if (!folded) {
        unfolded.current = width
        if (width > outer.clientWidth) setFolded(true)
      } else if (foldedWidth.current === undefined) {
        foldedWidth.current = width
      } else if (outer.clientWidth >= unfolded.current || width !== foldedWidth.current) {
        foldedWidth.current = undefined
        setFolded(false)
      }
    }
    fit()
    const observer = new ResizeObserver(fit)
    observer.observe(outer)
    observer.observe(inner)
    return () => observer.disconnect()
  }, [folded])
  return { room, content, folded }
}

/**
 * The one composer (ADR 0044): centred on an empty thread, docked under a
 * running one. What's above the text, the text, then a row of its tools and
 * the button that sends it.
 */
export function ComposerBox({
  id,
  label,
  value,
  onChange,
  onSend,
  placeholder,
  rows = 3,
  above,
  below,
  tools,
  actions,
  waiting = false,
  onCycle,
  onFiles,
  className,
}: {
  id: string
  /** What the text is, for a screen reader and the tests. */
  label: string
  value: string
  onChange: (value: string) => void
  /** ⏎ sends; mod+⏎ sends now, interrupting the agent's turn where it has one. */
  onSend: (now: boolean) => void
  placeholder: string
  rows?: number
  /** Over the text: what's queued, what others are writing. */
  above?: ReactNode
  /** Between the text and the tools: attached files. */
  below?: ReactNode
  tools?: ReactNode
  actions?: ReactNode
  /** The agent waits on the person, so the box says so. */
  waiting?: boolean
  /** Shift+Tab: the agent's next mode, as in its own terminal. */
  onCycle?: () => void
  /** Files pasted or dropped into the box; without it, the box takes none. */
  onFiles?: (files: File[], pasted: boolean) => void
  className?: string
}) {
  const { room, content, folded } = useFolded()
  const [dragging, setDragging] = useState(false)
  const paste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = filesFrom(event.clipboardData)
    if (!onFiles || files.length === 0) return
    // An image copied from a web page brings its address as text too, which still pastes.
    if (!event.clipboardData.types.includes('text/plain')) event.preventDefault()
    onFiles(files, true)
  }
  const dragOver = (event: DragEvent<HTMLDivElement>) => {
    if (!onFiles || !event.dataTransfer.types.includes('Files')) return
    event.preventDefault()
    setDragging(true)
  }
  const drop = (event: DragEvent<HTMLDivElement>) => {
    setDragging(false)
    const files = filesFrom(event.dataTransfer)
    if (!onFiles || files.length === 0) return
    event.preventDefault()
    onFiles(files, false)
  }
  const keyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // ⏎ sends and shift+⏎ breaks the line, as in a chat; on a touch keyboard, with no shift, ⏎ breaks it and
    // the button sends. Never while an input method is still composing a character.
    const touch = window.matchMedia?.('(pointer: coarse)').matches ?? false
    if (event.key === 'Enter' && !event.nativeEvent.isComposing && !event.shiftKey && !event.altKey && (!touch || event.metaKey || event.ctrlKey)) {
      event.preventDefault()
      onSend(event.metaKey || event.ctrlKey)
    }
    if (event.key === 'Tab' && event.shiftKey && onCycle) {
      event.preventDefault()
      onCycle()
    }
  }
  return (
    <div
      onDragOver={dragOver}
      onDragLeave={() => setDragging(false)}
      onDrop={drop}
      className={cx('flex flex-col rounded-lg border bg-elevated', dragging ? 'border-accent' : waiting ? 'border-warn' : 'border-line-strong', className)}
    >
      {above}
      <label htmlFor={id} className="sr-only">
        {label}
      </label>
      <textarea
        id={id}
        aria-label={label}
        rows={rows}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={keyDown}
        onPaste={paste}
        placeholder={placeholder}
        className="resize-none border-0 bg-transparent px-4 pt-3.5 pb-1.5 text-sm leading-normal text-fg-1 placeholder:text-fg-3 focus:outline-none max-md:text-base"
      />
      {below}
      {/* One row however many tools: the agent's settings fold into one chip where they don't fit, and what still
          doesn't scrolls sideways rather than starting a second row. */}
      <div className="flex items-center gap-1 px-2.5 pt-1 pb-2.5">
        <span ref={room} className="min-w-0 flex-1 overflow-x-auto [scrollbar-width:none]">
          <span ref={content} className="flex w-max items-center gap-1 *:shrink-0">
            <ComposerFolded.Provider value={folded}>{tools}</ComposerFolded.Provider>
          </span>
        </span>
        <span className="flex shrink-0 items-center gap-2">{actions}</span>
      </div>
    </div>
  )
}

/** The composer's round button: send, start or stop. */
export function RoundButton({
  label,
  icon,
  onClick,
  disabled,
  type = 'button',
  quiet = false,
  title,
}: {
  label: string
  icon: IconName
  onClick?: () => void
  disabled?: boolean
  type?: 'button' | 'submit'
  /** Stop: drawn on the surface rather than in amber. */
  quiet?: boolean
  title?: string
}) {
  return (
    <button
      type={type}
      aria-label={label}
      title={title ?? label}
      onClick={onClick}
      disabled={disabled}
      className={cx(
        'inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full max-md:h-11 max-md:w-11',
        quiet
          ? 'border border-line-strong bg-surface-2 text-fg-1 hover:bg-surface-3'
          : 'bg-accent text-on-accent hover:bg-accent-hover disabled:bg-surface-2 disabled:text-fg-3',
      )}
    >
      <Icon name={icon} className="h-4 w-4" />
    </button>
  )
}
