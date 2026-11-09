/**
 * What the agent streamed that its status dropped once the daemon recorded it. The status comes at once and the
 * history a fetch later, so the page shows these until the history has them rather than letting them blink out
 * and back.
 */
export interface Said {
  text: string
  thought: string
  /** How many entries the history had as each began: only an entry after them can be it. */
  textAfter: number
  thoughtAfter: number
}

export const NOTHING_SAID: Said = { text: '', thought: '', textAfter: 0, thoughtAfter: 0 }

type Entry = { index: number; kind: string; text: string }

/**
 * Whether `entries` has `text` as an entry of `kind`, looking back to the start of the turn `after` falls in.
 * Prompts from `after` on don't stop the search, as a queued one can follow the words into the history at once.
 * Those before it do, as an earlier turn may say the same. `after` itself can be late: the status saying the
 * words began can be read after the fetch that brought them.
 */
export function recorded(text: string, kind: 'assistant' | 'thinking', entries: Entry[], after: number): boolean {
  const said = text.trim()
  for (let at = entries.length - 1; at >= 0; at--) {
    const entry = entries[at]
    if (entry.kind === kind && entry.text.trim() === said) return true
    if (entry.index < after && (entry.kind === 'user' || entry.kind === 'wake')) return false
  }
  return false
}

/** What of `said` the page still shows: whatever `entries` doesn't have yet. */
export function unrecorded(said: Said, entries: Entry[]): Said {
  return {
    ...said,
    text: said.text && !recorded(said.text, 'assistant', entries, said.textAfter) ? said.text : '',
    thought: said.thought && !recorded(said.thought, 'thinking', entries, said.thoughtAfter) ? said.thought : '',
  }
}
