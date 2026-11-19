// The viewers for artifact types (ADR 0076): a deck, a canvas of boards, and a data page, drawn by Silt from the
// version's JSON. Slides and boards hold an agent's HTML, so each renders in its own sealed frame; a data page holds no
// HTML at all, and every number it shows names its source.

import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { api } from '../api'
import { type FrameTheme, type InlineFiles, withFilesInlined } from '../artifacts'
import { printInBrowser } from '../printing'
import { hosted } from '../served'
import { useShownTheme } from '../theme'
import {
  type Block,
  type Source,
  canvasOf,
  chartColors,
  chartTable,
  runs,
  stepsOf,
  toolValue,
  withLive,
  xPositions,
  yDomain,
  csvOf,
  dataPageOf,
  deckOf,
  deckPrint,
  layout,
  partDocument,
  shown,
  slideFromHash,
  drawnSeries,
  onPage,
  pagesOf,
  deckDocument,
  NOTE_WIDTH,
} from '../typed'
import { Button, Icon, IconButton, cx } from '../ui'
import { Markdown } from './Markdown'

/** The scale that fits a `width`×`height` stage into the box `ref` measures. */
function useFit(width: number, height: number) {
  const box = useRef<HTMLDivElement>(null)
  const [scale, setScale] = useState(1)
  useEffect(() => {
    const element = box.current
    if (!element) return
    const measure = () => setScale(Math.max(0.05, Math.min(element.clientWidth / width, element.clientHeight / height)))
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [width, height])
  return { box, scale }
}

export function SlidesView({
  text,
  style,
  theme,
  title,
  files,
  onComment,
  onPrint,
}: {
  text: string
  style: string | null
  theme: FrameTheme
  title: string
  /** The version's own files, which slides name by relative path. */
  files?: InlineFiles
  /** Opens the artifact's comment composer, quoting what the comment is on. */
  onComment?: (quote: string) => void
  /** Prints the deck, a slide a page, where this computer can. */
  onPrint?: (html: string) => void
}) {
  const deck = useMemo(() => deckOf(text), [text])
  const count = deck?.slides.length ?? 0
  const [at, setAt] = useState(() => slideFromHash(window.location.hash, count))
  const [presenting, setPresenting] = useState(false)
  const [started, setStarted] = useState(0)
  const [now, setNow] = useState(() => Date.now())
  const [width, height] = deck?.aspect === '4:3' ? [1024, 768] : [1280, 720]
  const { box, scale } = useFit(width, height)
  const room = useRef<HTMLDivElement>(null)
  // The click steps shown of the slide, its `data-step` elements up to it.
  const [step, setStep] = useState(0)
  const go = (to: number) => {
    setAt(Math.min(Math.max(to, 0), count - 1))
    setStep(0)
  }
  const placed = useRef({ at, step, deck })
  // A step forward, or the next slide once its steps are all shown; back, the same in reverse.
  const forward = () => {
    const { at, step, deck } = placed.current
    if (step < stepsOf(deck?.slides[at]?.html ?? '')) setStep(step + 1)
    else if (at < count - 1) {
      setAt(at + 1)
      setStep(0)
    }
  }
  const back = () => {
    const { at, step, deck } = placed.current
    if (step > 0) setStep(step - 1)
    else if (at > 0) {
      setAt(at - 1)
      setStep(stepsOf(deck?.slides[at - 1]?.html ?? ''))
    }
  }
  const moves = useRef({ forward, back, go })
  useEffect(() => {
    placed.current = { at, step, deck }
    moves.current = { forward, back, go }
  })
  // Every slide is in one frame, loaded once; it's told which to show (ADR 0076).
  const stage = useRef<HTMLIFrameElement>(null)
  const staged = useMemo(() => (deck ? deckDocument(deck, style, theme, { files }) : ''), [deck, style, theme, files])
  const tell = (moved: boolean) =>
    stage.current?.contentWindow?.postMessage({ silt: 'show', at, step, fade: deck?.transition === 'fade' && moved }, '*')
  const told = useRef(tell)
  useEffect(() => {
    told.current = tell
  })
  useEffect(() => {
    told.current(true)
  }, [at, step])
  useEffect(() => {
    // Keys pressed in the frame page the deck as they do outside it.
    const heard = (event: MessageEvent) => {
      const said = event.data as { silt?: unknown; key?: unknown } | null
      if (event.source !== stage.current?.contentWindow || said?.silt !== 'key') return
      if (['ArrowRight', 'PageDown', ' '].includes(String(said.key))) moves.current.forward()
      else if (['ArrowLeft', 'PageUp'].includes(String(said.key))) moves.current.back()
    }
    window.addEventListener('message', heard)
    return () => window.removeEventListener('message', heard)
  }, [])
  useEffect(() => {
    const keys = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return
      if (['ArrowRight', 'PageDown', ' '].includes(event.key)) moves.current.forward()
      else if (['ArrowLeft', 'PageUp'].includes(event.key)) moves.current.back()
      else if (event.key === 'Home') moves.current.go(0)
      else if (event.key === 'End') moves.current.go(count - 1)
      else return
      event.preventDefault()
    }
    window.addEventListener('keydown', keys)
    return () => window.removeEventListener('keydown', keys)
  }, [count])
  useEffect(() => {
    if (!presenting) return
    const tick = window.setInterval(() => setNow(Date.now()), 1000)
    const left = () => {
      if (!document.fullscreenElement) setPresenting(false)
    }
    document.addEventListener('fullscreenchange', left)
    return () => {
      window.clearInterval(tick)
      document.removeEventListener('fullscreenchange', left)
    }
  }, [presenting])
  if (!deck || count === 0) return <p className="px-5 py-6 text-sm text-danger">This deck's content isn't one Silt can draw.</p>
  const slide = deck.slides[at]
  const steps = stepsOf(slide.html)
  const present = async () => {
    setStarted(Date.now())
    setNow(Date.now())
    setPresenting(true)
    try {
      await room.current?.requestFullscreen()
    } catch {
      // Without full screen, the presenter view still shows in the panel.
    }
  }
  const elapsed = Math.floor((now - started) / 1000)
  return (
    <div ref={room} className="flex h-full min-h-0 flex-col bg-elevated" data-testid="slides">
      <div className="flex min-h-0 flex-1">
        <div ref={box} className="relative min-w-0 flex-1 overflow-hidden" aria-label={`slide ${at + 1} of ${count}`}>
          <iframe
            ref={stage}
            title={`${title}, slide ${at + 1}: ${slide.title}`}
            sandbox="allow-scripts"
            srcDoc={staged}
            onLoad={() => told.current(false)}
            className="absolute top-1/2 left-1/2 border-0 bg-white shadow-overlay"
            style={{
              width,
              height,
              transform: `translate(-50%, -50%) scale(${scale})`,
            }}
            data-testid="slide-frame"
          />
        </div>
        {presenting && (
          <aside
            className="flex w-72 shrink-0 flex-col gap-3 border-l border-line bg-bg p-4 text-sm"
            aria-label="presenter view"
            data-testid="presenter"
          >
            <p className="font-mono text-2xl tabular-nums" aria-label="time presenting">
              {Math.floor(elapsed / 60)}:{String(elapsed % 60).padStart(2, '0')}
            </p>
            <div>
              <h3 className="font-mono text-2xs tracking-[0.08em] text-fg-2 uppercase">Notes</h3>
              <p className="mt-1 whitespace-pre-wrap text-fg-1">{slide.notes || 'No notes for this slide.'}</p>
            </div>
            <div>
              <h3 className="font-mono text-2xs tracking-[0.08em] text-fg-2 uppercase">Next</h3>
              <p className="mt-1 text-fg-1">{deck.slides[at + 1]?.title ?? 'The end'}</p>
            </div>
          </aside>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2 border-t border-line-subtle px-2 py-1.5">
        <IconButton icon="chevronLeft" label="Previous slide" disabled={at === 0 && step === 0} onClick={back} />
        <span className="font-mono text-xs text-fg-2 tabular-nums">
          {at + 1} / {count}
          {steps > 0 && ` · ${step}/${steps}`}
        </span>
        <IconButton icon="chevronRight" label="Next slide" disabled={at === count - 1 && step >= steps} onClick={forward} />
        <ol className="flex min-w-0 flex-1 gap-1 overflow-x-auto" aria-label="slides">
          {deck.slides.map((each, index) => (
            <li key={index}>
              <button
                type="button"
                aria-current={index === at}
                onClick={() => go(index)}
                className={cx(
                  'max-w-40 truncate rounded-sm border px-2 py-1 text-xs',
                  index === at ? 'border-accent text-fg-1' : 'border-line text-fg-2 hover:border-line-strong',
                )}
              >
                {index + 1}. {each.title}
              </button>
            </li>
          ))}
        </ol>
        {onComment && (
          <IconButton icon="comment" label="Comment on this slide" onClick={() => onComment(`Slide ${at + 1}: ${slide.title}`)} />
        )}
        {onPrint && (
          <IconButton
            icon="download"
            label={hosted() ? 'Print, or save as PDF' : 'Download as PDF'}
            onClick={() => {
              const deckPage = withFilesInlined(deckPrint(deck, width, height, style), files ?? {})
              // The hosted app has no headless browser: this browser's own print dialog saves the PDF.
              if (hosted()) printInBrowser(deckPage)
              else onPrint(deckPage)
            }}
          />
        )}
        <Button
          size="sm"
          onClick={() => void (presenting ? document.exitFullscreen?.().catch(() => setPresenting(false)) : present())}
        >
          {presenting ? 'Stop presenting' : 'Present'}
        </Button>
      </div>
    </div>
  )
}

export function CanvasView({
  text,
  style,
  theme,
  files,
  onComment,
}: {
  text: string
  style: string | null
  theme: FrameTheme
  /** The version's own files, which boards name by relative path. */
  files?: InlineFiles
  /** Opens the artifact's comment composer, quoting what the comment is on. */
  onComment?: (quote: string) => void
}) {
  const canvas = useMemo(() => canvasOf(text), [text])
  const pages = useMemo(() => (canvas ? pagesOf(canvas) : []), [canvas])
  const [page, setPage] = useState(0)
  const shownPage = pages[Math.min(page, pages.length - 1)] ?? ''
  const notes = useMemo(() => (canvas?.notes ?? []).filter((note) => onPage(note, shownPage, pages)), [canvas, shownPage, pages])
  // How tall each board that grows says its content is, from its own frame.
  const [grown, setGrown] = useState<Record<string, number>>({})
  const frames = useRef(new Map<string, HTMLIFrameElement>())
  useEffect(() => {
    const heard = (event: MessageEvent) => {
      const said = event.data as { silt?: unknown; height?: unknown } | null
      if (said?.silt !== 'grown' || typeof said.height !== 'number') return
      for (const [id, frame] of frames.current)
        if (frame.contentWindow === event.source) {
          const height = Math.min(Math.max(Math.round(said.height as number), 1), 8192)
          setGrown((was) => (was[id] === height ? was : { ...was, [id]: height }))
        }
    }
    window.addEventListener('message', heard)
    return () => window.removeEventListener('message', heard)
  }, [])
  const placed = useMemo(
    () =>
      layout(
        (canvas?.boards ?? [])
          .filter((board) => onPage(board, shownPage, pages))
          .map((board) =>
            board.grow && grown[board.id]
              ? { ...board, height: Math.max(board.height, grown[board.id]), least: board.height }
              : board,
          ),
      ),
    [canvas, grown, shownPage, pages],
  )
  const box = useRef<HTMLDivElement>(null)
  const [view, setView] = useState({ x: 40, y: 60, scale: 0.4 })
  const drag = useRef<{ x: number; y: number; from: typeof view } | null>(null)
  // The fingers on the canvas, for a pinch to zoom around their middle.
  const pointers = useRef(new Map<number, { x: number; y: number }>())
  const pinch = useRef<{ distance: number; middle: [number, number]; from: typeof view } | null>(null)
  // Two boards shown side by side at one scale, chosen by their names.
  const [comparing, setComparing] = useState<string[] | null>(null)
  const [size, setSize] = useState({ width: 0, height: 0 })
  const extent = useMemo(
    () => ({
      width: Math.max(
        1,
        ...placed.map((board) => board.x + board.width),
        ...notes.map((note) => note.x + (note.w ?? NOTE_WIDTH)),
      ),
      height: Math.max(1, ...placed.map((board) => board.y + board.height + 40), ...notes.map((note) => note.y + 120)),
    }),
    [placed, notes],
  )
  const fit = () => {
    const element = box.current
    if (!element) return
    const scale = Math.min((element.clientWidth - 80) / extent.width, (element.clientHeight - 100) / extent.height, 1)
    setView({ x: 40, y: 60, scale: Math.max(scale, 0.05) })
  }
  // A new canvas opens with every board in view, and fits again as its panel resizes, until someone moves it.
  const moved = useRef(false)
  useEffect(fit, [extent])
  const fitting = useRef(fit)
  useEffect(() => {
    fitting.current = fit
  })
  // Observed once: a new observer each render reports at once, which drew the canvas again, every frame it was open.
  useEffect(() => {
    const element = box.current
    if (!element) return
    const observer = new ResizeObserver(() => {
      const [width, height] = [element.clientWidth, element.clientHeight]
      setSize((was) => (was.width === width && was.height === height ? was : { width, height }))
      if (!moved.current) fitting.current()
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  // Each board's document, made once: with the version's files inlined, it can be megabytes each.
  const documents = useMemo(
    () =>
      new Map(
        placed.map((board) => [
          board.id,
          partDocument(
            withFilesInlined(board.html, files ?? {}),
            canvas?.style,
            board.width,
            // The least it shows, as written, so its growing doesn't load it again.
            (board as { least?: number }).least ?? board.height,
            style,
            theme,
            undefined,
            board.grow,
          ),
        ]),
      ),
    [placed, files, canvas, style, theme],
  )
  useEffect(() => {
    const element = box.current
    if (!element) return
    const wheel = (event: WheelEvent) => {
      event.preventDefault()
      moved.current = true
      const rect = element.getBoundingClientRect()
      const [px, py] = [event.clientX - rect.left, event.clientY - rect.top]
      setView((was) => {
        const scale = Math.min(Math.max(was.scale * Math.exp(-event.deltaY / 400), 0.05), 4)
        return {
          scale,
          x: px - ((px - was.x) * scale) / was.scale,
          y: py - ((py - was.y) * scale) / was.scale,
        }
      })
    }
    element.addEventListener('wheel', wheel, { passive: false })
    return () => element.removeEventListener('wheel', wheel)
  }, [])
  if (!canvas || canvas.boards.length === 0)
    return <p className="px-5 py-6 text-sm text-danger">This canvas's content isn't one Silt can draw.</p>
  const local = (event: { clientX: number; clientY: number }) => {
    const rect = box.current?.getBoundingClientRect()
    return { x: event.clientX - (rect?.left ?? 0), y: event.clientY - (rect?.top ?? 0) }
  }
  const pinchOf = () => {
    const [a, b] = [...pointers.current.values()]
    return { distance: Math.hypot(a.x - b.x, a.y - b.y), middle: [(a.x + b.x) / 2, (a.y + b.y) / 2] as [number, number] }
  }
  const down = (event: ReactPointerEvent) => {
    if ((event.target as HTMLElement).closest('[data-board-label]')) return
    moved.current = true
    pointers.current.set(event.pointerId, local(event))
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
    if (pointers.current.size === 2) {
      drag.current = null
      pinch.current = { ...pinchOf(), from: view }
      return
    }
    drag.current = { x: event.clientX, y: event.clientY, from: view }
  }
  const move = (event: ReactPointerEvent) => {
    if (pointers.current.has(event.pointerId)) pointers.current.set(event.pointerId, local(event))
    const pinching = pinch.current
    if (pinching && pointers.current.size === 2) {
      const now = pinchOf()
      const scale = Math.min(Math.max(pinching.from.scale * (now.distance / Math.max(pinching.distance, 1)), 0.05), 4)
      const [px, py] = pinching.middle
      setView({
        scale,
        x: now.middle[0] - ((px - pinching.from.x) * scale) / pinching.from.scale,
        y: now.middle[1] - ((py - pinching.from.y) * scale) / pinching.from.scale,
      })
      return
    }
    const from = drag.current
    if (from)
      setView({
        ...from.from,
        x: from.from.x + event.clientX - from.x,
        y: from.from.y + event.clientY - from.y,
      })
  }
  const lift = (event: ReactPointerEvent) => {
    pointers.current.delete(event.pointerId)
    if (pointers.current.size < 2) pinch.current = null
    drag.current = null
  }
  // A board well off screen keeps its place but not its frame, so a large canvas stays light.
  const onScreen = (board: (typeof placed)[number]) => {
    if (size.width === 0) return true
    const margin = 400
    const [left, top] = [view.x + board.x * view.scale, view.y + board.y * view.scale]
    const [right, bottom] = [left + board.width * view.scale, top + board.height * view.scale]
    return right > -margin && bottom > -margin && left < size.width + margin && top < size.height + margin
  }
  const zoomTo = (board: (typeof placed)[number]) => {
    const element = box.current
    if (!element) return
    const scale = Math.min((element.clientWidth - 40) / board.width, (element.clientHeight - 80) / board.height, 1)
    setView({ scale, x: 20 - board.x * scale, y: 50 - board.y * scale })
  }
  return (
    <div className="relative h-full min-h-0 overflow-hidden bg-surface-1" data-testid="canvas">
      <div
        ref={box}
        className="absolute inset-0 cursor-grab touch-none active:cursor-grabbing"
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={lift}
        onPointerCancel={lift}
      >
        <div
          className="absolute top-0 left-0 origin-top-left"
          style={{
            transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})`,
          }}
        >
          {placed.map((board) => (
            <figure
              key={board.id}
              className="absolute m-0"
              style={{ left: board.x, top: board.y, width: board.width }}
              data-testid="board"
            >
              <figcaption
                data-board-label
                className="absolute bottom-full left-0 mb-2 flex items-baseline gap-2 overflow-hidden whitespace-nowrap"
                style={{
                  fontSize: Math.min(14 / view.scale, board.width / 10),
                  maxWidth: board.width,
                }}
              >
                <button type="button" onClick={() => zoomTo(board)} className="font-medium text-fg-1 hover:underline">
                  {board.name}
                </button>
                <span className="truncate text-fg-2">
                  {board.width}×{(board as { least?: number }).least ?? board.height}
                  {board.caption ? ` · ${board.caption}` : ''}
                </span>
                {placed.length > 1 && (
                  <button
                    type="button"
                    onClick={() =>
                      setComparing((was) => {
                        const chosen = [...(was ?? []).filter((name) => name !== board.name), board.name]
                        return chosen.slice(-2)
                      })
                    }
                    className={cx(
                      'hover:underline',
                      comparing?.includes(board.name) ? 'text-accent-fg' : 'text-fg-2 hover:text-fg-1',
                    )}
                    aria-pressed={comparing?.includes(board.name) ?? false}
                  >
                    Compare
                  </button>
                )}
                {onComment && (
                  <button
                    type="button"
                    onClick={() => onComment(`Board ${board.name}`)}
                    className="text-fg-2 hover:text-fg-1 hover:underline"
                    aria-label={`Comment on ${board.name}`}
                  >
                    Comment
                  </button>
                )}
              </figcaption>
              {onScreen(board) ? (
                <iframe
                  title={board.name}
                  sandbox="allow-scripts"
                  ref={(frame) => {
                    if (frame) frames.current.set(board.id, frame)
                    else frames.current.delete(board.id)
                  }}
                  srcDoc={documents.get(board.id)}
                  className="pointer-events-auto block border-0 bg-white shadow-overlay"
                  style={{ width: board.width, height: board.height }}
                />
              ) : (
                <div
                  className="bg-white/60 shadow-overlay"
                  style={{ width: board.width, height: board.height }}
                  data-testid="board-unloaded"
                />
              )}
            </figure>
          ))}
          {notes.map((note, at) => (
            <p
              key={note.id ?? at}
              className="absolute m-0 rounded-sm p-3 whitespace-pre-wrap shadow-overlay"
              // A sticky note's own paper and ink, which read alike on either theme's canvas.
              style={{
                left: note.x,
                top: note.y,
                width: note.w ?? NOTE_WIDTH,
                fontSize: 16,
                lineHeight: 1.4,
                background: '#fff3b0',
                color: '#3d3a2a',
              }}
              data-testid="canvas-note"
            >
              {note.text}
            </p>
          ))}
        </div>
      </div>
      {pages.length > 1 && (
        <div
          role="tablist"
          aria-label="pages"
          className="absolute top-2 left-2 flex gap-1 rounded-md border border-line bg-elevated p-1 shadow-overlay"
        >
          {pages.map((name, at) => (
            <button
              key={name}
              type="button"
              role="tab"
              aria-selected={name === shownPage}
              onClick={() => {
                moved.current = false
                setComparing(null)
                setPage(at)
              }}
              className={cx(
                'rounded-sm px-2 py-1 text-xs',
                name === shownPage ? 'bg-surface-2 text-fg-1' : 'text-fg-2 hover:text-fg-1',
              )}
            >
              {name}
            </button>
          ))}
        </div>
      )}
      <div className="absolute right-2 bottom-2 flex items-center gap-1 rounded-md border border-line bg-elevated p-1 shadow-overlay">
        <IconButton
          icon="plus"
          label="Zoom in"
          onClick={() => setView((was) => ({ ...was, scale: Math.min(was.scale * 1.25, 4) }))}
        />
        <span className="w-12 text-center font-mono text-xs text-fg-2 tabular-nums">{Math.round(view.scale * 100)}%</span>
        <IconButton
          icon="x"
          label="Zoom out"
          onClick={() =>
            setView((was) => ({
              ...was,
              scale: Math.max(was.scale / 1.25, 0.05),
            }))
          }
        />
        <Button size="sm" variant="ghost" onClick={fit}>
          Fit all
        </Button>
      </div>
      {comparing && comparing.length === 2 && (
        <BoardsCompared
          boards={comparing.map((name) => placed.find((board) => board.name === name)).filter((board) => board !== undefined)}
          shared={canvas.style}
          style={style}
          theme={theme}
          files={files}
          onDone={() => setComparing(null)}
        />
      )}
    </div>
  )
}

/** Two boards side by side at one scale, each whole, to tell them apart. */
function BoardsCompared({
  boards,
  shared,
  style,
  theme,
  files,
  onDone,
}: {
  boards: { name: string; width: number; height: number; html: string }[]
  shared: string | undefined
  style: string | null
  theme: FrameTheme
  files?: InlineFiles
  onDone: () => void
}) {
  const box = useRef<HTMLDivElement>(null)
  const [room, setRoom] = useState({ width: 0, height: 0 })
  useEffect(() => {
    const element = box.current
    if (!element) return
    const measure = () => setRoom({ width: element.clientWidth, height: element.clientHeight })
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  const gap = 32
  const widths = boards.reduce((sum, board) => sum + board.width, 0)
  const tallest = Math.max(...boards.map((board) => board.height))
  const scale = Math.max(Math.min((room.width - gap * 3) / widths, (room.height - 80) / tallest, 1), 0.05)
  return (
    <div className="absolute inset-0 z-10 flex flex-col bg-surface-1" data-testid="boards-compared">
      <div className="flex shrink-0 items-center gap-2 border-b border-line-subtle bg-elevated px-3 py-1.5 text-sm">
        <span className="min-w-0 flex-1 truncate text-fg-1">
          {boards[0]?.name} and {boards[1]?.name}, at {Math.round(scale * 100)}%
        </span>
        <Button size="sm" variant="ghost" onClick={onDone}>
          Done
        </Button>
      </div>
      <div ref={box} className="flex min-h-0 flex-1 items-start justify-center overflow-hidden pt-6" style={{ gap }}>
        {boards.map((board) => (
          <figure key={board.name} className="m-0 flex flex-col gap-2">
            <figcaption className="text-xs text-fg-2">
              {board.name} · {board.width}×{board.height}
            </figcaption>
            <div style={{ width: board.width * scale, height: board.height * scale }} className="overflow-hidden shadow-overlay">
              <iframe
                title={`${board.name}, compared`}
                sandbox="allow-scripts"
                srcDoc={partDocument(withFilesInlined(board.html, files ?? {}), shared, board.width, board.height, style, theme)}
                className="block origin-top-left border-0 bg-white"
                style={{ width: board.width, height: board.height, transform: `scale(${scale})` }}
              />
            </div>
          </figure>
        ))}
      </div>
    </div>
  )
}

const CHART = {
  light: ['#009183', '#a8432b', '#855193', '#7a8a2b', '#15669d', '#c2702b'],
  dark: ['#06a596', '#d36f4f', '#b074c4', '#8b9837', '#5194d2', '#bf7f3b'],
}

function saveCsv(name: string, columns: string[], rows: unknown[][]) {
  const url = URL.createObjectURL(new Blob([csvOf(columns, rows)], { type: 'text/csv' }))
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = `${name.replace(/[^A-Za-z0-9_-]+/g, '-').replace(/^-|-$/g, '') || 'data'}.csv`
  anchor.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

function SourceLink({ id, sources }: { id: string; sources: Source[] }) {
  const source = sources.find((candidate) => candidate.id === id)
  return (
    <a
      href={`#source-${id}`}
      className="font-mono text-2xs text-fg-3 hover:text-fg-1"
      title={source ? `${source.detail} · ${source.at}` : id}
    >
      source: {source?.name ?? id}
    </a>
  )
}

/**
 * A chart in a doc, `<?silt chart {…}?>` (ADR 0077): a data page's chart block, checked as one, with Silt's colours
 * for the theme shown, and no source of its own.
 */
export function DocChart({ json }: { json: string }) {
  const theme = useShownTheme()
  const parsed = (() => {
    try {
      return JSON.parse(json) as Record<string, unknown>
    } catch {
      return null
    }
  })()
  const page = parsed
    ? dataPageOf(
        JSON.stringify({
          title: '',
          sources: [{ id: 'doc', name: 'the doc', kind: 'manual', detail: '', at: '' }],
          blocks: [{ ...parsed, type: 'chart', source: 'doc' }],
        }),
      )
    : null
  const block = page?.blocks[0]
  if (!block || block.type !== 'chart')
    return <p className="my-2 text-xs text-danger">This chart doesn't draw: it isn't a chart block of a data page.</p>
  return (
    <figure className="my-3">
      {block.title && <figcaption className="mb-1 text-sm font-medium text-fg-1">{block.title}</figcaption>}
      <Chart block={block} theme={theme} palette={CHART[theme]} />
    </figure>
  )
}

/** A series' colour: the palette's in order, and past its end a grey, so no two series share a hue (ADR 0076). */
function seriesColor(colors: string[], series: number): string {
  return colors[series] ?? '#8a8780'
}

function Chart({ block, theme, palette }: { block: Extract<Block, { type: 'chart' }>; theme: FrameTheme; palette: string[] }) {
  const colors = palette.length >= 6 ? palette : CHART[theme === 'dark' ? 'dark' : 'light']
  const [hover, setHover] = useState<number | null>(null)
  const { columns, rows } = chartTable(block.series)
  // Drawn at the width it's shown, so its labels stay the size of the page's text.
  const holder = useRef<HTMLDivElement>(null)
  const [shownWidth, setShownWidth] = useState(640)
  useEffect(() => {
    const element = holder.current
    if (!element) return
    const measure = () => setShownWidth(Math.max(280, Math.round(element.clientWidth)))
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  // Room for the axes' titles where the page gives them.
  const [W, H, L, B, T, R] = [shownWidth, block.x ? 276 : 260, block.y ? 64 : 48, block.x ? 44 : 28, 12, 12]
  const values = block.series.flatMap((one) => one.points.map(([, y]) => y)).filter((y): y is number => typeof y === 'number')
  const stackedTotals = rows.map((row) => row.slice(1).reduce<number>((sum, y) => sum + (typeof y === 'number' ? y : 0), 0))
  // Bars and areas measure from zero; a line or dots may start near their values.
  const fromZero =
    block.y_zero !== false || ['bar', 'segmented', 'area', 'heatmap'].includes(block.chart) || Boolean(block.stacked)
  const [bottom, top] = yDomain(block.stacked ? [...values, ...stackedTotals] : values, fromZero)
  const span = top - bottom || 1
  const across = xPositions(
    rows.map((row) => row[0]),
    block.x_scale,
  )
  const order = rows.map((_row, index) => index).sort((a, b) => across[a] - across[b])
  const sx = (index: number) => L + (W - L - R) * (across[index] ?? 0.5)
  const sy = (value: number) => T + (H - T - B) * (1 - (value - bottom) / span)
  const ticks = [bottom, bottom + span / 2, top]
  const band = (W - L - R) / Math.max(rows.length, 1)
  const marks: React.ReactNode[] = []
  if (block.chart === 'bar' || block.chart === 'segmented') {
    rows.forEach((row, index) => {
      let base = 0
      const ys = row.slice(1)
      const each = block.stacked || block.chart === 'segmented' ? band * 0.7 : (band * 0.7) / ys.length
      ys.forEach((y, s) => {
        if (typeof y !== 'number') return
        const x = L + band * index + band * 0.15 + (block.stacked || block.chart === 'segmented' ? 0 : each * s)
        const from = block.stacked || block.chart === 'segmented' ? base : 0
        base += y
        const [y0, y1] = [sy(from), sy(from + y)]
        marks.push(
          <rect
            key={`${index}-${s}`}
            x={x + 1}
            y={Math.min(y0, y1)}
            width={Math.max(each - 2, 1)}
            height={Math.max(Math.abs(y0 - y1) - 2, 1)}
            rx={2}
            fill={seriesColor(colors, s)}
          />,
        )
      })
    })
  } else if (block.chart === 'heatmap') {
    const cell = (H - T - B) / block.series.length
    block.series.forEach((_one, s) =>
      rows.forEach((row, index) => {
        const y = row[s + 1]
        if (typeof y !== 'number') return
        marks.push(
          <rect
            key={`${s}-${index}`}
            x={L + band * index + 1}
            y={T + cell * s + 1}
            width={band - 2}
            height={cell - 2}
            fill={colors[0]}
            opacity={0.15 + (0.85 * (y - bottom)) / span}
          />,
        )
      }),
    )
  } else {
    block.series.forEach((_one, s) => {
      const color = seriesColor(colors, s)
      const points = rows
        .map((row, index) => [index, row[s + 1]] as const)
        .filter((point): point is readonly [number, number] => typeof point[1] === 'number')
      // A missing value breaks the line, in the order the x scale puts them.
      const path = runs(order.map((index) => rows[index][s + 1]))
        .map((run) => run.map(([at, y], step) => `${step === 0 ? 'M' : 'L'}${sx(order[at])},${sy(y)}`).join(''))
        .join('')
      if (block.chart === 'line' || block.chart === 'area')
        marks.push(<path key={`l${s}`} d={path} fill="none" stroke={color} strokeWidth={2} />)
      if (block.chart === 'area' && points.length > 1)
        marks.push(
          <path
            key={`a${s}`}
            d={`${path}L${sx(points[points.length - 1][0])},${sy(Math.max(bottom, 0))}L${sx(points[0][0])},${sy(Math.max(bottom, 0))}Z`}
            fill={color}
            opacity={0.15}
          />,
        )
      if (block.chart === 'dot' || block.chart === 'scatter')
        for (const [index, y] of points)
          marks.push(
            <circle
              key={`d${s}-${index}`}
              cx={sx(index)}
              cy={sy(y)}
              r={4}
              fill={color}
              stroke="var(--color-elevated, #fff)"
              strokeWidth={2}
            />,
          )
    })
  }
  const everyNth = Math.max(1, Math.ceil(rows.length / Math.max(2, Math.floor((W - L - R) / 56))))
  return (
    <div ref={holder}>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        width={W}
        height={H}
        role="img"
        aria-label={`${block.title ?? block.chart}: ${block.series.map((one) => one.name).join(', ')}`}
        className="w-full text-fg-2"
        onMouseLeave={() => setHover(null)}
      >
        {ticks.map((tick) => (
          <g key={tick}>
            <line x1={L} x2={W - R} y1={sy(tick)} y2={sy(tick)} stroke="currentColor" strokeOpacity={0.15} />
            <text x={L - 6} y={sy(tick) + 4} textAnchor="end" fontSize={11} fill="currentColor">
              {shown(Number(tick.toPrecision(3)))}
            </text>
          </g>
        ))}
        {marks}
        {rows.map((_row, index) => (
          <rect
            key={`h${index}`}
            x={L + band * index}
            y={T}
            width={band}
            height={H - T - B}
            fill="transparent"
            onMouseEnter={() => setHover(index)}
          />
        ))}
        {rows.map((row, index) =>
          index % everyNth !== 0 ? null : (
            <text
              key={`x${index}`}
              x={
                block.chart === 'line' || block.chart === 'area' || block.chart === 'dot' || block.chart === 'scatter'
                  ? sx(index)
                  : L + band * (index + 0.5)
              }
              y={H - B + 18}
              textAnchor="middle"
              fontSize={11}
              fill="currentColor"
            >
              {String(row[0])}
            </text>
          ),
        )}
        {block.x && (
          <text x={L + (W - L - R) / 2} y={H - 6} textAnchor="middle" fontSize={11} fill="currentColor">
            {block.x}
          </text>
        )}
        {block.y && (
          <text
            x={14}
            y={T + (H - T - B) / 2}
            transform={`rotate(-90 14 ${T + (H - T - B) / 2})`}
            textAnchor="middle"
            fontSize={11}
            fill="currentColor"
          >
            {block.y}
          </text>
        )}
        {hover !== null && (
          <g pointerEvents="none">
            <rect
              x={Math.min(L + band * hover, W - 170)}
              y={T}
              width={160}
              height={18 + 16 * block.series.length}
              rx={4}
              fill="var(--color-elevated, #fff)"
              stroke="currentColor"
              strokeOpacity={0.3}
            />
            <text x={Math.min(L + band * hover, W - 170) + 8} y={T + 14} fontSize={11} fill="currentColor">
              {String(rows[hover][0])}
            </text>
            {block.series.map((one, s) => (
              <text
                key={s}
                x={Math.min(L + band * hover, W - 170) + 8}
                y={T + 30 + 16 * s}
                fontSize={11}
                fill={seriesColor(colors, s)}
              >
                {one.name}: {shown(rows[hover][s + 1])}
              </text>
            ))}
          </g>
        )}
        <desc>{columns.join(', ')}</desc>
      </svg>
      {block.chart === 'heatmap' && (
        <div className="mt-1 flex items-center gap-2 text-xs text-fg-2" aria-label="scale">
          <span className="tabular-nums">{shown(Number(bottom.toPrecision(3)))}</span>
          <span
            aria-hidden="true"
            className="h-2 w-28 rounded-sm"
            style={{ background: `linear-gradient(to right, color-mix(in srgb, ${colors[0]} 15%, transparent), ${colors[0]})` }}
          />
          <span className="tabular-nums">{shown(Number(top.toPrecision(3)))}</span>
        </div>
      )}
    </div>
  )
}

export function DataView({
  text,
  theme,
  style,
  live,
}: {
  text: string
  theme: FrameTheme
  style?: string | null
  /** Where the page's `mcp` sources are run from: its thread and version (ADR 0084). */
  live?: { thread: string; artifact: string; version: number }
}) {
  // By what it names, not the object: a new one each time the panel drew ran every source's tools again.
  const [liveThread, liveArtifact, liveVersion] = [live?.thread, live?.artifact, live?.version]
  const sourced = useMemo(
    () =>
      liveThread !== undefined && liveArtifact !== undefined && liveVersion !== undefined
        ? { thread: liveThread, artifact: liveArtifact, version: liveVersion }
        : undefined,
    [liveThread, liveArtifact, liveVersion],
  )
  const written = useMemo(() => dataPageOf(text), [text])
  const [tables, setTables] = useState<Set<number>>(new Set())
  const sources = useMemo(
    () => (written?.sources ?? []).filter((source) => source.kind === 'mcp' && source.server && source.tool),
    [written],
  )
  // Whether the person let the page run its sources' tools; null until known.
  const [allowed, setAllowed] = useState<boolean | null>(null)
  const [fetched, setFetched] = useState<Record<string, { value?: unknown; at: string; error?: string }>>({})
  useEffect(() => {
    if (!sourced || sources.length === 0) return
    let gone = false
    api.artifactGrants(sourced.thread, sourced.artifact).then(
      (grants) => !gone && setAllowed(grants.mcp === 'granted'),
      () => !gone && setAllowed(false),
    )
    return () => {
      gone = true
    }
  }, [sourced, sources.length])
  const refresh = useCallback(async () => {
    if (!sourced) return
    const answers: Record<string, { value?: unknown; at: string; error?: string }> = {}
    for (const source of sources) {
      const at = new Date().toISOString()
      try {
        // Only a tool the server says only reads runs on its own.
        const about = (await api.pageMcp(sourced.thread, sourced.artifact, {
          op: 'describe',
          server: source.server,
          tool: source.tool,
        })) as { read_only?: boolean }
        if (!about.read_only) throw new Error(`${source.server}'s ${source.tool} doesn't only read, so it doesn't run on its own`)
        const result = await api.pageMcp(sourced.thread, sourced.artifact, {
          op: 'call',
          server: source.server,
          tool: source.tool,
          input: source.input ?? {},
        })
        answers[source.id] = { value: toolValue(result), at }
      } catch (failure) {
        answers[source.id] = { at, error: failure instanceof Error ? failure.message : String(failure) }
      }
    }
    setFetched(answers)
  }, [sourced, sources])
  useEffect(() => {
    // `refresh` sets state only once the page's tools have answered.
    // oxlint-disable-next-line react/set-state-in-effect
    if (allowed) void refresh()
  }, [allowed, refresh])
  const page = useMemo(() => {
    if (!written) return null
    const answers = Object.fromEntries(
      Object.entries(fetched).flatMap(([id, got]) => (got.error === undefined ? [[id, got.value]] : [])),
    )
    return withLive(written, answers)
  }, [written, fetched])
  if (!page) return <p className="px-5 py-6 text-sm text-danger">This data page's content isn't one Silt can draw.</p>
  const colors = chartColors(style, theme === 'dark' ? 'dark' : 'light') ?? CHART[theme === 'dark' ? 'dark' : 'light']
  const toggle = (index: number) =>
    setTables((was) => {
      const next = new Set(was)
      if (!next.delete(index)) next.add(index)
      return next
    })
  const stats = page.blocks.filter((block) => block.type === 'stat')
  return (
    <div className="h-full overflow-auto" data-testid="data-page">
      <article className="mx-auto flex max-w-4xl flex-col gap-5 px-6 pt-6 pb-10">
        <header>
          <h1 className="text-xl font-semibold text-balance text-fg-1">{page.title}</h1>
          {sourced && sources.length > 0 && (
            <div
              className="mt-2 flex flex-wrap items-center gap-2 rounded-md border border-line bg-surface-1 px-3 py-2 text-xs text-fg-2"
              data-testid="live-sources"
            >
              {allowed === false ? (
                <>
                  <span className="min-w-0 flex-1">
                    Live numbers from {sources.map((source) => `${source.server}'s ${source.tool}`).join(', ')}, run on this
                    computer.
                  </span>
                  <Button
                    size="sm"
                    onClick={() =>
                      void api
                        .setArtifactGrant(sourced.thread, sourced.artifact, 'mcp', 'granted', sourced.version)
                        .then(() => setAllowed(true))
                    }
                  >
                    Load them
                  </Button>
                </>
              ) : (
                <>
                  <span className="min-w-0 flex-1">
                    {Object.keys(fetched).length === 0
                      ? 'Loading live numbers…'
                      : sources
                          .map((source) => {
                            const got = fetched[source.id]
                            if (!got) return `${source.name}: not yet`
                            return got.error
                              ? `${source.name}: ${got.error}`
                              : `${source.name}: fetched ${new Date(got.at).toLocaleTimeString()}`
                          })
                          .join(' · ')}
                  </span>
                  <Button size="sm" variant="ghost" onClick={() => void refresh()} disabled={!allowed}>
                    Refresh
                  </Button>
                </>
              )}
            </div>
          )}
          {page.summary && <p className="mt-1 text-sm text-fg-2">{page.summary}</p>}
        </header>
        {stats.length > 0 && (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(10rem,1fr))] gap-3">
            {stats.map((block, index) => (
              <div key={index} className="flex flex-col gap-1 rounded-md border border-line bg-elevated p-3" data-testid="stat">
                <span className="text-xs text-fg-2">{block.label}</span>
                <span className="text-2xl font-semibold text-fg-1 tabular-nums">
                  {shown(block.value)}
                  {block.unit && <span className="ml-1 text-sm font-normal text-fg-2">{block.unit}</span>}
                </span>
                {typeof block.change === 'number' && (
                  <span
                    className={cx(
                      'text-xs',
                      !block.better || block.change === 0
                        ? 'text-fg-2'
                        : block.change > 0 === (block.better === 'up')
                          ? 'text-ok'
                          : 'text-danger',
                    )}
                  >
                    {block.change > 0 ? '▲' : block.change < 0 ? '▼' : '='} {shown(Math.abs(block.change * 100))}%
                  </span>
                )}
                <SourceLink id={block.source} sources={page.sources} />
              </div>
            ))}
          </div>
        )}
        {page.blocks.map((block, index) => {
          if (block.type === 'stat') return null
          if (block.type === 'text')
            return (
              <div key={index} className="text-sm text-fg-1">
                <Markdown text={block.text} />
              </div>
            )
          const table = block.type === 'table' ? { columns: block.columns, rows: block.rows } : chartTable(block.series)
          const asTable = block.type === 'table' || tables.has(index)
          return (
            <section
              key={index}
              className="flex flex-col gap-2 rounded-md border border-line bg-elevated p-3"
              aria-label={block.title ?? `block ${index + 1}`}
            >
              <div className="flex items-center gap-2">
                <h2 className="min-w-0 flex-1 truncate text-sm font-medium text-fg-1">
                  {block.title ?? (block.type === 'chart' ? block.chart : 'Table')}
                </h2>
                {block.type === 'chart' && (
                  <Button size="sm" variant="ghost" aria-pressed={asTable} onClick={() => toggle(index)}>
                    {asTable ? 'Chart' : 'Table'}
                  </Button>
                )}
                <IconButton
                  icon="download"
                  label="Download as CSV"
                  onClick={() => saveCsv(block.title ?? page.title, table.columns, table.rows)}
                />
              </div>
              {block.type === 'chart' && !asTable && (
                <>
                  <Chart block={{ ...block, series: drawnSeries(block.chart, block.series) }} theme={theme} palette={colors} />
                  {block.series.length > 1 && (
                    <ul className="flex flex-wrap gap-3 text-xs text-fg-2" aria-label="legend">
                      {drawnSeries(block.chart, block.series).map((one, s) => (
                        <li key={one.name} className="inline-flex items-center gap-1.5">
                          <span
                            aria-hidden="true"
                            className="h-2 w-2 rounded-full"
                            style={{ background: seriesColor(colors, s) }}
                          />
                          {one.name}
                        </li>
                      ))}
                    </ul>
                  )}
                </>
              )}
              {asTable && (
                <div className="overflow-x-auto">
                  <table className="w-full text-left text-xs">
                    <thead>
                      <tr>
                        {table.columns.map((column) => (
                          <th key={column} className="border-b border-line px-2 py-1 font-medium text-fg-2">
                            {column}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {table.rows.map((row, at) => (
                        <tr key={at}>
                          {row.map((cell, c) => (
                            <td
                              key={c}
                              className={cx(
                                'border-b border-line-subtle px-2 py-1 text-fg-1',
                                typeof cell === 'number' && 'text-right tabular-nums',
                              )}
                            >
                              {shown(cell)}
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
              <SourceLink id={block.source} sources={page.sources} />
            </section>
          )
        })}
        {page.sources.length > 0 && (
          <footer className="border-t border-line pt-3">
            <h2 className="font-mono text-2xs tracking-[0.08em] text-fg-2 uppercase">Sources</h2>
            <ul className="mt-2 flex flex-col gap-2 text-xs">
              {page.sources.map((source) => (
                <li key={source.id} id={`source-${source.id}`} className="flex flex-col">
                  <span className="text-fg-1">
                    <Icon name="info" className="mr-1 inline h-3 w-3" />
                    {source.name} <span className="text-fg-3">· {source.kind}</span>
                  </span>
                  <code className="break-all text-fg-2">{source.detail}</code>
                  <span className="text-fg-3">{source.at}</span>
                </li>
              ))}
            </ul>
          </footer>
        )}
      </article>
    </div>
  )
}
