// Themes for Silt's UI (ADR 0098): a light and a dark theme, and whether the
// light one, the dark one or the system's choice shows. A daemon keeps the
// pick and puts the theme into the page it serves; the hosted app keeps it
// in this browser.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState, useSyncExternalStore } from 'react'
import { type AppearanceMode, type AppearanceSettings, type ThemeCatalog, api } from './api'
import { tellDesktopTheme } from './desktop'
import { keys } from './query'
import { hosted } from './served'

export type Theme = AppearanceMode

const MODE_KEY = 'silt-theme'
/** The hosted app's light and dark theme, and its text and layout (ADR 0107). */
const PICK_KEY = 'silt-theme-pick'
/** The last CSS shown, so the hosted app's next page starts in it. */
const CSS_KEY = 'silt-theme-css'
/** Set once a mode kept by the browser has moved to the daemon. */
const MOVED_KEY = 'silt-theme-moved'
export const THEMES: Theme[] = ['system', 'light', 'dark']

export const DEFAULT_APPEARANCE: AppearanceSettings = {
  mode: 'system',
  light: 'nous-light',
  dark: 'nous-dark',
  text: {},
  seasonal: { show: 'auto', snow: true },
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key)
    else localStorage.setItem(key, value)
  } catch {
    // Storage may be off; the choice then lasts for this page only.
  }
}

export function storedTheme(): Theme {
  const stored = read(MODE_KEY)
  return stored === 'light' || stored === 'dark' ? stored : 'system'
}

/** Shows `theme`'s mode: a light or dark theme, or the system's choice. */
export function applyMode(theme: Theme) {
  if (theme === 'system') delete document.documentElement.dataset.theme
  else document.documentElement.dataset.theme = theme
}

/** Shows `theme`'s mode and keeps it in this browser, as the hosted app does. */
export function applyTheme(theme: Theme) {
  applyMode(theme)
  write(MODE_KEY, theme === 'system' ? null : theme)
}

function styleElement(id: string): HTMLStyleElement {
  const found = document.getElementById(id)
  if (found instanceof HTMLStyleElement) return found
  const style = document.createElement('style')
  style.id = id
  document.head.appendChild(style)
  return style
}

/** The theme's CSS, after nous.css; it ends any preview. */
export function injectThemeCss(css: string) {
  const style = styleElement('silt-theme')
  if (style.textContent !== css) style.textContent = css
  if (hosted()) write(CSS_KEY, css)
  endPreview()
}

let previewing: { mode: string | undefined } | null = null

/** Shows `css` over the theme until `endPreview`, in its own appearance. */
export function previewTheme(css: string, appearance: string) {
  previewing ??= { mode: document.documentElement.dataset.theme }
  styleElement('silt-theme-preview').textContent = css
  document.documentElement.dataset.theme = appearance
}

export function endPreview() {
  document.getElementById('silt-theme-preview')?.remove()
  if (!previewing) return
  if (previewing.mode === undefined) delete document.documentElement.dataset.theme
  else document.documentElement.dataset.theme = previewing.mode
  previewing = null
}

/** At startup: a daemon's page has its theme already; the hosted app's comes from this browser. */
export function startTheme() {
  if (!hosted() && document.getElementById('silt-theme')) return
  applyMode(storedTheme())
  const css = read(CSS_KEY)
  if (css) styleElement('silt-theme').textContent = css
}

const pickHeard = new Set<() => void>()

/** What the hosted app keeps, as a string that changes when the pick does, so the page draws a change at once. */
function keptPick(): string {
  return `${read(MODE_KEY) ?? ''}|${read(PICK_KEY) ?? ''}`
}

function hearPick(listener: () => void) {
  pickHeard.add(listener)
  return () => pickHeard.delete(listener)
}

/** The hosted app's pick, from what `keptPick` read. */
function hostedPick(kept: string): AppearanceSettings {
  try {
    const pick = JSON.parse(kept.slice(kept.indexOf('|') + 1) || '{}') as Partial<AppearanceSettings>
    const text = pick.text && typeof pick.text === 'object' ? pick.text : {}
    return { ...DEFAULT_APPEARANCE, ...pick, text, mode: storedTheme() }
  } catch {
    return { ...DEFAULT_APPEARANCE, mode: storedTheme() }
  }
}

