// December's decorations in the desktop app (ADR 0133): when they show, and
// whether anything in them may move.

import { useSyncExternalStore } from 'react'
import type { SeasonalSettings } from './api'
import { inDesktop } from './desktop'
import { useNow } from './now'
import { hosted } from './served'
import { useAppearance } from './theme'

export const DEFAULT_SEASONAL: SeasonalSettings = { show: 'auto', snow: true }

/** 1 December to 6 January, by the local calendar. */
export function inSeason(date: Date): boolean {
  const month = date.getMonth()
  return month === 11 || (month === 0 && date.getDate() <= 6)
}

/** Whether the decorations show: only in the desktop app, and on Auto only in season. */
export function decorated(settings: SeasonalSettings, date: Date, desktop: boolean): boolean {
  if (!desktop) return false
  return settings.show === 'always' || (settings.show === 'auto' && inSeason(date))
}

/** Whether this page is the desktop app's, which alone offers the decorations. */
export function seasonalOffered(): boolean {
  return inDesktop() && !hosted()
}

/** Whether the decorations show now, and whether snow may fall. */
export function useSeasonal(): { on: boolean; snow: boolean } {
  const { settings } = useAppearance()
  const now = useNow(15 * 60_000)
  const seasonal = settings.seasonal ?? DEFAULT_SEASONAL
  const on = decorated(seasonal, new Date(now), seasonalOffered())
  return { on, snow: on && seasonal.snow }
}

function motionReduced(): boolean {
  const set = document.documentElement.dataset.motion
  if (set === 'reduced') return true
  if (set === 'full') return false
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches
}

function hearMotion(changed: () => void) {
  const root = new MutationObserver(changed)
  root.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-motion'],
  })
  const query = typeof matchMedia === 'function' ? matchMedia('(prefers-reduced-motion: reduce)') : null
  query?.addEventListener('change', changed)
  return () => {
    root.disconnect()
    query?.removeEventListener('change', changed)
  }
}

/** Less motion, as Settings › Text and layout or the system asks (ADR 0107). */
export function useMotionReduced(): boolean {
  return useSyncExternalStore(hearMotion, motionReduced)
}
