// Text and layout (ADR 0107): how big and how dense the UI is, over any
// theme. The daemon keeps it per machine in appearance.json beside the theme
// pick and compiles its CSS; the hosted app keeps it in this browser. A
// setting left out is Silt's default.

import { useEffect } from 'react'
import type { Density, Detail, DiffLayout, Grouping, LineHeight, Motion, ReadingWidth, TextSettings } from './api'
import { inDesktop, tellDesktopZoom } from './desktop'
import { hosted } from './served'
import { type Action, formatKeys, useActions } from './shortcuts'
import { useAppearance } from './theme'
import { toast } from './ui'

export type TextKey = keyof TextSettings

/** A size's range and step, as silt-theme clamps it. */
export interface Range {
  min: number
  max: number
  step: number
  default: number
}

export const RANGES = {
  zoom: { min: 50, max: 200, step: 10, default: 100 },
  ui_size: { min: 11, max: 18, step: 1, default: 13 },
  chat_size: { min: 11, max: 22, step: 1, default: 13 },
  code_size: { min: 10, max: 20, step: 1, default: 12 },
} satisfies Record<string, Range>

export type SizeKey = keyof typeof RANGES

/** What each setting is when it's left out. */
export const TEXT_DEFAULTS = {
  zoom: 100,
  ui_size: 13,
  chat_size: 13,
  code_size: 12,
  ui_font: null as string | null,
  code_font: null as string | null,
  ligatures: true,
  density: 'regular' as Density,
  line_height: 'regular' as LineHeight,
  width: 'regular' as ReadingWidth,
  diff_layout: null as DiffLayout | null,
  wrap: true,
  tab_width: 4,
  detail: 'normal' as Detail,
  grouping: 'turn' as Grouping,
  motion: 'system' as Motion,
} satisfies Record<TextKey, unknown>

export type Effective = typeof TEXT_DEFAULTS

/** Every setting, its default where it's left out. */
export function effective(text: TextSettings): Effective {
  const out: Record<string, unknown> = { ...TEXT_DEFAULTS }
  for (const [key, value] of Object.entries(text)) if (value !== null && value !== undefined) out[key] = value
  return out as Effective
}

/** `text` without what's left at its default, so it reads as unchanged and a later default applies. */
export function cleaned(text: TextSettings): TextSettings {
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(text)) {
    if (value === null || value === undefined || value === '') continue
    if (value === (TEXT_DEFAULTS as Record<string, unknown>)[key]) continue
    out[key] = value
  }
  return out as TextSettings
}

/** Whether `key` is set to something other than Silt's default. */
export function changed(text: TextSettings, key: TextKey): boolean {
  return key in cleaned(text)
}

/** `value` moved `steps` steps within `range`. */
export function stepped(range: Range, value: number, steps: number): number {
  return Math.min(range.max, Math.max(range.min, value + steps * range.step))
}

/** The text and layout shown, and a way to change it. */
export function useTextSettings() {
  const { catalog, settings, save } = useAppearance()
  const text = settings.text ?? {}
  const change = (next: TextSettings) => save({ ...settings, text: cleaned(next) })
  return {
    text,
    shown: effective(text),
    /** Whether the daemon has answered, so what's shown is what's kept. */
    ready: hosted() || catalog.isSuccess,
    error: catalog.isError ? catalog.error : null,
    change,
    set: <K extends TextKey>(key: K, value: TextSettings[K]) => change({ ...text, [key]: value }),
    reset: (key: TextKey) => change({ ...text, [key]: null }),
    resetAll: () => change({}),
  }
}

const OLD_DETAIL = 'silt.transcript-detail'
const OLD_GROUPING = 'silt.transcript-grouping'

function takeOld(key: string): string | null {
  try {
    const value = localStorage.getItem(key)
    localStorage.removeItem(key)
    return value
  } catch {
    return null
  }
}

/**
 * Keeps the page's motion and the desktop app's zoom as set, and moves the
 * transcript choices this browser kept to the machine's settings once.
 */
export function useTextSync() {
  const { text, shown, ready, change } = useTextSettings()
  const motion = shown.motion
  useEffect(() => {
    if (motion === 'system') delete document.documentElement.dataset.motion
    else document.documentElement.dataset.motion = motion
  }, [motion])
  const zoom = shown.zoom
  useEffect(() => {
    if (ready) tellDesktopZoom(zoom)
  }, [zoom, ready])
  useEffect(() => {
    if (!ready || hosted()) return
    const detail = takeOld(OLD_DETAIL)
    const grouping = takeOld(OLD_GROUPING)
    const moved: TextSettings = { ...text }
    if (!text.detail && (detail === 'normal' || detail === 'verbose')) moved.detail = detail
    if (!text.grouping && (grouping === 'turn' || grouping === 'run')) moved.grouping = grouping
    if (moved.detail !== text.detail || moved.grouping !== text.grouping) void change(moved).catch(() => {})
    // Once, as the daemon first answers.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready])
}

/** Zoom by key in the desktop app, where nothing else zooms; a browser keeps its own. */
export function useTextActions(goToSettings: () => void) {
  const { shown, set } = useTextSettings()
  const zoomTo = (percent: number) => {
    set('zoom', percent)
    toast(percent === 100 ? 'Zoom reset' : `Zoom ${percent}%. ${formatKeys('mod+0')} resets it.`, 'ok', 'zoom')
  }
  const size = (key: SizeKey, steps: number) => set(key, stepped(RANGES[key], shown[key], steps))
  const desktop = inDesktop()
  const actions: Action[] = [
    ...(desktop
      ? [
          { id: 'zoom-in', label: 'Zoom in', keys: 'mod+=', run: () => zoomTo(stepped(RANGES.zoom, shown.zoom, 1)) },
          { id: 'zoom-out', label: 'Zoom out', keys: 'mod+-', run: () => zoomTo(stepped(RANGES.zoom, shown.zoom, -1)) },
          { id: 'zoom-reset', label: 'Reset zoom', keys: 'mod+0', run: () => zoomTo(100) },
        ]
      : []),
    { id: 'chat-bigger', label: 'Bigger conversation text', run: () => size('chat_size', 1) },
    { id: 'chat-smaller', label: 'Smaller conversation text', run: () => size('chat_size', -1) },
    { id: 'code-bigger', label: 'Bigger code text', run: () => size('code_size', 1) },
    { id: 'code-smaller', label: 'Smaller code text', run: () => size('code_size', -1) },
    { id: 'text-layout', label: 'Text and layout settings', run: goToSettings },
  ]
  useActions(actions)
}
