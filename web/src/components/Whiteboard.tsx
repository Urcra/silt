// A whiteboard (ADR 0081): items in the thread's page data, one document each under `items/`, drawn by Silt
// with no agent code. People pan, zoom, drag, resize, draw, connect and add images; a change is written when it
// ends, and cursors go through the room.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { api } from '../api'
import { edgeOf } from '../boardGeometry'
import { useTopic } from '../bus'
import { message } from '../query'
import { RoomSocket, type PeerPresence } from '../rooms'
import { Button, IconButton, cx, toast } from '../ui'

type Item = {
  id: string
  type: 'block' | 'arrow' | 'section' | 'stroke' | 'image'
  x?: number
  y?: number
  w?: number
  h?: number
  text?: string
  color?: string
  from?: string
  to?: string
  label?: string
  points?: [number, number][]
  /** An image's asset, added to the page (ADR 0074). */
  asset?: string
}

type Tool = 'move' | 'draw' | 'connect'

const COLORS: Record<string, string> = { yellow: '#f6e7a1', blue: '#c9dcf2', green: '#cfe8d2', pink: '#f3cfd8', gray: '#e4e2dc' }
/** The images a whiteboard draws; an SVG could carry what a board shouldn't run. */
const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']
/** The most points one stroke keeps. */
const STROKE_POINTS = 500
const MIN_SIZE = 40

/** How big an item is drawn when it doesn't say. */
function sizeOf(item: Item): [number, number] {
  if (item.type === 'section') return [item.w ?? 400, item.h ?? 300]
  if (item.type === 'image') return [item.w ?? 240, item.h ?? 180]
  return [item.w ?? 160, item.h ?? 100]
}

/** An item's box, for arrows to meet. */
function boxOf(item: Item | undefined): { x: number; y: number; w: number; h: number } | null {
  if (!item || item.x === undefined || item.y === undefined) return null
  const [w, h] = sizeOf(item)
  return { x: item.x, y: item.y, w, h }
}

/** An arrow's two ends, each at the edge of its item facing the other. */
function ends(from: Item | undefined, to: Item | undefined): [[number, number], [number, number]] | null {
  const [a, b] = [boxOf(from), boxOf(to)]
  if (!a || !b) return null
  const [middleA, middleB]: [number, number][] = [
    [a.x + a.w / 2, a.y + a.h / 2],
    [b.x + b.w / 2, b.y + b.h / 2],
  ]
  return [edgeOf(a, middleB, 2), edgeOf(b, middleA)]
}

/** An id for a new item, unlikely to meet another person's made at once. */
function newId(prefix: string): string {
  return `${prefix}${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`
}

function base64Of(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result).split(',')[1] ?? '')
    reader.onerror = () => reject(reader.error ?? new Error('the file could not be read'))
    reader.readAsDataURL(file)
  })
}

/** An image on the board, read from the page's assets. */
function BoardImage({ thread, artifact, item, onDown }: { thread: string; artifact: string; item: Item; onDown: (event: ReactPointerEvent) => void }) {
  const read = useQuery({
    queryKey: ['page-asset', thread, artifact, item.asset],
    queryFn: () => api.pageAsset(thread, artifact, item.asset ?? ''),
    enabled: Boolean(item.asset),
    staleTime: Infinity,
  })
  const [w, h] = sizeOf(item)
  const shown = read.data && IMAGE_TYPES.includes(read.data.media_type) ? `data:${read.data.media_type};base64,${read.data.data}` : null
  return (
    <g onPointerDown={onDown} className="cursor-grab" data-item={item.id} data-testid="whiteboard-image">
      {shown ? (
        <image href={shown} x={item.x} y={item.y} width={w} height={h} preserveAspectRatio="xMidYMid meet" />
      ) : (
        <rect x={item.x} y={item.y} width={w} height={h} rx={6} fill="none" stroke="currentColor" strokeDasharray="4 4" />
      )}
    </g>
  )
}

