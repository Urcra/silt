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
