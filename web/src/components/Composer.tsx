import {
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import type { AgentCommand } from '../api'
import { filesFrom } from '../attachments'
import { matching, typedCommand } from '../slashCommands'
import { Icon, Menu, type MenuItem, cx } from '../ui'
import { DICTATION_KEYS, hasVoiceNote, insertWords, showKeyHint, taking, useDictation } from '../voice'
import { ComposerFolded } from './composerFolded'
import { RoundButton } from './RoundButton'
import { useComposerHistory } from './useComposerHistory'
import { Keys, VoiceBar, VoiceButton, VoiceNote, VoiceOffer } from './VoiceButton'
import { SnowDrift } from './Seasonal'

export { RoundButton }

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

/** The pane each box is in, found once: looking for it walked every ancestor's style at each key. */
const panes = new WeakMap<HTMLElement, HTMLElement | null>()

/** Half the pane the composer is in, past which the box scrolls rather than grows. */
function growLimit(element: HTMLElement): number {
  let pane = panes.get(element)
  if (pane === undefined || (pane && !pane.contains(element))) {
    pane = null
    for (let parent = element.parentElement; parent; parent = parent.parentElement) {
      const { overflowY } = getComputedStyle(parent)
      if (overflowY === 'auto' || overflowY === 'scroll') {
        pane = parent
        break
      }
    }
    panes.set(element, pane)
  }
  return (pane?.clientHeight ?? window.innerHeight) / 2
}

/** How long each box's text was when it was last fitted. */
const fitted = new WeakMap<HTMLTextAreaElement, number>()

/** Makes `element` as tall as its text, from its `rows` up to half the pane, and keeps it so as the pane resizes. */
function growWithText(element: HTMLTextAreaElement): () => void {
  const fit = (force = true) => {
    // Hidden during a take, it measures nothing and would collapse.
    if (element.offsetParent === null) return
    // Text added that still fits needs no measuring: shrinking the box to measure it laid out the whole page at
    // each key, which beside a large diff took tens of milliseconds.
    const before = fitted.get(element)
    fitted.set(element, element.value.length)
    if (!force && before !== undefined && element.value.length >= before && element.scrollHeight <= element.clientHeight) return
    element.style.height = 'auto'
    const limit = growLimit(element)
    element.style.height = `${Math.min(element.scrollHeight, limit)}px`
    element.style.overflowY = element.scrollHeight > limit ? 'auto' : 'hidden'
  }
  fit(false)
  const resized = () => fit()
  window.addEventListener('resize', resized)
  if (typeof ResizeObserver === 'undefined') return () => window.removeEventListener('resize', resized)
  let width = element.clientWidth
  const rewrapped = new ResizeObserver(() => {
    if (element.clientWidth === width) return
    width = element.clientWidth
    fit()
  })
  rewrapped.observe(element)
  return () => {
    rewrapped.disconnect()
    window.removeEventListener('resize', resized)
  }
}

/** The agent's commands that match the one being typed, over the box; the one picked fills in its name. */
function CommandMenu({
  id,
  offered,
  active,
  onPick,
}: {
  id: string
  offered: AgentCommand[]
  active: number
  onPick: (command: AgentCommand) => void
}) {
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
          className={cx(
            'flex cursor-pointer flex-col rounded-sm px-2.5 py-1.5 max-md:min-h-11',
            index === active && 'bg-surface-2',
          )}
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

/** Whether the pointer is a finger, where ⏎ on the keyboard breaks the line and a button has to send. */
function useTouch(): boolean {
  const query = '(pointer: coarse)'
  const [touch, setTouch] = useState(() => window.matchMedia?.(query).matches ?? false)
  useEffect(() => {
    const media = window.matchMedia?.(query)
    if (!media) return undefined
    const changed = () => setTouch(media.matches)
    media.addEventListener('change', changed)
    return () => media.removeEventListener('change', changed)
  }, [])
  return touch
}

/** The box's text during a take, with the words heard so far in grey where they'll go, and the caret as a level meter. */
function Heard({
  value,
  start,
  end,
  heard,
  loudness,
  height,
  limit,
  onKeyDown,
}: {
  value: string
  start: number
  end: number
  heard: string
  loudness: number
  height: number | undefined
  limit: number | undefined
  onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => void
}) {
  const shown = useRef<HTMLDivElement>(null)
  const caret = useRef<HTMLSpanElement>(null)
  // It takes the focus from the box, so ⏎, Esc and the dictation key still reach the take.
  useEffect(() => shown.current?.focus(), [])
  useEffect(() => {
    caret.current?.scrollIntoView({ block: 'nearest' })
  }, [heard])
  const next = heard ? insertWords(value, start, end, heard) : { value: value.slice(0, start) + value.slice(end), caret: start }
  return (
    <div
      ref={shown}
      tabIndex={-1}
      aria-live="polite"
      onKeyDown={onKeyDown}
      style={{ minHeight: height, maxHeight: limit }}
      className="min-w-0 flex-1 overflow-y-auto px-1.5 py-1.5 text-sm leading-normal break-words whitespace-pre-wrap text-fg-1 focus:outline-none max-md:text-base"
    >
      {next.value.slice(0, heard ? start : next.caret)}
      {heard && (
        <span aria-label="heard so far" className="bg-[linear-gradient(transparent_84%,var(--accent-tint)_84%)] text-fg-3">
          {next.value.slice(start, next.caret)}
        </span>
      )}
      <span
        ref={caret}
        aria-hidden="true"
        className="mx-px inline-block w-[3px] rounded-full bg-accent align-[-2px] transition-[height] duration-100"
        style={{ height: `${4 + Math.round(loudness * 12)}px` }}
      />
      {next.value.slice(next.caret)}
    </div>
  )
}

/** The touch keyboard's way to send: ⏎ there breaks the line. */
export interface TouchSend {
  label: string
  title: string
  disabled?: boolean
  busy?: boolean
  /** Submits the form the composer is in rather than calling `onSend`. */
  submit?: boolean
}

/**
 * The one composer (ADR 0044, ADR 0062, ADR 0063): + for what goes with the prompt, the text, and the
 * microphone; ⏎ sends. Stacked, on New thread and New chat, the text fills the box over a row of + and the
 * microphone. In line, under a thread, the box is one line of text, with + and the microphone under it. Under
 * the box, the agent's settings, or the take while one is recorded.
 */
export function ComposerBox({
  id,
  label,
  value,
  onChange,
  onSend,
  placeholder,
  rows = 3,
  layout = 'stacked',
  above,
  below,
  tools,
  actions,
  note,
  menu = [],
  attach,
  send,
  enter = 'send',
  waiting = false,
  onCycle,
  onFiles,
  commands = [],
  voice = false,
  suggestion,
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
  /** The box's least height, in lines; it grows with the text up to half the pane. */
  rows?: number
  layout?: 'stacked' | 'inline'
  /** Over the text: what's queued, what others are writing. */
  above?: ReactNode
  /** Over the text, under `above`: attached files. */
  below?: ReactNode
  /** Under the box, on the right: the agent and its settings. */
  tools?: ReactNode
  /** In the box on the right, such as stopping the agent's turn. */
  actions?: ReactNode
  /** Under the box, on the left, such as steering now. */
  note?: ReactNode
  /** More of + 's menu, after attaching and the commands. */
  menu?: MenuItem[]
  /** What + 's Attach takes, where `onFiles` takes files. */
  attach?: { label: string; accept?: string }
  /** The send button on a touch screen; none elsewhere, where ⏎ sends. */
  send?: TouchSend
  /** What ⏎ does, for the hint: send, queue, start. */
  enter?: string
  /** The agent waits on the person, so the box says so. */
  waiting?: boolean
  /** Shift+Tab: the agent's next mode, as in its own terminal. */
  onCycle?: () => void
  /** Files pasted, dropped or attached through +; without it, the box takes none. */
  onFiles?: (files: File[], pasted: boolean) => void
  /** The agent's commands, offered as `/` starts the text. */
  commands?: AgentCommand[]
  /** Offers dictation, where this machine can do it. */
  voice?: boolean
  /** The prompt the agent suggests next, grey in the empty box until Tab or → takes it (ADR 0110). */
  suggestion?: string
  className?: string
}) {
  const box = useRef<HTMLTextAreaElement>(null)
  const touch = useTouch()
  const [hint] = useState(showKeyHint)
  // The draft that dictation will send, and where the caret goes, once it's in the box.
  const dictatedToSend = useRef<string | null>(null)
  const dictatedCaret = useRef<number | null>(null)
  const dictation = useDictation({
    enabled: voice,
    composer: id,
    onBegin: () =>
      setFrozen({
        start: box.current?.selectionStart ?? value.length,
        end: box.current?.selectionEnd ?? value.length,
        height: box.current?.offsetHeight,
        limit: box.current ? growLimit(box.current) : undefined,
      }),
    onText: (words, sendIt) => {
      const next = insertWords(
        value,
        box.current?.selectionStart ?? value.length,
        box.current?.selectionEnd ?? value.length,
        words,
      )
      dictatedCaret.current = next.caret
      dictatedToSend.current = sendIt ? next.value : null
      edit(next.value)
    },
  })
  const take = dictation.available && taking(dictation.phase)
  // Where the words will go and the box's height, as they were when the take began, which the words heard keep to.
  const [frozen, setFrozen] = useState<{ start: number; end: number; height: number | undefined; limit: number | undefined }>()
  useLayoutEffect(() => (box.current ? growWithText(box.current) : undefined), [value])
  useLayoutEffect(() => {
    if (dictatedCaret.current === null) return
    box.current?.focus()
    box.current?.setSelectionRange(dictatedCaret.current, dictatedCaret.current)
    dictatedCaret.current = null
  }, [value])
  // A take thrown away or failed gives the box its focus back.
  const tookBefore = useRef(false)
  useEffect(() => {
    if (tookBefore.current && !take && dictatedCaret.current === null) box.current?.focus()
    tookBefore.current = take
  }, [take])
  useEffect(() => {
    if (dictatedToSend.current !== null && dictatedToSend.current === value) {
      dictatedToSend.current = null
      onSend(false)
    }
  })
  const { room, content, folded } = useFolded()
  const [dragging, setDragging] = useState(false)
  const typed = typedCommand(value)
  // The text Escape closed the menu on; it opens again once the text changes.
  const [dismissed, setDismissed] = useState<string>()
  const [chosen, setChosen] = useState(0)
  const history = useComposerHistory(id, value, box, onChange)
  const offered = typed === null || dismissed === value || take ? [] : matching(commands, typed)
  const active = Math.min(chosen, offered.length - 1)
  const edit = (text: string, kind = '') => {
    setChosen(0)
    history.edit(text, kind)
  }
  const pick = (command: AgentCommand) => edit(`/${command.name} `)
  const ghost = suggestion && value === '' && !take ? suggestion : undefined
  const takeSuggestion = (text: string) => {
    dictatedToSend.current = null
    edit(text)
  }
  const choosing = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    const step = { ArrowDown: 1, ArrowUp: offered.length - 1 }[event.key]
    if (step !== undefined) setChosen((active + step) % offered.length)
    else if ((event.key === 'Enter' || event.key === 'Tab') && !event.shiftKey && !event.nativeEvent.isComposing)
      pick(offered[active]!)
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
    // Only into an empty box: there Tab would otherwise leave it, and → did nothing.
    if (
      ghost &&
      (event.key === 'Tab' || event.key === 'ArrowRight') &&
      !event.shiftKey &&
      !event.altKey &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.nativeEvent.isComposing
    ) {
      event.preventDefault()
      takeSuggestion(ghost)
      return
    }
    if (history.keyDown(event)) {
      dictatedToSend.current = null
      setChosen(0)
      return
    }
    if (offered.length > 0 && choosing(event)) return
    if (dictation.onKey(event)) return
    // ⏎ sends and shift+⏎ breaks the line, as in a chat; on a touch keyboard, with no shift, ⏎ breaks it and
    // the button sends. Never while an input method is still composing a character.
    if (
      event.key === 'Enter' &&
      !event.nativeEvent.isComposing &&
      !event.shiftKey &&
      !event.altKey &&
      (!touch || event.metaKey || event.ctrlKey)
    ) {
      event.preventDefault()
      onSend(event.metaKey || event.ctrlKey)
    }
    if (event.key === 'Tab' && event.shiftKey && onCycle) {
      event.preventDefault()
      onCycle()
    }
  }
  // By id rather than ref: these run from the menu's items, which are made as the box renders.
  const choose = () => document.getElementById(`${id}-attach`)?.click()
  const slash = () => {
    edit('/')
    document.getElementById(id)?.focus()
  }
  const plus: MenuItem[] = [
    ...(onFiles && attach ? [{ label: attach.label, icon: 'attach' as const, onSelect: choose }] : []),
    ...(commands.length > 0
      ? [
          {
            label: 'Slash commands',
            icon: 'terminal' as const,
            hint: '/',
            disabled: value.trim() !== '',
            onSelect: slash,
          },
        ]
      : []),
    ...menu.map((item, index) => ({ ...item, divided: index === 0 && (Boolean(onFiles && attach) || commands.length > 0) })),
  ]
  const heard = [...dictation.phrases, dictation.partial].filter(Boolean).join(' ')
  const left =
    (dictation.available && hasVoiceNote(dictation) && <VoiceNote dictation={dictation} />) ||
    note ||
    (hint && ghost && (
      <span className="inline-flex items-center gap-1 text-xs whitespace-nowrap text-fg-3 max-md:hidden">
        <Keys keys={['Tab']} /> or <Keys keys={['→']} /> use the suggestion · <Keys keys={['⏎']} /> {enter}
      </span>
    )) ||
    (hint && value === '' && (
      <span className="inline-flex items-center gap-1 text-xs whitespace-nowrap text-fg-3 max-md:hidden">
        <Keys keys={['⏎']} /> {enter}
        {dictation.available && (
          <>
            {' '}
            · hold <Keys keys={DICTATION_KEYS} /> to talk
          </>
        )}
      </span>
    ))
  const inline = layout === 'inline'
  const adding = plus.length > 0 && (
    <Menu
      above
      align="left"
      label="add to the prompt"
      items={plus}
      triggerClassName="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-fg-2 hover:bg-surface-2 hover:text-fg-1 aria-expanded:bg-surface-2 aria-expanded:text-fg-1 max-md:h-11 max-md:w-11"
      trigger={<Icon name="plus" className="h-[18px] w-[18px]" />}
    />
  )
  const microphone = dictation.available && <VoiceButton dictation={dictation} />
  const touchSend = touch && send && !take && (
    <RoundButton
      type={send.submit ? 'submit' : 'button'}
      label={send.label}
      title={send.title}
      icon="arrowUp"
      disabled={send.disabled}
      busy={send.busy}
      onClick={send.submit ? undefined : () => onSend(false)}
    />
  )
  // A touch keyboard has no Tab, so a button takes the suggestion there.
  const touchUse = touch && ghost && (
    <button
      type="button"
      title="Puts the suggested prompt in the box"
      onClick={() => {
        takeSuggestion(ghost)
        box.current?.focus()
      }}
      className="inline-flex h-11 shrink-0 items-center rounded-md border border-line px-3 text-sm text-fg-1 hover:bg-surface-2"
    >
      Use
    </button>
  )
  const ending = (actions || touchUse || touchSend) && (
    <span className="flex shrink-0 items-center gap-1">
      {actions}
      {touchUse}
      {touchSend}
    </span>
  )
  return (
    <div className={cx('flex flex-col', className)}>
      <div
        onDragOver={dragOver}
        onDragLeave={() => setDragging(false)}
        onDrop={drop}
        className={cx(
          'relative flex flex-col rounded-xl border bg-elevated shadow-raised',
          dragging ? 'border-accent' : waiting ? 'border-warn' : 'border-line-strong',
        )}
      >
        <SnowDrift />
        {offered.length > 0 && <CommandMenu id={id} offered={offered} active={active} onPick={pick} />}
        {!inline && dictation.available && <VoiceOffer dictation={dictation} side="right" />}
        {above}
        {below}
        {onFiles && attach && (
          <input
            id={`${id}-attach`}
            type="file"
            multiple
            aria-label="attach files"
            className="sr-only"
            tabIndex={-1}
            accept={attach.accept}
            onChange={(event) => {
              const files = [...(event.target.files ?? [])]
              event.target.value = ''
              onFiles(files, false)
            }}
          />
        )}
        <div className={cx('flex gap-1 p-1.5', inline ? 'items-end' : 'flex-col')}>
          {/* A row of its own: as a column's flex-1, the text would shrink rather than grow the box. */}
          <div className={cx('flex min-w-0 items-end', inline && 'flex-1')}>
            <label htmlFor={id} className="sr-only">
              {label}
            </label>
            {/* The suggestion shares the text's cell, so a long one grows the box; the text stays mounted under it. */}
            <div className={cx('grid min-w-0 flex-1', take && 'hidden')}>
              <textarea
                ref={box}
                id={id}
                aria-label={label}
                aria-describedby={ghost ? `${id}-suggestion` : undefined}
                rows={rows}
                value={value}
                onChange={(event) => {
                  dictatedToSend.current = null
                  edit(event.target.value, (event.nativeEvent as InputEvent).inputType)
                }}
                onSelect={history.select}
                onKeyDown={keyDown}
                aria-controls={offered.length > 0 ? `${id}-commands` : undefined}
                aria-activedescendant={offered.length > 0 ? `${id}-command-${active}` : undefined}
                onPaste={paste}
                placeholder={ghost ? '' : placeholder}
                className="col-start-1 row-start-1 w-full min-w-0 resize-none border-0 bg-transparent px-1.5 py-1.5 text-sm leading-normal text-fg-1 placeholder:text-fg-3 focus:outline-none max-md:text-base"
              />
              {ghost && (
                <div
                  id={`${id}-suggestion`}
                  aria-hidden="true"
                  onMouseDown={(event) => {
                    event.preventDefault()
                    box.current?.focus()
                  }}
                  className="col-start-1 row-start-1 min-w-0 cursor-text px-1.5 py-1.5 text-sm leading-normal break-words whitespace-pre-wrap text-fg-3 max-md:text-base"
                >
                  {ghost}
                  {!touch && (
                    <span className="ml-1.5 inline-flex -translate-y-px align-middle">
                      <Keys keys={['Tab']} />
                    </span>
                  )}
                </div>
              )}
            </div>
            {take && frozen && (
              <Heard
                value={value}
                start={Math.min(frozen.start, value.length)}
                end={Math.min(frozen.end, value.length)}
                heard={heard}
                loudness={dictation.levels[dictation.levels.length - 1] ?? 0}
                height={frozen.height}
                limit={frozen.limit}
                onKeyDown={(event) => void dictation.onKey(event)}
              />
            )}
          </div>
          {inline ? (
            ending
          ) : (
            <span className="flex items-center gap-1">
              {adding}
              <span className="ml-auto flex items-center gap-1">
                {microphone}
                {ending}
              </span>
            </span>
          )}
        </div>
      </div>
      {/* Under the box, one row: + and the microphone in line, a note or hint, and the agent's settings on the right,
          folded into one chip where they don't fit; while a take runs, the take in their place. */}
      <div className="flex min-h-9 items-center gap-2 px-1 pt-1">
        {take ? (
          <>
            {/* It stays where it was pressed, to be pressed again to keep the words. */}
            {inline && microphone}
            <VoiceBar dictation={dictation} />
          </>
        ) : (
          <>
            {inline && (adding || microphone) && (
              <span className="relative flex shrink-0 items-center gap-0.5">
                {adding}
                {microphone}
                {dictation.available && <VoiceOffer dictation={dictation} side="left" />}
              </span>
            )}
            {/* The settings come first: a note or hint gives way, cut short, where both don't fit. */}
            {left && <span className="flex max-w-[60%] min-w-0 shrink items-center gap-2 overflow-hidden">{left}</span>}
          </>
        )}
        <span ref={room} className={cx('min-w-0 flex-1 overflow-x-auto [scrollbar-width:none]', take && 'hidden')}>
          <span ref={content} className="ml-auto flex w-max items-center gap-0.5 *:shrink-0">
            <ComposerFolded.Provider value={folded}>{tools}</ComposerFolded.Provider>
          </span>
        </span>
      </div>
    </div>
  )
}
