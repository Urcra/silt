import { useCallback, useEffect, useState, useSyncExternalStore } from 'react'

/** One clock per period, shared by every view on it: each row of a long list kept timers of its own. */
const clocks = new Map<number, { now: number; listeners: Set<() => void>; timer: ReturnType<typeof setInterval> | null }>()

function clockOf(everyMs: number) {
  let clock = clocks.get(everyMs)
  if (!clock) {
    clock = { now: Date.now(), listeners: new Set(), timer: null }
    clocks.set(everyMs, clock)
  }
  return clock
}

/** The time, brought up to date every `everyMs` (never with 0), for views that change with it. */
export function useNow(everyMs = 60_000): number {
  const subscribe = useCallback(
    (changed: () => void) => {
      if (everyMs <= 0) return () => undefined
      const clock = clockOf(everyMs)
      clock.listeners.add(changed)
      if (clock.timer === null) {
        clock.now = Date.now()
        clock.timer = setInterval(() => {
          clock.now = Date.now()
          for (const listener of clock.listeners) listener()
        }, everyMs)
      }
      return () => {
        clock.listeners.delete(changed)
        if (clock.listeners.size > 0 || clock.timer === null) return
        clearInterval(clock.timer)
        clock.timer = null
      }
    },
    [everyMs],
  )
  const [mounted] = useState(() => Date.now())
  return useSyncExternalStore(subscribe, () => (everyMs <= 0 ? mounted : Math.max(clockOf(everyMs).now, mounted)))
}

/** `value` once it has stayed the same for `ms`. */
export function useSettled<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value)
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), ms)
    return () => clearTimeout(timer)
  }, [value, ms])
  return settled
}
