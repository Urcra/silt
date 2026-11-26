import { type MouseEvent, type TouchEvent, useCallback, useEffect, useRef, useState } from 'react'
import type { MenuAt } from './overlays'

/** How long a touch is held before it opens the menu a right-click would, in milliseconds. */
const LONG_PRESS_MS = 500
/** How far a touch may move, in pixels, and still be held rather than scrolling. */
const LONG_PRESS_SLOP = 10

/**
 * Where a right-click, or on a touch screen a long press, opened an element's menu, with the handlers that open it. A
 * long press doesn't then follow the link it was on; iOS fires no contextmenu for one, so the touch is timed here.
 */
export function useMenuAt() {
  const [at, setAt] = useState<MenuAt>()
  const pressing = useRef<{ x: number; y: number; timer: number }>(undefined)
  const pressed = useRef(false)
  const stop = () => {
    if (pressing.current) window.clearTimeout(pressing.current.timer)
    pressing.current = undefined
  }
  useEffect(() => stop, [])
  const close = useCallback(() => setAt(undefined), [])
  const handlers = {
    onContextMenu: (event: MouseEvent) => {
      event.preventDefault()
      stop()
      setAt({ x: event.clientX, y: event.clientY })
    },
    onTouchStart: (event: TouchEvent) => {
      const touch = event.touches[0]
      if (!touch || event.touches.length > 1) return stop()
      pressed.current = false
      const { clientX: x, clientY: y } = touch
      pressing.current = {
        x,
        y,
        timer: window.setTimeout(() => {
          pressing.current = undefined
          pressed.current = true
          navigator.vibrate?.(10)
          setAt({ x, y })
        }, LONG_PRESS_MS),
      }
    },
    onTouchMove: (event: TouchEvent) => {
      const touch = event.touches[0]
      const from = pressing.current
      if (from && touch && Math.hypot(touch.clientX - from.x, touch.clientY - from.y) > LONG_PRESS_SLOP) stop()
    },
    onTouchEnd: (event: TouchEvent) => {
      stop()
      // The press opened the menu, so the tap that ends it isn't a click on the row.
      if (pressed.current) event.preventDefault()
    },
    onTouchCancel: stop,
    onClickCapture: (event: MouseEvent) => {
      if (!pressed.current) return
      pressed.current = false
      event.preventDefault()
      event.stopPropagation()
    },
  }
  return { at, close, handlers }
}
