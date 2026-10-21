// Light, dark, or whatever the system prefers; remembered per browser.

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