/** The themes, the pick, and a way to change it. */
export function useAppearance() {
  const queries = useQueryClient()
  const catalog = useQuery<ThemeCatalog>({ queryKey: keys.themes, queryFn: api.themes, staleTime: 60_000 })
  // A daemon from before themes answers with no catalog; it shows Nous, and one from before text and layout no text.
  const kept = useSyncExternalStore(hearPick, keptPick)
  const given = hosted() ? hostedPick(kept) : (catalog.data?.appearance ?? DEFAULT_APPEARANCE)
  // A daemon from before the December decorations answers without them (ADR 0133).
  const settings = { ...given, text: given.text ?? {}, seasonal: given.seasonal ?? DEFAULT_APPEARANCE.seasonal }
  const save = async (next: AppearanceSettings) => {
    if (hosted()) {
      write(PICK_KEY, JSON.stringify({ light: next.light, dark: next.dark, text: next.text }))
      applyTheme(next.mode)
      for (const listener of pickHeard) listener()
    } else {
      // Shown at once, so a size stepped through draws each step; put back if the daemon refuses it.
      queries.setQueryData<ThemeCatalog>(keys.themes, (before) => before && { ...before, appearance: next })
      try {
        await api.setAppearance(next)
      } catch (failure) {
        await queries.invalidateQueries({ queryKey: keys.themes })
        throw failure
      }
      applyMode(next.mode)
    }
    await queries.invalidateQueries({ queryKey: keys.themes })
  }
  return { catalog, settings, save }
}

/** Keeps the page in the picked theme, and moves a mode this browser kept to the daemon once. */
export function useThemeSync() {
  const { catalog, settings, save } = useAppearance()
  const css = useQuery({
    queryKey: [...keys.themeCss, settings.light, settings.dark, JSON.stringify(settings.text)],
    queryFn: () => api.themeCss({ light: settings.light, dark: settings.dark, text: settings.text }),
    enabled: hosted() || catalog.isSuccess,
  })
  useEffect(() => {
    if (typeof css.data?.css === 'string') injectThemeCss(css.data.css)
  }, [css.data])
  const shown = useShownTheme()
  useEffect(() => {
    // After the page has drawn in it, so the colours read are the theme's.
    const frame = requestAnimationFrame(() => tellDesktopTheme(shown, settings.seasonal.show))
    return () => cancelAnimationFrame(frame)
  }, [shown, css.data, settings.seasonal.show])
  const daemonMode = hosted() ? null : catalog.data?.appearance?.mode
  useEffect(() => {
    if (!daemonMode || previewing) return
    const kept = storedTheme()
    if (kept !== 'system' && daemonMode === 'system' && read(MOVED_KEY) === null) {
      write(MOVED_KEY, '1')
      write(MODE_KEY, null)
      void save({ ...settings, mode: kept })
      return
    }
    write(MOVED_KEY, '1')
    applyMode(daemonMode)
    // Only the mode the daemon answers with matters here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [daemonMode])
}

/** Previews a theme from the palette: its CSS, fetched as the highlight lands on it. */
let previewAsked = 0
export function previewThemeById(id: string, appearance: string) {
  const asked = ++previewAsked
  const pick = appearance === 'light' ? { light: id } : { dark: id }
  void api.themeCss(pick).then(({ css }) => {
    if (asked === previewAsked) previewTheme(css, appearance)
  })
}

export function cancelPreview() {
  previewAsked++
  endPreview()
}

/** The theme the page shows now: the one picked, else the system's. */
export function shownTheme(): 'light' | 'dark' {
  const picked = document.documentElement.dataset.theme
  if (picked === 'light' || picked === 'dark') return picked
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

/** The shown theme, following a new pick or a change of the system's. */
export function useShownTheme(): 'light' | 'dark' {
  const [theme, setTheme] = useState(shownTheme)
  useEffect(() => {
    const update = () => setTheme(shownTheme())
    const picks = new MutationObserver(update)
    picks.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] })
    const system = window.matchMedia?.('(prefers-color-scheme: dark)')
    system?.addEventListener('change', update)
    return () => {
      picks.disconnect()
      system?.removeEventListener('change', update)
    }
  }, [])
  return theme
}
