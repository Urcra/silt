// Images and videos as Silt shows them (ADR 0121): an image people zoom and pan, Silt's own video player, and the
// viewer that opens either over the whole window.

import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { api, type BlobState } from '../api'
import { clock, fetchWithProgress, sizeOf, type ImageMedia, type Media, type VideoMedia } from '../mediaInfo'
import { Icon, type IconName, Spinner, cx } from '../ui'
import {
  fitted,
  MAX_SCALE,
  pannable,
  panned,
  pastPreview,
  percent,
  placed,
  STEP,
  type Size,
  type View,
  visible,
  zoomedAt,
} from '../zoom'

const BAR_BUTTON =
  'inline-flex h-7 min-w-7 items-center justify-center gap-1.5 rounded-sm px-1.5 text-xs whitespace-nowrap text-fg-2 hover:bg-surface-2 hover:text-fg-1 aria-pressed:bg-surface-2 aria-pressed:text-fg-1 max-md:min-h-11 max-md:min-w-11'

function BarButton({
  icon,
  label,
  onClick,
  pressed,
  children,
}: {
  icon?: IconName
  label: string
  onClick: () => void
  pressed?: boolean
  children?: ReactNode
}) {
  return (
    <button type="button" aria-label={label} title={label} aria-pressed={pressed} onClick={onClick} className={BAR_BUTTON}>
      {icon && <Icon name={icon} className="h-4 w-4" />}
      {children}
    </button>
  )
}

// ---------- The zoomable image ----------

type Full = { state: 'none' } | { state: 'loading'; loaded: number } | { state: 'done' } | { state: 'failed'; error: string }

/**
 * An image people zoom with the wheel or a pinch and pan by dragging, with − % + Fit 1:1, a navigator once it's larger
 * than its stage, and Full resolution for the original when it shows a preview. `layout` puts the tools in the
 * viewer's bar, or floats them over the image in a panel or a page.
 */