/** `html`, a note's text on screen, as SVG text wrapped to its width, for an exported board. */
function textInstead(html: SVGForeignObjectElement) {
  const svg = 'http://www.w3.org/2000/svg'
  const [x, y, width, height] = ['x', 'y', 'width', 'height'].map((name) => Number(html.getAttribute(name) ?? 0))
  const size = 13
  const most = Math.max(4, Math.floor((width - 16) / (size * 0.55)))
  const lines: string[] = []
  for (const paragraph of (html.textContent ?? '').split('\n')) {
    let line = ''
    for (const word of paragraph.split(/\s+/)) {
      if (line && (line + ' ' + word).length > most) {
        lines.push(line)
        line = word
      } else line = line ? `${line} ${word}` : word
    }
    lines.push(line)
  }
  const fits = Math.max(1, Math.floor((height - 12) / (size * 1.3)))
  const text = document.createElementNS(svg, 'text')
  text.setAttribute('font-family', 'system-ui, sans-serif')
  text.setAttribute('font-size', String(size))
  text.setAttribute('fill', '#1d1d1f')
  lines.slice(0, fits).forEach((shown, at) => {
    const span = document.createElementNS(svg, 'tspan')
    span.setAttribute('x', String(x + 8))
    span.setAttribute('y', String(y + 8 + size + at * size * 1.3))
    span.textContent = shown
    text.appendChild(span)
  })
  html.replaceWith(text)
}

/** What's on the board, as left, top, width and height with a margin; none when it's empty. */
function boundsOf(items: Item[]): [number, number, number, number] | null {
  const boxes = items
    .filter((item) => item.x !== undefined && item.y !== undefined)
    .map((item) => [item.x ?? 0, item.y ?? 0, ...sizeOf(item)] as const)
  const strokes = items.flatMap((item) => (item.type === 'stroke' ? (item.points ?? []) : []))
  const xs = [...boxes.flatMap(([x, , w]) => [x, x + w]), ...strokes.map(([x]) => x)]
  const ys = [...boxes.flatMap(([, y, , h]) => [y, y + h]), ...strokes.map(([, y]) => y)]
  if (xs.length === 0 || ys.length === 0) return null
  const [left, top] = [Math.min(...xs) - 24, Math.min(...ys) - 24]
  return [left, top, Math.max(...xs) - left + 24, Math.max(...ys) - top + 24]
}

