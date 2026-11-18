import { useEffect, useState } from 'react'

/** Where the desktop app can show Silt from: this computer, and each WSL distribution (ADR 0045). */
export interface Backend {
  id: string
  label: string
}

declare global {
  interface Window {
    /** Set by the desktop app's window when it has more than one place to show Silt from. */
    __siltBackends?: Backend[]
    __siltBackend?: string
  }
}

/** The backends the desktop app offers and the one it shows, or none outside the app or with nothing to choose. */
export function useBackends(): { backends: Backend[]; current: string | undefined } {
  const [current, setCurrent] = useState(() => window.__siltBackend)
  useEffect(() => {
    const shown = (event: Event) => setCurrent((event as CustomEvent<string>).detail)
    window.addEventListener('silt-backend', shown)
    return () => window.removeEventListener('silt-backend', shown)
  }, [])
  return { backends: window.__siltBackends ?? [], current }
}

/** Asks the desktop app's window to show Silt from `backend`, which it does in place of opening the link. */
export function switchBackend(backend: string) {
  window.location.href = `silt://open/?backend=${encodeURIComponent(backend)}`
}

/** `rgb(r, g, b)` or `rgba(…)` as a browser computes it, as `#rrggbb`. */
export function hexOf(computed: string): string | null {
  const channels = computed.match(/^rgba?\((\d+),\s*(\d+),\s*(\d+)/)
  if (!channels) return null
  return `#${channels
    .slice(1, 4)
    .map((channel) => Number(channel).toString(16).padStart(2, '0'))
    .join('')}`
}

/** The link that tells the desktop app's window which theme the page shows (ADR 0099). */
export function themeLink(appearance: 'light' | 'dark', bg: string, fg: string): string {
  return `silt://theme/?${new URLSearchParams({ appearance, bg, fg })}`
}

let told = ''

/** Tells the desktop app's window the theme shown, so its frame and next start follow it; nothing in a browser. */
export function tellDesktopTheme(appearance: 'light' | 'dark') {
  if (!window.__siltBackends) return
  const body = getComputedStyle(document.body)
  const bg = hexOf(body.backgroundColor)
  const fg = hexOf(body.color)
  if (!bg || !fg) return
  const link = themeLink(appearance, bg, fg)
  if (link === told) return
  told = link
  window.location.href = link
}
