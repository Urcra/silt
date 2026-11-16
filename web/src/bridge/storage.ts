// A viewer's own storage for an artifact's page (ADR 0072): the frame's opaque origin has none, so the holding page
// keeps it, in its own `localStorage` for `localStorage` and in memory for `sessionStorage`. It never leaves the browser.

/** The most one artifact's storage may hold, as JSON. */
export const STORAGE_BYTES = 1 << 20

export type Kept = { local: Record<string, string>; session: Record<string, string> }

const sessions = new Map<string, Record<string, string>>()

function key(thread: string, artifact: string): string {
  return `silt-artifact:${thread}:${artifact}`
}

function strings(value: unknown): Record<string, string> {
  const out: Record<string, string> = {}
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return out
  for (const [name, item] of Object.entries(value)) out[name] = String(item)
  return out
}

/** What the viewer's page stored for this artifact. */
export function keptFor(thread: string, artifact: string): Kept {
  let local: Record<string, string> = {}
  try {
    local = strings(JSON.parse(window.localStorage.getItem(key(thread, artifact)) ?? '{}'))
  } catch {
    local = {}
  }
  return { local, session: { ...(sessions.get(key(thread, artifact)) ?? {}) } }
}

/** Keeps what the page stored; false when it's over the limit, and nothing is kept. */
export function keep(thread: string, artifact: string, kind: unknown, items: unknown): boolean {
  const stored = strings(items)
  if (JSON.stringify(stored).length > STORAGE_BYTES) return false
  if (kind === 'session') {
    sessions.set(key(thread, artifact), stored)
    return true
  }
  if (kind !== 'local') return false
  try {
    window.localStorage.setItem(key(thread, artifact), JSON.stringify(stored))
    return true
  } catch {
    return false
  }
}