export function ZoomableImage({
  media,
  layout,
  title,
  actions,
  className,
}: {
  media: ImageMedia
  layout: 'viewer' | 'floating'
  /** The viewer bar's title, left of the tools. */
  title?: ReactNode
  /** The viewer bar's buttons after the tools. */
  actions?: ReactNode
  className?: string
}) {
  const stage = useRef<HTMLDivElement>(null)
  const [room, setRoom] = useState<Size>({ width: 0, height: 0 })
  const [natural, setNatural] = useState<Size | null>(
    media.width && media.height ? { width: media.width, height: media.height } : null,
  )
  const [own, setOwn] = useState<View | null>(null)
  const [src, setSrc] = useState(media.src)
  const [full, setFull] = useState<Full>(media.full ? { state: 'none' } : { state: 'done' })
  const [failed, setFailed] = useState(false)
  const loading = useRef<AbortController | null>(null)
  const pointers = useRef(new Map<number, { x: number; y: number }>())
  const pinch = useRef<{ distance: number; scale: number } | null>(null)
  const [dragging, setDragging] = useState(false)
  const ready = natural !== null && room.width > 0 && room.height > 0
  const view = ready ? (own ? placed(own, room, natural) : fitted(room, natural)) : null
  // The wheel's listener reads the view as it stands, not as it was when added.
  const latest = useRef({ view, room, natural })
  useLayoutEffect(() => {
    latest.current = { view, room, natural }
  })

  useEffect(() => {
    const element = stage.current
    if (!element) return
    const observer = new ResizeObserver(([seen]) => setRoom({ width: seen.contentRect.width, height: seen.contentRect.height }))
    observer.observe(element)
    // The wheel zooms rather than scrolling the page, so its listener can't be passive.
    const wheel = (event: WheelEvent) => {
      const { view: at, room: size, natural: image } = latest.current
      if (!at || !image) return
      event.preventDefault()
      const box = element.getBoundingClientRect()
      const factor = Math.exp(-event.deltaY * (event.ctrlKey ? 0.01 : 0.0018))
      setOwn(zoomedAt(at, at.scale * factor, event.clientX - box.left, event.clientY - box.top, size, image))
    }
    element.addEventListener('wheel', wheel, { passive: false })
    return () => {
      observer.disconnect()
      element.removeEventListener('wheel', wheel)
    }
  }, [])
  useEffect(() => () => loading.current?.abort(), [])
  useEffect(
    () => () => {
      if (src.startsWith('blob:')) URL.revokeObjectURL(src)
    },
    [src],
  )

  const zoomTo = (scale: number, px = room.width / 2, py = room.height / 2) => {
    if (view && natural) setOwn(zoomedAt(view, scale, px, py, room, natural))
  }
  const fit = () => setOwn(null)
  const loadFull = () => {
    if (!media.full || full.state === 'loading' || full.state === 'done') return
    const controller = new AbortController()
    loading.current = controller
    setFull({ state: 'loading', loaded: 0 })
    fetchWithProgress(media.full.src, (loaded) => setFull({ state: 'loading', loaded }), controller.signal).then(
      (blob) => {
        setSrc(URL.createObjectURL(blob))
        setFull({ state: 'done' })
      },
      (error: unknown) =>
        setFull(
          controller.signal.aborted
            ? { state: 'none' }
            : { state: 'failed', error: error instanceof Error ? error.message : String(error) },
        ),
    )
  }
  const stopFull = () => loading.current?.abort()
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.metaKey || event.ctrlKey || event.altKey) return
    const step = 80
    const keys: Record<string, () => void> = {
      '+': () => view && zoomTo(view.scale * STEP),
      '=': () => view && zoomTo(view.scale * STEP),
      '-': () => view && zoomTo(view.scale / STEP),
      '0': fit,
      '1': () => zoomTo(1),
      f: loadFull,
      ArrowLeft: () => view && natural && setOwn(panned(view, step, 0, room, natural)),
      ArrowRight: () => view && natural && setOwn(panned(view, -step, 0, room, natural)),
      ArrowUp: () => view && natural && setOwn(panned(view, 0, step, room, natural)),
      ArrowDown: () => view && natural && setOwn(panned(view, 0, -step, room, natural)),
    }
    if (event.key === 'Escape' && full.state === 'loading') {
      event.stopPropagation()
      stopFull()
      return
    }
    const run = keys[event.key]
    if (!run) return
    event.preventDefault()
    event.stopPropagation()
    run()
  }
  const box = () => stage.current?.getBoundingClientRect()
  const onPointerDown = (event: React.PointerEvent) => {
    stage.current?.setPointerCapture(event.pointerId)
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY })
    if (pointers.current.size === 1) setDragging(true)
    if (pointers.current.size === 2 && view) {
      const [a, b] = [...pointers.current.values()]
      pinch.current = { distance: Math.hypot(a.x - b.x, a.y - b.y), scale: view.scale }
    }
  }
  const onPointerMove = (event: React.PointerEvent) => {
    const before = pointers.current.get(event.pointerId)
    if (!before || !view || !natural) return
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY })
    if (pointers.current.size === 2 && pinch.current) {
      const [a, b] = [...pointers.current.values()]
      const at = box()
      if (!at) return
      const scale = (pinch.current.scale * Math.hypot(a.x - b.x, a.y - b.y)) / pinch.current.distance
      setOwn(zoomedAt(view, scale, (a.x + b.x) / 2 - at.left, (a.y + b.y) / 2 - at.top, room, natural))
    } else if (pointers.current.size === 1) {
      setOwn(panned(view, event.clientX - before.x, event.clientY - before.y, room, natural))
    }
  }
  const onPointerUp = (event: React.PointerEvent) => {
    pointers.current.delete(event.pointerId)
    if (pointers.current.size < 2) pinch.current = null
    if (pointers.current.size === 0) setDragging(false)
  }
  const onDoubleClick = (event: React.MouseEvent) => {
    if (!view || !natural) return
    const at = box()
    const fits = fitted(room, natural)
    if (view.scale > fits.scale * 1.05) fit()
    else if (at) zoomTo(Math.max(1, fits.scale * 2.5), event.clientX - at.left, event.clientY - at.top)
  }

  const showingFull = full.state === 'done'
  const past = view !== null && natural !== null && !showingFull && pastPreview(view, natural, media.shownWidth)
  const large = view !== null && natural !== null && pannable(view, room, natural)
  const atFit = view !== null && natural !== null && Math.abs(view.scale - fitted(room, natural).scale) < 0.002
  const seen = view && natural ? visible(view, room, natural) : null
  const resolution =
    showingFull || !media.full
      ? natural
        ? `${natural.width}×${natural.height}`
        : ''
      : `Preview · ${media.shownWidth ?? ''}×${natural && media.shownWidth ? Math.round((natural.height * media.shownWidth) / natural.width) : ''}`

  const tools = (
    <>
      <span role="group" aria-label="zoom" className="inline-flex items-center gap-px rounded-sm bg-surface-1 p-0.5">
        <BarButton icon="minus" label="Zoom out (−)" onClick={() => view && zoomTo(view.scale / STEP)} />
        <span className="min-w-11 text-center font-mono text-2xs text-fg-1 tabular-nums" data-testid="zoom-level">
          {view ? percent(view.scale) : '–'}
        </span>
        <BarButton icon="plus" label="Zoom in (+)" onClick={() => view && zoomTo(Math.min(MAX_SCALE, view.scale * STEP))} />
      </span>
      <BarButton label="Fit (0)" pressed={atFit} onClick={fit}>
        Fit
      </BarButton>
      {layout === 'viewer' && (
        <BarButton label="Actual size (1)" pressed={view !== null && Math.abs(view.scale - 1) < 0.002} onClick={() => zoomTo(1)}>
          1:1
        </BarButton>
      )}
    </>
  )
  const fullButton = media.full && (
    <FullResolution full={full} size={media.full.size} nudge={past} onLoad={loadFull} onStop={stopFull} />
  )
  const body = (
    <div
      ref={stage}
      tabIndex={0}
      role="img"
      aria-label={`${media.title}, ${view ? percent(view.scale) : ''}: scroll or pinch to zoom, drag to pan`}
      onKeyDown={onKeyDown}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onDoubleClick={onDoubleClick}
      className={cx(
        'relative min-h-0 touch-none overflow-hidden bg-surface-1 outline-none select-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-inset',
        large && (dragging ? 'cursor-grabbing' : 'cursor-grab'),
        layout === 'floating' && 'h-full',
      )}
      data-testid="zoom-stage"
    >
      {failed && media.state ? (
        <div className="absolute inset-0 grid place-items-center p-4">
          <Unavailable state={media.state} kind="image" onBack={() => setFailed(false)} />
        </div>
      ) : (
        <img
          src={src}
          alt=""
          draggable={false}
          onLoad={(event) => {
            const loaded = { width: event.currentTarget.naturalWidth, height: event.currentTarget.naturalHeight }
            if (media.width && media.height) return
            // Without its size given, the original's pixels replace the preview's, and the view keeps its size on screen.
            if (natural && natural.width !== loaded.width && own) {
              const ratio = natural.width / loaded.width
              setOwn({ ...own, scale: own.scale * ratio })
            }
            if (!natural || natural.width !== loaded.width) setNatural(loaded)
          }}
          onError={() => setFailed(true)}
          className={cx(
            'absolute top-0 left-0 max-w-none origin-top-left bg-white shadow-raised',
            view && view.scale >= 3 && showingFull && '[image-rendering:pixelated]',
            !view && 'invisible',
          )}
          style={
            natural && view
              ? {
                  width: natural.width,
                  height: natural.height,
                  transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})`,
                }
              : undefined
          }
        />
      )}
      {!view && !failed && (
        <span className="absolute inset-0 grid place-items-center">
          <Spinner />
        </span>
      )}
      {seen && large && (
        <div
          aria-hidden="true"
          className="absolute right-4 bottom-4 w-44 rounded-md border border-line bg-elevated p-1 shadow-overlay max-md:hidden"
        >
          <div className="relative">
            <img src={src} alt="" className="block w-full rounded-sm" draggable={false} />
            <i
              className="absolute rounded-[1px] border-[1.5px] border-accent bg-accent-tint"
              style={{
                left: `${seen.left * 100}%`,
                top: `${seen.top * 100}%`,
                width: `${seen.width * 100}%`,
                height: `${seen.height * 100}%`,
              }}
            />
          </div>
        </div>
      )}
      {layout === 'viewer' && (
        <p
          className={cx(
            'absolute bottom-4 left-4 flex max-w-[calc(100%-14rem)] items-center gap-2 rounded-md border bg-elevated px-2.5 py-1.5 text-2xs shadow-raised max-md:hidden',
            past ? 'border-accent text-accent-fg' : 'border-line text-fg-2',
          )}
        >
          {past ? (
            <>
              <Icon name="info" className="h-3.5 w-3.5" />
              Zoomed past the preview’s detail. <Key>F</Key> loads the full image.
            </>
          ) : (
            <>
              Scroll or pinch to zoom · drag to pan · <Key>0</Key> fits · <Key>1</Key> actual size
            </>
          )}
        </p>
      )}
      {layout === 'floating' && (
        <div className="absolute top-2 right-2 flex flex-wrap items-center justify-end gap-1 rounded-md border border-line bg-elevated p-0.5 shadow-raised">
          {tools}
          {fullButton}
        </div>
      )}
    </div>
  )
  if (layout === 'floating') return <div className={cx('relative h-full', className)}>{body}</div>
  return (
    <div className={cx('grid min-h-0 grid-rows-[auto_minmax(0,1fr)]', className)}>
      <div className="flex min-w-0 flex-wrap items-center gap-1 border-b border-line-subtle px-2 py-1.5 md:h-12 md:flex-nowrap md:py-0 md:pl-4">
        <div className="flex min-w-0 flex-1 flex-col">
          {title}
          <span className="truncate text-2xs text-fg-3 tabular-nums" data-testid="viewer-resolution">
            {resolution}
            {media.meta ? ` · ${media.meta}` : ''}
          </span>
        </div>
        <span className="flex items-center gap-1 max-md:order-last max-md:w-full max-md:justify-between">
          {tools}
          {fullButton && <span className="mx-1 h-4 w-px bg-line max-md:hidden" />}
          {fullButton}
        </span>
        <span className="mx-1 h-4 w-px bg-line max-md:hidden" />
        {actions}
      </div>
      {body}
    </div>
  )
}

function Key({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex min-w-[18px] justify-center rounded-sm border border-line bg-surface-1 px-1 font-mono text-2xs text-fg-2">
      {children}
    </kbd>
  )
}

/** Loads the original of an image shown as its preview, filling as it comes. */
function FullResolution({
  full,
  size,
  nudge,
  onLoad,
  onStop,
}: {
  full: Full
  size: number
  nudge: boolean
  onLoad: () => void
  onStop: () => void
}) {
  const base =
    'relative isolate inline-flex h-7 items-center gap-1.5 overflow-hidden rounded-sm border px-2 text-xs whitespace-nowrap max-md:min-h-11'
  if (full.state === 'done')
    return (
      <span className={cx(base, 'border-transparent text-ok')} data-testid="full-resolution">
        <Icon name="check" className="h-3.5 w-3.5" />
        Full resolution
      </span>
    )
  if (full.state === 'loading')
    return (
      <button
        type="button"
        onClick={onStop}
        aria-label="stop loading the full image (Esc)"
        className={cx(base, 'border-line text-fg-1')}
        data-testid="full-resolution"
      >
        <span
          aria-hidden="true"
          className="absolute inset-y-0 left-0 -z-10 bg-accent-tint"
          style={{ width: `${Math.min(100, (full.loaded / Math.max(1, size)) * 100)}%` }}
        />
        <Icon name="download" className="h-3.5 w-3.5" />
        Loading {sizeOf(full.loaded)} of {sizeOf(size)}
        <span className="font-mono text-2xs text-fg-3">· Esc stops</span>
      </button>
    )
  return (
    <button
      type="button"
      onClick={onLoad}
      title={full.state === 'failed' ? full.error : 'Load the full image (F)'}
      className={cx(
        base,
        nudge ? 'border-accent bg-accent-tint text-accent-fg' : 'border-line bg-bg text-fg-1 hover:border-line-strong',
      )}
      data-testid="full-resolution"
    >
      <Icon name={full.state === 'failed' ? 'reload' : 'download'} className="h-3.5 w-3.5" />
      {full.state === 'failed' ? 'Try the full image again' : 'Full resolution'}
      <span className="font-mono text-2xs text-fg-3">{sizeOf(size)}</span>
    </button>
  )
}

// ---------- The player ----------

const RATES = [0.25, 0.5, 1, 1.25, 1.5, 2]
/** The video playing, so starting another pauses it: one plays at a time. */
let playing: HTMLVideoElement | null = null

/**
 * Silt's video player: play, seek with what has loaded shaded, volume, time, speed, and in the viewer loop, picture
 * in picture and the keys. `inline` sits in the conversation with a poster frame and pauses once scrolled away.
 */
export function VideoPlayer({
  media,
  variant,
  startAt,
  autoFocus,
  onExpand,
  onMetadata,
  className,
}: {
  media: VideoMedia
  variant: 'inline' | 'fill' | 'panel'
  startAt?: number
  autoFocus?: boolean
  /** Opens the viewer at the moment shown. */
  onExpand?: (at: number) => void
  /** Hears the video's length and pixels once it knows them, for its caption. */
  onMetadata?: (seen: { duration: number; width: number; height: number }) => void
  className?: string
}) {
  const root = useRef<HTMLDivElement>(null)
  const video = useRef<HTMLVideoElement>(null)
  const track = useRef<HTMLDivElement>(null)
  const [started, setStarted] = useState(variant !== 'inline' || startAt !== undefined)
  const [paused, setPaused] = useState(true)
  const [time, setTime] = useState(startAt ?? 0)
  const [duration, setDuration] = useState(0)
  const [buffered, setBuffered] = useState(0)
  const [volume, setVolume] = useState(1)
  const [muted, setMuted] = useState(false)
  const [rate, setRate] = useState(1)
  const [loop, setLoop] = useState(false)
  const [menu, setMenu] = useState(false)
  const [help, setHelp] = useState(false)
  const [idle, setIdle] = useState(false)
  const [waiting, setWaiting] = useState(false)
  const [hover, setHover] = useState<number | null>(null)
  const [failed, setFailed] = useState<'unplayable' | 'missing' | null>(null)
  const scrubbing = useRef(false)
  const idleTimer = useRef<number | undefined>(undefined)
  const full = variant !== 'inline'

  useEffect(() => {
    const element = video.current
    if (!element) return
    if (startAt) element.currentTime = startAt
    if (autoFocus) root.current?.focus()
    if (variant === 'fill' && startAt !== undefined) void element.play().catch(() => {})
    // Scrolled out of the conversation, it stops.
    const observer =
      variant === 'inline'
        ? new IntersectionObserver(([seen]) => {
            if (!seen.isIntersecting && !element.paused) element.pause()
          })
        : null
    if (root.current) observer?.observe(root.current)
    return () => {
      observer?.disconnect()
      if (playing === element) playing = null
    }
    // Only when it first shows: the props it reads are where it starts.
    // oxlint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  useEffect(() => () => window.clearTimeout(idleTimer.current), [])

  const element = () => video.current
  const toggle = () => {
    const at = element()
    if (!at) return
    setStarted(true)
    if (at.paused) void at.play().catch(() => {})
    else at.pause()
  }
  const seek = (to: number) => {
    const at = element()
    if (!at || !Number.isFinite(at.duration)) return
    at.currentTime = Math.min(at.duration, Math.max(0, to))
    setTime(at.currentTime)
    setStarted(true)
  }
  const setSpeed = (next: number) => {
    const at = element()
    if (at) at.playbackRate = next
    setRate(next)
    setMenu(false)
  }
  const fullscreen = () => {
    const box = root.current
    if (!box) return
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => {})
    else void box.requestFullscreen?.().catch(() => {})
  }
  const wake = () => {
    setIdle(false)
    window.clearTimeout(idleTimer.current)
    idleTimer.current = window.setTimeout(() => {
      if (!element()?.paused && !menu && !help) setIdle(true)
    }, 2500)
  }
  const fraction = (clientX: number) => {
    const box = track.current?.getBoundingClientRect()
    if (!box || box.width === 0) return 0
    return Math.min(1, Math.max(0, (clientX - box.left) / box.width))
  }
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.metaKey || event.ctrlKey || event.altKey) return
    const at = element()
    if (!at) return
    const frame = 1 / 30
    const keys: Record<string, () => void> = {
      ' ': toggle,
      k: toggle,
      j: () => seek(at.currentTime - 5),
      l: () => seek(at.currentTime + 5),
      ArrowLeft: () => seek(at.currentTime - 5),
      ArrowRight: () => seek(at.currentTime + 5),
      ',': () => at.paused && seek(at.currentTime - frame),
      '.': () => at.paused && seek(at.currentTime + frame),
      m: () => {
        at.muted = !at.muted
      },
      f: fullscreen,
      '<': () => setSpeed(RATES[Math.max(0, RATES.indexOf(rate) - 1)]),
      '>': () => setSpeed(RATES[Math.min(RATES.length - 1, RATES.indexOf(rate) + 1)]),
      '?': () => full && setHelp((open) => !open),
    }
    for (let digit = 0; digit <= 9; digit++) keys[String(digit)] = () => seek(((at.duration || 0) * digit) / 10)
    if (event.key === 'Escape' && (menu || help)) {
      event.stopPropagation()
      setMenu(false)
      setHelp(false)
      return
    }
    const run = keys[event.key]
    if (!run) return
    event.preventDefault()
    event.stopPropagation()
    run()
    wake()
  }
  const played = duration ? (time / duration) * 100 : 0
  const controlsShown = started && !(idle && !paused)
  const button =
    'inline-flex h-7 min-w-7 items-center justify-center gap-1 rounded-sm px-1.5 text-xs text-[#f4ecdd] hover:bg-white/15 focus-visible:outline-2 focus-visible:outline-amber-400 aria-pressed:text-[#e8b97a] max-md:h-9 max-md:min-w-9'

  if (failed && media.state)
    return (
      <div
        className={cx(
          'grid w-full place-items-center bg-surface-1 p-4',
          variant === 'fill' ? 'h-full' : 'aspect-video rounded-md border border-line',
          variant === 'inline' && 'max-w-[420px]',
          className,
        )}
      >
        <Unavailable state={media.state} kind="video" mediaType={media.mediaType} download={media.download} />
      </div>
    )
  return (
    <div
      ref={root}
      role="group"
      aria-label={`video: ${media.title}`}
      tabIndex={0}
      onKeyDown={onKeyDown}
      onPointerMove={wake}
      className={cx(
        'group/player relative isolate overflow-hidden bg-[#0d0b09] text-[#f4ecdd] outline-none',
        variant !== 'fill' && 'focus-visible:ring-2 focus-visible:ring-accent',
        variant === 'inline' && 'aspect-video w-[420px] max-w-full rounded-md border border-line',
        variant === 'panel' && 'aspect-video w-full rounded-md border border-line',
        variant === 'fill' && 'h-full w-full',
        idle && !paused && 'cursor-none',
        className,
      )}
      data-testid="video-player"
    >
      <video
        ref={video}
        src={media.src}
        preload="metadata"
        playsInline
        loop={loop}
        onClick={toggle}
        onPlay={(event) => {
          if (playing && playing !== event.currentTarget) playing.pause()
          playing = event.currentTarget
          setPaused(false)
          setStarted(true)
          wake()
        }}
        onPause={() => {
          setPaused(true)
          setIdle(false)
        }}
        onTimeUpdate={(event) => {
          if (!scrubbing.current) setTime(event.currentTarget.currentTime)
        }}
        onDurationChange={(event) => setDuration(event.currentTarget.duration)}
        onLoadedMetadata={(event) => {
          const seen = event.currentTarget
          setDuration(seen.duration)
          onMetadata?.({ duration: seen.duration, width: seen.videoWidth, height: seen.videoHeight })
        }}
        onProgress={(event) => {
          const ranges = event.currentTarget.buffered
          if (ranges.length > 0) setBuffered(ranges.end(ranges.length - 1))
        }}
        onVolumeChange={(event) => {
          setVolume(event.currentTarget.volume)
          setMuted(event.currentTarget.muted)
        }}
        onWaiting={() => setWaiting(true)}
        onPlaying={() => setWaiting(false)}
        onCanPlay={() => setWaiting(false)}
        onError={(event) => setFailed(event.currentTarget.error?.code === 4 ? 'unplayable' : 'missing')}
        className="block h-full w-full object-contain"
      />
      {!started && (
        <>
          <button
            type="button"
            aria-label={`play ${media.title}`}
            onClick={toggle}
            className="absolute inset-0 z-10 m-auto grid h-13 w-13 place-items-center rounded-full border border-white/20 bg-[#140e08]/60 transition-transform hover:scale-105 hover:bg-[#140e08]/80"
          >
            <Icon name="playSolid" className="ml-0.5 h-5.5 w-5.5" />
          </button>
          {duration > 0 && (
            <span className="absolute right-2 bottom-2 z-10 rounded-sm bg-black/60 px-1.5 py-1 font-mono text-2xs leading-none tabular-nums">
              {clock(duration)}
            </span>
          )}
        </>
      )}
      {waiting && !paused && (
        <span
          role="status"
          aria-label="loading"
          className="absolute inset-0 z-10 m-auto h-9 w-9 animate-spin rounded-full border-[3px] border-white/25 border-t-amber-400"
        />
      )}
      <div
        className={cx(
          'absolute inset-x-0 bottom-0 z-20 flex flex-col gap-0.5 bg-gradient-to-b from-transparent to-[#080604]/80 px-2 pt-6 pb-1.5 transition-opacity',
          controlsShown ? 'opacity-100' : 'pointer-events-none opacity-0',
        )}
      >
        <div
          ref={track}
          role="slider"
          aria-label="seek"
          aria-valuemin={0}
          aria-valuemax={Math.round(duration)}
          aria-valuenow={Math.round(time)}
          aria-valuetext={`${clock(time)} of ${clock(duration)}`}
          tabIndex={-1}
          onPointerDown={(event) => {
            event.currentTarget.setPointerCapture(event.pointerId)
            scrubbing.current = true
            seek(fraction(event.clientX) * duration)
          }}
          onPointerMove={(event) => {
            setHover(fraction(event.clientX))
            if (scrubbing.current) seek(fraction(event.clientX) * duration)
          }}
          onPointerUp={() => {
            scrubbing.current = false
          }}
          onPointerLeave={() => setHover(null)}
          className="group/track relative h-4 cursor-pointer touch-none"
        >
          <span className="absolute inset-x-0 top-1.5 h-1 rounded-full bg-white/20 group-hover/track:top-[5px] group-hover/track:h-1.5" />
          <span
            className="absolute top-1.5 left-0 h-1 rounded-full bg-white/40 group-hover/track:top-[5px] group-hover/track:h-1.5"
            style={{ width: `${duration ? (buffered / duration) * 100 : 0}%` }}
          />
          <span
            className="absolute top-1.5 left-0 h-1 rounded-full bg-amber-400 group-hover/track:top-[5px] group-hover/track:h-1.5"
            style={{ width: `${played}%` }}
          />
          <span
            className="absolute top-0.5 -ml-1.5 h-3 w-3 scale-0 rounded-full bg-amber-300 shadow-[0_0_0_3px_rgb(0_0_0/0.25)] transition-transform group-hover/track:scale-100"
            style={{ left: `${played}%` }}
          />
          {hover !== null && (
            <span
              className="pointer-events-none absolute bottom-5 -translate-x-1/2 rounded-sm bg-[#080604]/90 px-1.5 py-0.5 font-mono text-2xs whitespace-nowrap"
              style={{ left: `${hover * 100}%` }}
            >
              {clock(hover * duration)}
            </span>
          )}
        </div>
        <div className="flex min-w-0 items-center gap-0.5">
          <button type="button" className={button} aria-label={paused ? 'play (k)' : 'pause (k)'} onClick={toggle}>
            <Icon name={paused ? 'playSolid' : 'pause'} className="h-4 w-4" />
          </button>
          <span className="group/volume inline-flex items-center">
            <button
              type="button"
              className={button}
              aria-label={muted ? 'unmute (m)' : 'mute (m)'}
              onClick={() => {
                const at = element()
                if (at) at.muted = !at.muted
              }}
            >
              <Icon name={muted || volume === 0 ? 'muted' : 'sound'} className="h-4 w-4" />
            </button>
            <input
              type="range"
              aria-label="volume"
              min={0}
              max={1}
              step={0.05}
              value={muted ? 0 : volume}
              onChange={(event) => {
                const at = element()
                if (!at) return
                at.volume = Number(event.target.value)
                at.muted = false
              }}
              className="hidden w-16 accent-amber-300 group-focus-within/volume:block group-hover/volume:block max-md:hidden!"
            />
          </span>
          <span className="px-1.5 font-mono text-2xs whitespace-nowrap text-[#e3d8c4] tabular-nums" data-testid="video-time">
            {clock(time)} / {clock(duration)}
          </span>
          <span className="flex-1" />
          <span className="relative">
            <button
              type="button"
              className={button}
              aria-label="speed"
              aria-haspopup="menu"
              aria-expanded={menu}
              onClick={() => setMenu((open) => !open)}
            >
              {rate}×
            </button>
            {menu && (
              <div
                role="menu"
                aria-label="playback speed"
                className="absolute right-0 bottom-9 z-30 min-w-40 rounded-md border border-line bg-elevated p-1 text-xs text-fg-1 shadow-overlay"
              >
                <p className="px-2 pt-1 pb-1 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">Speed</p>
                {RATES.map((option) => (
                  <button
                    key={option}
                    type="button"
                    role="menuitemradio"
                    aria-checked={option === rate}
                    onClick={() => setSpeed(option)}
                    className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left hover:bg-surface-2"
                  >
                    <Icon name="check" className={cx('h-3.5 w-3.5 text-accent-fg', option !== rate && 'invisible')} />
                    {option === 1 ? 'Normal' : `${option}×`}
                  </button>
                ))}
              </div>
            )}
          </span>
          {full && (
            <>
              <button type="button" className={button} aria-label="loop" aria-pressed={loop} onClick={() => setLoop((on) => !on)}>
                <Icon name="loop" className="h-4 w-4" />
              </button>
              {'pictureInPictureEnabled' in document && (
                <button
                  type="button"
                  className={cx(button, 'max-md:hidden')}
                  aria-label="picture in picture"
                  onClick={() =>
                    void element()
                      ?.requestPictureInPicture?.()
                      .catch(() => {})
                  }
                >
                  <Icon name="pip" className="h-4 w-4" />
                </button>
              )}
            </>
          )}
          {onExpand && (
            <button
              type="button"
              className={button}
              aria-label="open in the viewer"
              onClick={() => {
                element()?.pause()
                onExpand(element()?.currentTime ?? 0)
              }}
            >
              <Icon name="expand" className="h-4 w-4" />
            </button>
          )}
          <button type="button" className={button} aria-label="full screen (f)" onClick={fullscreen}>
            <Icon name="fullscreen" className="h-4 w-4" />
          </button>
        </div>
      </div>
      {help && <KeysHelp onClose={() => setHelp(false)} />}
    </div>
  )
}

function KeysHelp({ onClose }: { onClose: () => void }) {
  const rows: [string[], string][] = [
    [['Space', 'K'], 'Play or pause'],
    [['J', 'L', '←', '→'], 'Back or on 5 seconds'],
    [[',', '.'], 'A frame back or on, while paused'],
    [['0', '9'], 'Jump to 0%–90%'],
    [['<', '>'], 'Slower or faster'],
    [['M'], 'Mute'],
    [['F'], 'Full screen'],
    [['Esc'], 'Close the viewer'],
  ]
  return (
    <div
      role="dialog"
      aria-label="keys"
      className="absolute top-3 right-3 z-30 w-72 rounded-md border border-line bg-elevated px-3 py-2.5 text-xs text-fg-1 shadow-overlay"
    >
      <p className="mb-1.5 flex items-center text-sm font-medium">
        Keys
        <button
          type="button"
          aria-label="close the keys"
          onClick={onClose}
          className="ml-auto rounded-sm p-1 text-fg-2 hover:bg-surface-2"
        >
          <Icon name="x" className="h-3.5 w-3.5" />
        </button>
      </p>
      <dl className="grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-1.5">
        {rows.map(([keys, what]) => (
          <div key={what} className="contents">
            <dt className="flex justify-end gap-0.5">
              {keys.map((key) => (
                <Key key={key}>{key}</Key>
              ))}
            </dt>
            <dd className="text-fg-2">{what}</dd>
          </div>
        ))}
      </dl>
    </div>
  )
}

// ---------- Not here yet ----------

/** A file larger than this goes to the server after its messages, and may be refused there (ADR 0121). */
const CHUNK = 8 << 20

/**
 * What an image or a video shows when it can't be read: arriving from the machine that made it, with how much has
 * come; not uploaded yet; refused by the server for its size; arrived damaged and coming again; or a format this
 * browser can't play. Each names the machine it comes from when the server knows it.
 */
export function Unavailable({
  state,
  kind,
  mediaType,
  download,
  onBack,
}: {
  state: NonNullable<Media['state']>
  kind: 'image' | 'video'
  mediaType?: string
  download?: string
  /** Tries reading it again once it has all arrived. */
  onBack?: () => void
}) {
  const [arrived, setArrived] = useState<BlobState | null>(null)
  // Its caller makes a new one each time it draws; the polling mustn't start over for that.
  const back = useRef(onBack)
  useLayoutEffect(() => {
    back.current = onBack
  })
  useEffect(() => {
    let gone = false
    let timer: number | undefined
    const ask = () =>
      api.blobState(state.thread, state.blob).then(
        (answer) => {
          if (gone) return
          setArrived(answer)
          if (answer.stored) back.current?.()
          else if (!answer.refused) timer = window.setTimeout(ask, 3000)
        },
        () => !gone && setArrived({ stored: false, received: 0 }),
      )
    void ask()
    return () => {
      gone = true
      window.clearTimeout(timer)
    }
  }, [state.thread, state.blob])
  const box = 'flex max-w-xs flex-col items-center gap-2 text-center text-xs text-fg-2'
  if (!arrived)
    return (
      <span className={box}>
        <Spinner />
      </span>
    )
  const machine = arrived.from ?? 'the machine that made it'
  const Machine = arrived.from ?? 'The machine that made it'
  const progress = (
    <span className="h-1 w-44 overflow-hidden rounded-full bg-surface-3">
      <span
        className="block h-full bg-accent"
        style={{ width: `${Math.min(100, (arrived.received / Math.max(1, state.size)) * 100)}%` }}
      />
    </span>
  )
  if (arrived.stored && kind === 'video')
    return (
      <span className={box} data-testid="media-unplayable">
        <Icon name="alert" className="h-4 w-4" />
        <strong className="text-sm font-medium text-fg-1">This browser can’t play {mediaType ?? 'this video'}</strong>
        Download it to play elsewhere, or ask the agent for MP4 (H.264) or WebM.
        {download && (
          <a
            href={download}
            className="inline-flex h-7 items-center gap-1.5 rounded-sm border border-line bg-bg px-2 text-fg-1 hover:border-line-strong"
          >
            <Icon name="download" className="h-3.5 w-3.5" />
            Download · {sizeOf(state.size)}
          </a>
        )}
      </span>
    )
  if (arrived.refused)
    return (
      <span className={box} data-testid="media-refused">
        <Icon name="laptop" className="h-4 w-4" />
        <strong className="text-sm font-medium text-fg-1">Only on {machine}</strong>
        The server won’t keep it: {arrived.refused}.
      </span>
    )
  if (arrived.damaged)
    return (
      <span className={box} data-testid="media-damaged">
        <Icon name="alert" className="h-4 w-4" />
        <strong className="text-sm font-medium text-fg-1">The copy that arrived was damaged</strong>
        Silt checks every file against its fingerprint. {Machine} is sending it again.
        {arrived.received > 0 && progress}
      </span>
    )
  if (arrived.received > 0)
    return (
      <span className={box} data-testid="media-arriving">
        <Icon name="laptop" className="h-4 w-4" />
        <strong className="text-sm font-medium text-fg-1">Coming from {machine}</strong>
        {sizeOf(arrived.received)} of {sizeOf(state.size)}
        {progress}
      </span>
    )
  return (
    <span className={box} data-testid="media-waiting">
      <Icon name="laptop" className="h-4 w-4" />
      <strong className="text-sm font-medium text-fg-1">Not here yet</strong>
      {Machine} hasn’t uploaded this {kind}. It arrives once {machine} is online.
    </span>
  )
}

/**
 * Under a large image or video this machine keeps: that the server refused it, so it's only here (ADR 0121). Asks
 * only for a file large enough to have been refused.
 */
export function KeptHereOnly({ state }: { state: Media['state'] }) {
  const [refused, setRefused] = useState<string | null>(null)
  const [thread, blob, size] = [state?.thread, state?.blob, state?.size ?? 0]
  useEffect(() => {
    if (!thread || !blob || size <= CHUNK) return
    let gone = false
    api.blobState(thread, blob).then(
      (answer) => !gone && answer.stored && setRefused(answer.refused ?? null),
      () => {},
    )
    return () => {
      gone = true
    }
  }, [thread, blob, size])
  if (!refused) return null
  return (
    <p className="flex items-center gap-1.5 text-2xs text-warn-fg" data-testid="media-kept-here">
      <Icon name="laptop" className="h-3.5 w-3.5" />
      Only on this machine: the server won’t keep it ({refused}).
    </p>
  )
}

// ---------- The viewer ----------

/** An image or a video over the whole window; Esc closes it and gives focus back to what opened it. */
export function MediaViewer({ media, onClose, startAt }: { media: Media; onClose: () => void; startAt?: number }) {
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
    return () => opener?.focus()
  }, [])
  const title = (
    <strong className="truncate text-sm font-medium" id="media-viewer-title">
      {media.title}
    </strong>
  )
  const actions = (
    <>
      {media.download && (
        <a href={media.download} aria-label="download" title="Download" className={BAR_BUTTON}>
          <Icon name="download" className="h-4 w-4" />
        </a>
      )}
      <BarButton icon="x" label="Close (Esc)" onClick={onClose} />
    </>
  )
  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="media-viewer-title"
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.stopPropagation()
          onClose()
        }
      }}
      className="fixed inset-0 z-50 grid bg-bg text-fg-1 [padding:env(safe-area-inset-top)_env(safe-area-inset-right)_env(safe-area-inset-bottom)_env(safe-area-inset-left)]"
      data-testid="media-viewer"
    >
      {media.kind === 'image' ? (
        <ViewerImage media={media} title={title} actions={actions} />
      ) : (
        <div className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)]">
          <div className="flex min-w-0 items-center gap-1 border-b border-line-subtle px-2 md:h-12 md:pl-4">
            <div className="flex min-w-0 flex-1 flex-col py-1.5">
              {title}
              {media.meta && <span className="truncate text-2xs text-fg-3">{media.meta}</span>}
            </div>
            {actions}
          </div>
          <div className="min-h-0 bg-[#0d0b09]">
            <VideoPlayer media={media} variant="fill" startAt={startAt ?? 0} autoFocus />
          </div>
        </div>
      )}
    </div>,
    document.body,
  )
}

function ViewerImage({ media, title, actions }: { media: ImageMedia; title: ReactNode; actions: ReactNode }) {
  const holder = useRef<HTMLDivElement>(null)
  useEffect(() => holder.current?.querySelector<HTMLElement>('[data-testid="zoom-stage"]')?.focus(), [])
  return (
    <div ref={holder} className="grid min-h-0">
      <ZoomableImage media={media} layout="viewer" title={title} actions={actions} />
    </div>
  )
}
