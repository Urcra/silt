// Light, dark, or whatever the system prefers; remembered per browser.

import { useEffect, useState } from 'react'

export type Theme = 'system' | 'light' | 'dark'

const KEY = 'silt-theme'
export const THEMES: Theme[] = ['system', 'light', 'dark']

export function storedTheme(): Theme {
  try {
    const stored = localStorage.getItem(KEY)
    return stored === 'light' || stored === 'dark' ? stored : 'system'
  } catch {
    return 'system'
  }
}

export function applyTheme(theme: Theme) {
  if (theme === 'system') delete document.documentElement.dataset.theme
  else document.documentElement.dataset.theme = theme
  try {
    if (theme === 'system') localStorage.removeItem(KEY)
    else localStorage.setItem(KEY, theme)
  } catch {
    // Storage may be off; the choice then lasts for this page only.
  }
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
