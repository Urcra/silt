import type { HistoryEntry } from './api'

// What an agent's edit tool calls changed, read from their recorded input (ADR 0044).

export interface Change {
  verb: 'Edited' | 'Wrote'
  file: string
  removed: string[]
  added: string[]
}

const lines = (text: unknown): string[] => (typeof text === 'string' && text !== '' ? text.replace(/\n$/, '').split('\n') : [])

/** What an edit tool call changed, from its recorded input: Claude Code's Edit, MultiEdit and Write;
 * none when the input doesn't say. */
export function changeOf(entry: HistoryEntry): Change | null {
  const change = readChange(entry)
  return change && (change.removed.length > 0 || change.added.length > 0) ? change : null
}

function readChange(entry: HistoryEntry): Change | null {
  if (entry.kind !== 'tool_call') return null
  const space = entry.text.indexOf(' ')
  if (space < 0) return null
  const name = entry.text.slice(0, space)
  let input: Record<string, unknown>
  try {
    input = JSON.parse(entry.text.slice(space + 1)) as Record<string, unknown>
  } catch {
    return null
  }
  const file = input.file_path ?? input.path
  if (typeof file !== 'string') return null
  if (name === 'Edit') return { verb: 'Edited', file, removed: lines(input.old_string), added: lines(input.new_string) }
  if (name === 'MultiEdit' && Array.isArray(input.edits)) {
    const edits = input.edits as Record<string, unknown>[]
    return { verb: 'Edited', file, removed: edits.flatMap((edit) => lines(edit.old_string)), added: edits.flatMap((edit) => lines(edit.new_string)) }
  }
  if (name === 'Write') return { verb: 'Wrote', file, removed: [], added: lines(input.content) }
  return null
}
