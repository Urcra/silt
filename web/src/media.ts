import { useSyncExternalStore } from 'react'

/** Whether `query` matches now, following changes, such as a window resized past a breakpoint. */
export function useMedia(query: string): boolean {
  return useSyncExternalStore(
    (changed) => {
      const list = window.matchMedia?.(query)
      list?.addEventListener('change', changed)
      return () => list?.removeEventListener('change', changed)
    },
    () => window.matchMedia?.(query).matches ?? false,
  )
}

/** Tailwind's `lg`: wide enough for a side panel beside the main view. */
export const WIDE = '(min-width: 1024px)'
