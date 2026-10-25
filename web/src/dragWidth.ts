import { type KeyboardEvent, type PointerEvent, useState } from 'react'
import { store, stored } from './stored'

export type Bounds = { min: number; max: number; start: number }

/** A panel's width, kept under `key`, dragged or moved with the arrow keys on its edge.
 * `edge` is the side its handle is on: a panel on the right grows as its left edge moves left. */
export function useDragWidth(key: string, bounds: Bounds, edge: 'left' | 'right') {
  const clamp = (next: number) => Math.min(bounds.max, Math.max(bounds.min, Math.round(next)))
  const [width, setWidth] = useState(() => {
    const kept = stored<number>(key, bounds.start)
    return typeof kept === 'number' && kept >= bounds.min && kept <= bounds.max ? kept : bounds.start
  })
  const grows = edge === 'right' ? 1 : -1
  const set = (next: number) => {
    const clamped = clamp(next)
    setWidth(clamped)
    store(key, clamped)
  }
  const drag = (event: PointerEvent<HTMLDivElement>) => {
    const start = { x: event.clientX, width }
    const handle = event.currentTarget
    handle.setPointerCapture(event.pointerId)
    const move = (moved: globalThis.PointerEvent) => set(start.width + grows * (moved.clientX - start.x))
    const stop = () => {
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('pointerup', stop)
    }
    handle.addEventListener('pointermove', move)
    handle.addEventListener('pointerup', stop)
  }
  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = { ArrowLeft: -16, ArrowRight: 16 }[event.key]
    if (step) {
      event.preventDefault()
      set(width + grows * step)
    }
  }
  return {
    width,
    handle: {
      role: 'separator',
      'aria-orientation': 'vertical',
      'aria-valuenow': width,
      'aria-valuemin': bounds.min,
      'aria-valuemax': bounds.max,
      tabIndex: 0,
      onPointerDown: drag,
      onKeyDown: onKey,
    } as const,
  }
}
