/**
 * What the agent streamed that its status dropped once the daemon recorded it. The status comes at once and the
 * history a fetch later, so the page shows these until the history has them rather than letting them blink out
 * and back.
 */
export interface Said {
  text: string
  thought: string
}

export const NOTHING_SAID: Said = { text: '', thought: '' }

/** Whether the current turn of `entries` has `text` as an entry of `kind`: the turn's own, as an earlier one may say the same. */
export function recorded(text: string, kind: 'assistant' | 'thinking', entries: { kind: string; text: string }[]): boolean {
  const said = text.trim()
  for (let at = entries.length - 1; at >= 0; at--) {
    const entry = entries[at]
    if (entry.kind === kind && entry.text.trim() === said) return true
    if (entry.kind === 'user' || entry.kind === 'wake') return false
  }
  return false
}

/** What of `said` the page still shows: whatever `entries` doesn't have yet. */
export function unrecorded(said: Said, entries: { kind: string; text: string }[]): Said {
  return {
    text: said.text && !recorded(said.text, 'assistant', entries) ? said.text : '',
    thought: said.thought && !recorded(said.thought, 'thinking', entries) ? said.thought : '',
  }
}