export function WhiteboardView({ thread, artifact, title }: { thread: string; artifact: string; title: string }) {
  useTopic(`thread:${thread}`)
  const queries = useQueryClient()
  const key = ['whiteboard', thread, artifact]
  const listed = useQuery({
    queryKey: key,
    queryFn: () => api.pageData(thread, artifact, { op: 'list', collection: 'items' }) as Promise<{ id: string; data: Item }[]>,
    refetchInterval: 4_000,
  })
  // Room above the board for its toolbar, so nothing starts under it.
  const [view, setView] = useState({ x: 48, y: 96, scale: 0.8 })
  const [tool, setTool] = useState<Tool>('move')
  const [selected, setSelected] = useState<string | null>(null)
  // The first end of an arrow being connected.
  const [connecting, setConnecting] = useState<string | null>(null)
  const [moving, setMoving] = useState<{ id: string; dx: number; dy: number } | null>(null)
  const [resizing, setResizing] = useState<{ id: string; w: number; h: number } | null>(null)
  const [stroke, setStroke] = useState<[number, number][] | null>(null)
  const [editing, setEditing] = useState<{ id: string; text: string } | null>(null)
  const [peers, setPeers] = useState<PeerPresence[]>([])
  const room = useRef<RoomSocket | null>(null)
  const drag = useRef<
    | { kind: 'pan'; x: number; y: number; from: typeof view }
    | { kind: 'item'; id: string; x: number; y: number }
    | { kind: 'resize'; id: string; x: number; y: number; w: number; h: number }
    | { kind: 'draw' }
    | null
  >(null)
  const box = useRef<HTMLDivElement>(null)
  const picture = useRef<SVGSVGElement>(null)
  const upload = useRef<HTMLInputElement>(null)
  useEffect(() => {
    const socket = new RoomSocket(thread, artifact)
    room.current = socket
    const off = socket.listen((heard) => {
      if (heard.event === 'peers') setPeers(heard.peers.filter((peer) => peer.peer !== socket.me))
    })
    return () => {
      off()
      socket.close()
    }
  }, [thread, artifact])
  const items = useMemo(() => (listed.data ?? []).map((doc) => doc.data).filter((item) => item && typeof item.id === 'string'), [listed.data])
  // The board is framed in the panel, as a canvas fits its boards, and again when the panel changes size, such as going
  // to the full view, until the person pans or zooms.
  const moved = useRef(false)
  const shownItems = useRef(items)
  useEffect(() => {
    shownItems.current = items
  })
  const fit = useRef(() => {
    const bounds = boundsOf(shownItems.current)
    const shown = box.current?.getBoundingClientRect()
    if (moved.current || !bounds || !shown || shown.width === 0) return
    const [left, top, width, height] = bounds
    const scale = Math.max(0.2, Math.min(1, (shown.width - 48) / width, (shown.height - 120) / height))
    setView({ x: Math.max(24, (shown.width - width * scale) / 2) - left * scale, y: 96 - top * scale, scale })
  })
  const zoom = (by: number) => {
    moved.current = true
    setView((was) => ({ ...was, scale: Math.min(Math.max(was.scale * by, 0.1), 3) }))
  }
  const fittedOnce = useRef(false)
  useEffect(() => {
    if (fittedOnce.current || items.length === 0) return
    fittedOnce.current = true
    const frame = requestAnimationFrame(() => fit.current())
    return () => cancelAnimationFrame(frame)
  }, [items])
  useEffect(() => {
    const shown = box.current
    if (!shown) return
    const observer = new ResizeObserver(() => fit.current())
    observer.observe(shown)
    return () => observer.disconnect()
  }, [])
  const byId = useMemo(() => new Map(items.map((item) => [item.id, item])), [items])
  const at = (event: { clientX: number; clientY: number }): [number, number] => {
    const rect = box.current?.getBoundingClientRect()
    return [(event.clientX - (rect?.left ?? 0) - view.x) / view.scale, (event.clientY - (rect?.top ?? 0) - view.y) / view.scale]
  }
  const refresh = () => queries.invalidateQueries({ queryKey: key })
  const write = async (item: Item) => {
    try {
      await api.pageData(thread, artifact, { op: 'set', path: `items/${item.id}`, value: item })
      await refresh()
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  // Only the fields changed, so what someone else changed meanwhile, the agent's text say, stays.
  const patch = async (id: string, fields: Partial<Item>) => {
    try {
      await api.pageData(thread, artifact, { op: 'update', path: `items/${id}`, fields })
      await refresh()
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const remove = async (id: string) => {
    // An arrow to or from it goes with it.
    const gone = [id, ...items.filter((item) => item.type === 'arrow' && (item.from === id || item.to === id)).map((item) => item.id)]
    try {
      await api.pageData(thread, artifact, { op: 'batch', writes: gone.map((each) => ({ op: 'delete', path: `items/${each}` })) })
      setSelected(null)
      await refresh()
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  useEffect(() => {
    if (!selected || editing) return
    const pressed = (event: KeyboardEvent) => {
      const typing = (event.target as HTMLElement | null)?.closest('input, textarea, [contenteditable]')
      if (!typing && (event.key === 'Delete' || event.key === 'Backspace')) void remove(selected)
    }
    window.addEventListener('keydown', pressed)
    return () => window.removeEventListener('keydown', pressed)
  })
  const down = (event: ReactPointerEvent, id?: string) => {
    event.stopPropagation()
    ;(event.currentTarget as Element).setPointerCapture?.(event.pointerId)
    const [x, y] = at(event)
    if (tool === 'draw' && !id) {
      drag.current = { kind: 'draw' }
      setStroke([[Math.round(x), Math.round(y)]])
      return
    }
    if (tool === 'connect' && id) {
      const item = byId.get(id)
      if (!item || item.type === 'arrow' || item.type === 'stroke') return
      if (!connecting) setConnecting(id)
      else if (connecting !== id) {
        void write({ id: newId('a'), type: 'arrow', from: connecting, to: id })
        setConnecting(null)
      }
      return
    }
    setSelected(id ?? null)
    drag.current = id ? { kind: 'item', id, x, y } : { kind: 'pan', x: event.clientX, y: event.clientY, from: view }
  }
  const startResize = (event: ReactPointerEvent, item: Item) => {
    event.stopPropagation()
    ;(event.currentTarget as Element).setPointerCapture?.(event.pointerId)
    const [x, y] = at(event)
    const [w, h] = sizeOf(item)
    drag.current = { kind: 'resize', id: item.id, x, y, w, h }
  }
  const move = (event: ReactPointerEvent) => {
    const [x, y] = at(event)
    room.current?.presence({ cursor: [Math.round(x), Math.round(y)], dragging: moving?.id ?? null })
    const now = drag.current
    if (!now) return
    if (now.kind === 'pan') {
      moved.current = true
      setView({ ...now.from, x: now.from.x + event.clientX - now.x, y: now.from.y + event.clientY - now.y })
    }
    else if (now.kind === 'item') setMoving({ id: now.id, dx: x - now.x, dy: y - now.y })
    else if (now.kind === 'resize') setResizing({ id: now.id, w: Math.max(MIN_SIZE, now.w + x - now.x), h: Math.max(MIN_SIZE, now.h + y - now.y) })
    else
      setStroke((points) => {
        if (!points || points.length >= STROKE_POINTS) return points
        const [lastX, lastY] = points[points.length - 1]
        return Math.hypot(x - lastX, y - lastY) < 3 ? points : [...points, [Math.round(x), Math.round(y)]]
      })
  }
  const up = () => {
    const now = drag.current
    drag.current = null
    if (now?.kind === 'item' && moving) {
      const item = byId.get(now.id)
      if (item && (moving.dx !== 0 || moving.dy !== 0)) void patch(item.id, { x: Math.round((item.x ?? 0) + moving.dx), y: Math.round((item.y ?? 0) + moving.dy) })
    }
    if (now?.kind === 'resize' && resizing) {
      const item = byId.get(now.id)
      if (item) void patch(item.id, { w: Math.round(resizing.w), h: Math.round(resizing.h) })
    }
    if (now?.kind === 'draw' && stroke && stroke.length > 1) void write({ id: newId('s'), type: 'stroke', points: stroke })
    setMoving(null)
    setResizing(null)
    setStroke(null)
  }
  const placed = (item: Item): Item => {
    let shown = item
    if (moving?.id === item.id) shown = { ...shown, x: (item.x ?? 0) + moving.dx, y: (item.y ?? 0) + moving.dy }
    if (resizing?.id === item.id) shown = { ...shown, w: resizing.w, h: resizing.h }
    return shown
  }
  const visibleCorner = (): [number, number] => [Math.round((-view.x + 80) / view.scale), Math.round((-view.y + 80) / view.scale)]
  const addNote = () => {
    const [x, y] = visibleCorner()
    void write({ id: newId('n'), type: 'block', x, y, w: 160, h: 100, text: 'New note', color: 'yellow' })
  }
  const addImage = async (file: File) => {
    if (!IMAGE_TYPES.includes(file.type)) return toast('Add a PNG, JPEG, GIF or WebP image', 'danger')
    try {
      const added = (await api.pageAssets(thread, artifact, { op: 'add', name: file.name, media_type: file.type, data: await base64Of(file) })) as { id: string }
      const size = await new Promise<[number, number]>((resolve) => {
        const image = new Image()
        image.onload = () => resolve([image.naturalWidth || 240, image.naturalHeight || 180])
        image.onerror = () => resolve([240, 180])
        image.src = URL.createObjectURL(file)
      })
      const scale = Math.min(1, 320 / Math.max(size[0], size[1]))
      const [x, y] = visibleCorner()
      await write({ id: newId('i'), type: 'image', asset: added.id, x, y, w: Math.round(size[0] * scale), h: Math.round(size[1] * scale) })
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const finishEditing = () => {
    if (!editing) return
    const item = byId.get(editing.id)
    if (item && item.text !== editing.text) void patch(item.id, { text: editing.text.slice(0, 2000) })
    setEditing(null)
  }
  /** The board as an SVG document: what's on it, framed, without anyone's cursor. */
  const boardSvg = () => {
    const svg = picture.current
    const bounds = boundsOf(items)
    if (!svg || !bounds) return null
    const [left, top, width, height] = bounds
    const copy = svg.cloneNode(true) as SVGSVGElement
    copy.querySelectorAll('[data-testid="whiteboard-cursor"], [data-handle]').forEach((node) => node.remove())
    // A note's text is HTML on screen, which neither a file nor a canvas draws: here it's SVG text, wrapped to the note.
    copy.querySelectorAll('foreignObject').forEach((html) => textInstead(html as SVGForeignObjectElement))
    copy.querySelector('g[data-board]')?.removeAttribute('transform')
    copy.setAttribute('xmlns', 'http://www.w3.org/2000/svg')
    copy.setAttribute('viewBox', `${left} ${top} ${width} ${height}`)
    copy.setAttribute('width', String(Math.round(width)))
    copy.setAttribute('height', String(Math.round(height)))
    copy.setAttribute('color', '#4a4a48')
    copy.removeAttribute('class')
    return { text: `<?xml version="1.0" encoding="UTF-8"?>\n${new XMLSerializer().serializeToString(copy)}`, width, height }
  }
  const save = (blob: Blob, extension: string) => {
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `${title.replace(/[^A-Za-z0-9_-]+/g, '-') || 'whiteboard'}.${extension}`
    anchor.click()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
  }
  const exportSvg = () => {
    const board = boardSvg()
    if (board) save(new Blob([board.text], { type: 'image/svg+xml' }), 'svg')
  }
  /** The board as a PNG at twice its size, on white, drawn from its SVG in this browser. */
  const exportPng = () => {
    const board = boardSvg()
    if (!board) return
    const scale = Math.min(2, 8192 / Math.max(board.width, board.height))
    const image = new Image()
    const url = URL.createObjectURL(new Blob([board.text], { type: 'image/svg+xml' }))
    image.onload = () => {
      const canvas = document.createElement('canvas')
      canvas.width = Math.round(board.width * scale)
      canvas.height = Math.round(board.height * scale)
      const context = canvas.getContext('2d')
      if (context) {
        context.fillStyle = '#ffffff'
        context.fillRect(0, 0, canvas.width, canvas.height)
        context.drawImage(image, 0, 0, canvas.width, canvas.height)
        canvas.toBlob((png) => png && save(png, 'png'), 'image/png')
      }
      URL.revokeObjectURL(url)
    }
    image.onerror = () => {
      URL.revokeObjectURL(url)
      toast("This board couldn't be drawn as a PNG", 'danger')
    }
    image.src = url
  }
  const handle = (item: Item) => {
    if (selected !== item.id || tool !== 'move') return null
    const shown = placed(item)
    const [w, h] = sizeOf(shown)
    return (
      <rect
        data-handle
        aria-label="resize"
        x={(shown.x ?? 0) + w - 6}
        y={(shown.y ?? 0) + h - 6}
        width={12}
        height={12}
        rx={2}
        className="cursor-nwse-resize"
        fill="var(--color-accent, #b86a2b)"
        onPointerDown={(event) => startResize(event, item)}
        data-testid="whiteboard-resize"
      />
    )
  }
  const marked = (id: string) => (selected === id && tool === 'move') || connecting === id
  const editingItem = editing ? byId.get(editing.id) : undefined
  if (listed.error) return <p className="px-5 py-6 text-sm text-danger">{message(listed.error)}</p>
  return (
    <div className="relative h-full min-h-0 overflow-hidden bg-surface-1" data-testid="whiteboard">
      <div
        ref={box}
        className={cx('absolute inset-0 touch-none', tool === 'draw' && 'cursor-crosshair')}
        onPointerDown={(event) => down(event)}
        onPointerMove={move}
        onPointerUp={up}
        onWheel={(event) => {
          moved.current = true
          setView((was) => ({ ...was, scale: Math.min(Math.max(was.scale * Math.exp(-event.deltaY / 500), 0.1), 3) }))
        }}
      >
        <svg ref={picture} className="absolute inset-0 h-full w-full" role="img" aria-label={title}>
          <defs>
            <marker id="whiteboard-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto">
              <path d="M0,0 L10,5 L0,10 z" fill="currentColor" />
            </marker>
          </defs>
          <g transform={`translate(${view.x} ${view.y}) scale(${view.scale})`} className="text-fg-2" data-board>
            {items.filter((item) => item.type === 'section').map((raw) => {
              const item = placed(raw)
              const [w, h] = sizeOf(item)
              return (
                <g key={item.id} onPointerDown={(event) => down(event, item.id)} data-item={item.id}>
                  <rect x={item.x} y={item.y} width={w} height={h} rx={12} fill="none" stroke={marked(item.id) ? 'var(--color-accent, #b86a2b)' : 'currentColor'} strokeDasharray="6 6" />
                  <text x={(item.x ?? 0) + 12} y={(item.y ?? 0) + 24} fontSize={16} fill="currentColor">
                    {item.text}
                  </text>
                </g>
              )
            })}
            {items.filter((item) => item.type === 'image').map((raw) => (
              <BoardImage key={raw.id} thread={thread} artifact={artifact} item={placed(raw)} onDown={(event) => down(event, raw.id)} />
            ))}
            {items.filter((item) => item.type === 'arrow').map((item) => {
              const joined = ends(placed(byId.get(item.from ?? '') ?? ({} as Item)), placed(byId.get(item.to ?? '') ?? ({} as Item)))
              if (!joined) return null
              const [from, to] = joined
              return (
                <g key={item.id} onPointerDown={(event) => down(event, item.id)} data-testid="whiteboard-arrow">
                  {/* A wider line no one sees, so an arrow is easy to pick. */}
                  <line x1={from[0]} y1={from[1]} x2={to[0]} y2={to[1]} stroke="transparent" strokeWidth={12} />
                  <line x1={from[0]} y1={from[1]} x2={to[0]} y2={to[1]} stroke={marked(item.id) ? 'var(--color-accent, #b86a2b)' : 'currentColor'} strokeWidth={2} markerEnd="url(#whiteboard-arrow)" />
                  {item.label && (
                    <text x={(from[0] + to[0]) / 2} y={(from[1] + to[1]) / 2 - 6} fontSize={13} textAnchor="middle" fill="currentColor">
                      {item.label}
                    </text>
                  )}
                </g>
              )
            })}
            {items.filter((item) => item.type === 'stroke').map((item) => (
              <polyline
                key={item.id}
                points={(item.points ?? []).map(([x, y]) => `${x},${y}`).join(' ')}
                fill="none"
                stroke={marked(item.id) ? 'var(--color-accent, #b86a2b)' : 'currentColor'}
                strokeWidth={3}
                strokeLinecap="round"
                strokeLinejoin="round"
                onPointerDown={(event) => down(event, item.id)}
                data-testid="whiteboard-stroke"
              />
            ))}
            {stroke && <polyline points={stroke.map(([x, y]) => `${x},${y}`).join(' ')} fill="none" stroke="currentColor" strokeWidth={3} strokeLinecap="round" strokeLinejoin="round" />}
            {items.filter((item) => item.type === 'block').map((raw) => {
              const item = placed(raw)
              const [w, h] = sizeOf(item)
              return (
                <g
                  key={item.id}
                  onPointerDown={(event) => down(event, item.id)}
                  onDoubleClick={() => setEditing({ id: item.id, text: item.text ?? '' })}
                  className="cursor-grab"
                  data-item={item.id}
                  data-testid="whiteboard-block"
                >
                  <rect
                    x={item.x}
                    y={item.y}
                    width={w}
                    height={h}
                    rx={6}
                    fill={COLORS[item.color ?? 'yellow'] ?? COLORS.yellow}
                    stroke={marked(item.id) ? 'var(--color-accent, #b86a2b)' : 'rgba(0,0,0,0.15)'}
                    strokeWidth={marked(item.id) ? 2 : 1}
                  />
                  <foreignObject x={item.x} y={item.y} width={w} height={h}>
                    <div className="h-full w-full overflow-hidden p-2 text-[13px] leading-snug whitespace-pre-wrap text-[#1d1d1f]">{item.text}</div>
                  </foreignObject>
                </g>
              )
            })}
            {items.filter((item) => item.type === 'block' || item.type === 'section' || item.type === 'image').map((item) => (
              <g key={`handle-${item.id}`}>{handle(item)}</g>
            ))}
            {peers.map((peer) => {
              const cursor = (peer.state as { cursor?: [number, number] } | null)?.cursor
              if (!cursor) return null
              return (
                <g key={peer.peer} transform={`translate(${cursor[0]} ${cursor[1]})`} data-testid="whiteboard-cursor" pointerEvents="none">
                  <path d="M0,0 L0,16 L4,12 L8,20 L11,19 L7,11 L13,11 z" fill="var(--color-accent, #b86a2b)" />
                  <text x={14} y={22} fontSize={12} fill="currentColor">
                    {peer.kind === 'agent' ? 'agent' : peer.user.slice(0, 6)}
                  </text>
                </g>
              )
            })}
          </g>
        </svg>
        {editing && editingItem && (
          <textarea
            aria-label="the note's text"
            autoFocus
            value={editing.text}
            onChange={(event) => setEditing({ ...editing, text: event.target.value })}
            onPointerDown={(event) => event.stopPropagation()}
            onBlur={finishEditing}
            onKeyDown={(event) => {
              if (event.key === 'Escape') setEditing(null)
              if (event.key === 'Enter' && !event.shiftKey) {
                event.preventDefault()
                finishEditing()
              }
            }}
            className="absolute resize-none rounded-md border border-accent bg-[#fffdf3] p-2 text-[13px] leading-snug text-[#1d1d1f] outline-none"
            style={{
              left: view.x + (editingItem.x ?? 0) * view.scale,
              top: view.y + (editingItem.y ?? 0) * view.scale,
              width: sizeOf(editingItem)[0] * view.scale,
              height: sizeOf(editingItem)[1] * view.scale,
            }}
          />
        )}
      </div>
      <div className="absolute top-2 left-2 flex flex-wrap items-center gap-1 rounded-md border border-line bg-elevated p-1 shadow-overlay">
        <Button size="sm" onClick={addNote}>
          Add a note
        </Button>
        <Button size="sm" variant="ghost" onClick={() => upload.current?.click()}>
          Add an image
        </Button>
        <input
          ref={upload}
          type="file"
          accept={IMAGE_TYPES.join(',')}
          className="hidden"
          aria-label="an image to add"
          onChange={(event) => {
            const file = event.target.files?.[0]
            event.target.value = ''
            if (file) void addImage(file)
          }}
        />
        <span className="mx-1 h-5 w-px bg-line" aria-hidden="true" />
        {(['move', 'draw', 'connect'] as const).map((each) => (
          <Button
            key={each}
            size="sm"
            variant={tool === each ? 'secondary' : 'ghost'}
            aria-pressed={tool === each}
            onClick={() => {
              setTool(each)
              setConnecting(null)
            }}
          >
            {each === 'move' ? 'Move' : each === 'draw' ? 'Draw' : 'Connect'}
          </Button>
        ))}
        {selected && tool === 'move' && (
          <Button size="sm" variant="ghost" onClick={() => void remove(selected)}>
            Delete
          </Button>
        )}
        <span className="px-1 text-xs text-fg-2">
          {tool === 'connect' ? (connecting ? 'Now pick where it goes' : 'Pick where an arrow starts') : peers.length > 0 ? `${peers.length + 1} here` : 'Only you'}
        </span>
      </div>
      <div className="absolute right-2 bottom-2 flex items-center gap-1 rounded-md border border-line bg-elevated p-1 shadow-overlay">
        <IconButton icon="download" label="Download as SVG" onClick={exportSvg} />
        <IconButton icon="image" label="Download as PNG" onClick={exportPng} />
        <IconButton icon="plus" label="Zoom in" onClick={() => zoom(1.25)} />
        <IconButton icon="x" label="Zoom out" onClick={() => zoom(1 / 1.25)} />
      </div>
    </div>
  )
}
