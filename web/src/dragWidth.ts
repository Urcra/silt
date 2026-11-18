import { type CSSProperties, type KeyboardEvent, type PointerEvent, type RefObject, useState } from 'react'
import { store, stored } from './stored'

export type Bounds = { min: number; max: number; start: number }

/** A panel's width, kept under `key`, dragged or moved with the arrow keys on its edge, as the CSS variable `name`
 * on `panel`. `edge` is the side its handle is on: a panel on the right grows as its left edge moves
 * left. While dragged, the width goes straight to the element and is kept once let go: rendering the page again on
 * each move, and the diff or list beside it, made dragging stutter. */
export function useDragWidth(
  key: string,
  bounds: Bounds,
  edge: 'left' | 'right',
  name: `--${string}`,
  panel: RefObject<HTMLElement | null>,
) {
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
    let shown = width
    const handle = event.currentTarget
    // Else the press starts a selection, and moving it drags a link under it, which cancels the pointer.
    event.preventDefault()
    handle.focus({ preventScroll: true })
    handle.setPointerCapture(event.pointerId)
    const move = (moved: globalThis.PointerEvent) => {
      shown = clamp(start.width + grows * (moved.clientX - start.x))
      panel.current?.style.setProperty(name, `${shown}px`)
      handle.setAttribute('aria-valuenow', String(shown))
    }
    const stop = () => {
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('lostpointercapture', stop)
      if (shown !== width) set(shown)
    }
    handle.addEventListener('pointermove', move)
    // After pointerup or pointercancel alike, and after either was missed.
    handle.addEventListener('lostpointercapture', stop)
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
    style: { [name]: `${width}px` } as CSSProperties,
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
