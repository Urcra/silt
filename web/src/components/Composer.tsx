import { type ClipboardEvent, type DragEvent, type KeyboardEvent, type ReactNode, useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { AgentCommand } from '../api'
import { filesFrom } from '../attachments'
import { matching, typedCommand } from '../slashCommands'
import { Icon, type IconName, Spinner, cx } from '../ui'
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

/** The agent's commands that match the one being typed, over the box; the one picked fills in its name. */
function CommandMenu({ id, offered, active, onPick }: { id: string; offered: AgentCommand[]; active: number; onPick: (command: AgentCommand) => void }) {
  useEffect(() => {
    document.getElementById(`${id}-command-${active}`)?.scrollIntoView({ block: 'nearest' })
  }, [id, active])
  return (
    <ul
      id={`${id}-commands`}
      role="listbox"
      aria-label="commands"
      className="absolute right-0 bottom-full left-0 z-20 mb-1.5 max-h-64 overflow-y-auto rounded-md border border-line bg-elevated p-1 shadow-overlay"
    >
      {offered.map((command, index) => (
        <li
          key={command.name}
          id={`${id}-command-${index}`}
          role="option"
          aria-selected={index === active}
          // The box keeps the focus, so typing goes on after a click.
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => onPick(command)}
          className={cx('flex cursor-pointer flex-col rounded-sm px-2.5 py-1.5 max-md:min-h-11', index === active && 'bg-surface-2')}
        >
          <span className="font-mono text-xs text-fg-1">
            /{command.name}
            {command.hint && <span className="text-fg-3"> {command.hint}</span>}
          </span>
          {command.description && <span className="truncate text-2xs text-fg-2">{command.description}</span>}
        </li>
      ))}
    </ul>
  )
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
  top,
  above,
  below,
  tools,
  actions,
  waiting = false,
  onCycle,
  onFiles,
  commands = [],
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
  /** Docked on the box's top edge, set off from the rest: what the thread changed. */
  top?: ReactNode
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
  /** The agent's commands, offered as `/` starts the text. */
  commands?: AgentCommand[]
  className?: string
}) {
  const { room, content, folded } = useFolded()
  const [dragging, setDragging] = useState(false)
  const typed = typedCommand(value)
  // The text Escape closed the menu on; it opens again once the text changes.
  const [dismissed, setDismissed] = useState<string>()
  const [chosen, setChosen] = useState(0)
  const offered = typed === null || dismissed === value ? [] : matching(commands, typed)
  const active = Math.min(chosen, offered.length - 1)
  const edit = (text: string) => {
    setChosen(0)
    onChange(text)
  }
  const pick = (command: AgentCommand) => edit(`/${command.name} `)
  const choosing = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    const step = { ArrowDown: 1, ArrowUp: offered.length - 1 }[event.key]
    if (step !== undefined) setChosen((active + step) % offered.length)
    else if ((event.key === 'Enter' || event.key === 'Tab') && !event.shiftKey && !event.nativeEvent.isComposing) pick(offered[active]!)
    else if (event.key === 'Escape') setDismissed(value)
    else return false
    event.preventDefault()
    return true
  }
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
    if (offered.length > 0 && choosing(event)) return
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
      className={cx('relative flex flex-col rounded-lg border bg-elevated', dragging ? 'border-accent' : waiting ? 'border-warn' : 'border-line-strong', className)}
    >
      {offered.length > 0 && <CommandMenu id={id} offered={offered} active={active} onPick={pick} />}
      {top}
      {above}
      <label htmlFor={id} className="sr-only">
        {label}
      </label>
      <textarea
        id={id}
        aria-label={label}
        rows={rows}
        value={value}
        onChange={(event) => edit(event.target.value)}
        onKeyDown={keyDown}
        aria-controls={offered.length > 0 ? `${id}-commands` : undefined}
        aria-activedescendant={offered.length > 0 ? `${id}-command-${active}` : undefined}
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
  busy = false,
  title,
}: {
  label: string
  icon: IconName
  onClick?: () => void
  disabled?: boolean
  type?: 'button' | 'submit'
  /** Stop: drawn on the surface rather than in amber. */
  quiet?: boolean
  /** What it did is under way: a spinner in place of its icon. */
  busy?: boolean
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
      {busy ? <Spinner className="h-4 w-4" /> : <Icon name={icon} className="h-4 w-4" />}
    </button>
  )
}
