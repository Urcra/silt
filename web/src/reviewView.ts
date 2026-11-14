// What the expanded review shows, kept for the tab while the person goes back to the conversation and returns.

import type { DiffLayout } from './components/DiffView'
import type { View } from './diffSpec'

export interface Kept {
  view: View
  other: string
  commit: string
  layout: DiffLayout
  /** The turns picked, first and last, in place of `view`. */
  turns?: [number, number]
  /** Whether the person picked the turns, or all of them, rather than start after the turn they reviewed through. */
  chosen?: boolean
  /** The file it was last on, for the panel to scroll to on the way back. */
  file?: string
}

const FRESH: Kept = { view: 'thread', other: 'merge-base', commit: '', layout: 'split' }

const keyOf = (thread: string) => `silt-review-view-${thread}`

export function keptView(thread: string): Kept {
  try {
    const value = sessionStorage.getItem(keyOf(thread))
    return value ? { ...FRESH, ...(JSON.parse(value) as Partial<Kept>) } : FRESH
  } catch {
    return FRESH
  }
}

export function keepView(thread: string, kept: Kept) {
  try {
    sessionStorage.setItem(keyOf(thread), JSON.stringify(kept))
  } catch {
    // Kept for this view only, then.
  }
}
