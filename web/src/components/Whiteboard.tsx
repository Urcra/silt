// A whiteboard (ADR 0081): items in the thread's page data, one document each under `items/`, drawn by Silt
// with no agent code. People pan, zoom and drag; a drag is written when it ends, and cursors go through the room.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { api } from '../api'
import { useTopic } from '../bus'
import { message } from '../query'
import { RoomSocket, type PeerPresence } from '../rooms'
import { Button, IconButton, toast } from '../ui'

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
}

const COLORS: Record<string, string> = { yellow: '#f6e7a1', blue: '#c9dcf2', green: '#cfe8d2', pink: '#f3cfd8', gray: '#e4e2dc' }

/** Where an item's middle is, for arrows to meet. */
function middle(item: Item | undefined): [number, number] | null {
  if (!item || item.x === undefined || item.y === undefined) return null
  return [item.x + (item.w ?? 160) / 2, item.y + (item.h ?? 100) / 2]
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
  const [moving, setMoving] = useState<{ id: string; dx: number; dy: number } | null>(null)
  const [peers, setPeers] = useState<PeerPresence[]>([])
  const room = useRef<RoomSocket | null>(null)
  const drag = useRef<{ kind: 'pan'; x: number; y: number; from: typeof view } | { kind: 'item'; id: string; x: number; y: number } | null>(null)
  const box = useRef<HTMLDivElement>(null)
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
  const byId = useMemo(() => new Map(items.map((item) => [item.id, item])), [items])
  const at = (event: { clientX: number; clientY: number }) => {
    const rect = box.current?.getBoundingClientRect()
    return [(event.clientX - (rect?.left ?? 0) - view.x) / view.scale, (event.clientY - (rect?.top ?? 0) - view.y) / view.scale]
  }
  const write = async (item: Item) => {
    try {
      await api.pageData(thread, artifact, { op: 'set', path: `items/${item.id}`, value: item })
      await queries.invalidateQueries({ queryKey: key })
    } catch (failure) {
      toast(message(failure), 'danger')
    }
  }
  const down = (event: ReactPointerEvent, id?: string) => {
    event.stopPropagation()
    ;(event.currentTarget as Element).setPointerCapture?.(event.pointerId)
    const [x, y] = at(event)
    drag.current = id ? { kind: 'item', id, x, y } : { kind: 'pan', x: event.clientX, y: event.clientY, from: view }
  }
  const move = (event: ReactPointerEvent) => {
    const [x, y] = at(event)
    room.current?.presence({ cursor: [Math.round(x), Math.round(y)], dragging: moving?.id ?? null })
    const now = drag.current
    if (!now) return
    if (now.kind === 'pan') setView({ ...now.from, x: now.from.x + event.clientX - now.x, y: now.from.y + event.clientY - now.y })
    else setMoving({ id: now.id, dx: x - now.x, dy: y - now.y })
  }
  const up = () => {
    const now = drag.current
    drag.current = null
    if (now?.kind === 'item' && moving) {
      const item = byId.get(now.id)
      if (item && (moving.dx !== 0 || moving.dy !== 0)) void write({ ...item, x: Math.round((item.x ?? 0) + moving.dx), y: Math.round((item.y ?? 0) + moving.dy) })
    }
    setMoving(null)
  }
  const placed = (item: Item) => (moving?.id === item.id ? { ...item, x: (item.x ?? 0) + moving.dx, y: (item.y ?? 0) + moving.dy } : item)
  const addNote = () => {
    const id = `n${Date.now().toString(36)}`
    void write({ id, type: 'block', x: Math.round((-view.x + 80) / view.scale), y: Math.round((-view.y + 80) / view.scale), w: 160, h: 100, text: 'New note', color: 'yellow' })
  }
  if (listed.error) return <p className="px-5 py-6 text-sm text-danger">{message(listed.error)}</p>
  return (
    <div className="relative h-full min-h-0 overflow-hidden bg-surface-1" data-testid="whiteboard">
      <div
        ref={box}
        className="absolute inset-0 touch-none"
        onPointerDown={(event) => down(event)}
        onPointerMove={move}
        onPointerUp={up}
        onWheel={(event) => setView((was) => ({ ...was, scale: Math.min(Math.max(was.scale * Math.exp(-event.deltaY / 500), 0.1), 3) }))}
      >
        <svg className="absolute inset-0 h-full w-full" role="img" aria-label={title}>
          <defs>
            <marker id="whiteboard-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto">
              <path d="M0,0 L10,5 L0,10 z" fill="currentColor" />
            </marker>
          </defs>
          <g transform={`translate(${view.x} ${view.y}) scale(${view.scale})`} className="text-fg-2">
            {items.filter((item) => item.type === 'section').map((raw) => {
              const item = placed(raw)
              return (
                <g key={item.id} onPointerDown={(event) => down(event, item.id)} data-item={item.id}>
                  <rect x={item.x} y={item.y} width={item.w ?? 400} height={item.h ?? 300} rx={12} fill="none" stroke="currentColor" strokeDasharray="6 6" />
                  <text x={(item.x ?? 0) + 12} y={(item.y ?? 0) + 24} fontSize={16} fill="currentColor">
                    {item.text}
                  </text>
                </g>
              )
            })}
            {items.filter((item) => item.type === 'arrow').map((item) => {
              const [from, to] = [middle(placed(byId.get(item.from ?? '') ?? ({} as Item))), middle(placed(byId.get(item.to ?? '') ?? ({} as Item)))]
              if (!from || !to) return null
              return (
                <g key={item.id}>
                  <line x1={from[0]} y1={from[1]} x2={to[0]} y2={to[1]} stroke="currentColor" strokeWidth={2} markerEnd="url(#whiteboard-arrow)" />
                  {item.label && (
                    <text x={(from[0] + to[0]) / 2} y={(from[1] + to[1]) / 2 - 6} fontSize={13} textAnchor="middle" fill="currentColor">
                      {item.label}
                    </text>
                  )}
                </g>
              )
            })}
            {items.filter((item) => item.type === 'stroke').map((item) => (
              <polyline key={item.id} points={(item.points ?? []).map(([x, y]) => `${x},${y}`).join(' ')} fill="none" stroke="currentColor" strokeWidth={3} strokeLinecap="round" />
            ))}
            {items.filter((item) => item.type === 'block').map((raw) => {
              const item = placed(raw)
              const [w, h] = [item.w ?? 160, item.h ?? 100]
              return (
                <g key={item.id} onPointerDown={(event) => down(event, item.id)} className="cursor-grab" data-item={item.id} data-testid="whiteboard-block">
                  <rect x={item.x} y={item.y} width={w} height={h} rx={6} fill={COLORS[item.color ?? 'yellow'] ?? COLORS.yellow} stroke="rgba(0,0,0,0.15)" />
                  <foreignObject x={item.x} y={item.y} width={w} height={h}>
                    <div className="h-full w-full overflow-hidden p-2 text-[13px] leading-snug text-[#1d1d1f]">{item.text}</div>
                  </foreignObject>
                </g>
              )
            })}
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
      </div>
      <div className="absolute top-2 left-2 flex items-center gap-1 rounded-md border border-line bg-elevated p-1 shadow-overlay">
        <Button size="sm" onClick={addNote}>
          Add a note
        </Button>
        <span className="px-1 text-xs text-fg-2">{peers.length > 0 ? `${peers.length + 1} here` : 'Only you'}</span>
      </div>
      <div className="absolute right-2 bottom-2 flex items-center gap-1 rounded-md border border-line bg-elevated p-1 shadow-overlay">
        <IconButton icon="plus" label="Zoom in" onClick={() => setView((was) => ({ ...was, scale: Math.min(was.scale * 1.25, 3) }))} />
        <IconButton icon="x" label="Zoom out" onClick={() => setView((was) => ({ ...was, scale: Math.max(was.scale / 1.25, 0.1) }))} />
      </div>
    </div>
  )
}
