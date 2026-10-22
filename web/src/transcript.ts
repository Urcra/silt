// A thread's history as the page draws it: turns, each a prompt with what
// followed it, between dividers for compaction and for what came from
// elsewhere.

import type { HistoryEntry, TurnInfo } from './api'

export type Row =
  | { kind: 'origin'; key: string; origin: string; thread: string }
  | { kind: 'compaction'; key: string; marker: HistoryEntry | null; summary: HistoryEntry | null }
  | { kind: 'turn'; key: string; prompt: HistoryEntry | null; entries: HistoryEntry[]; turn: TurnInfo | null }

/** Entries the agent's tools made, folded together: its calls, their results and its thinking. */
export const FOLDED = new Set(['tool_call', 'tool_result', 'thinking'])

function isMarker(entry: HistoryEntry): boolean {
  return entry.kind === 'compaction' && entry.text.startsWith('(context compacted')
}

export function rowsOf(entries: HistoryEntry[], turns: TurnInfo[]): Row[] {
  const turnAt = new Map(turns.filter((turn) => turn.entry !== null).map((turn) => [turn.entry, turn]))
  const rows: Row[] = []
  let current: Extract<Row, { kind: 'turn' }> | null = null
  let origin = 'own'
  for (const entry of entries) {
    if (entry.origin !== origin) {
      origin = entry.origin
      current = null
      if (origin !== 'own') rows.push({ kind: 'origin', key: `origin-${entry.index}`, origin, thread: entry.thread })
    }
    if (entry.kind === 'compaction') {
      const last = rows[rows.length - 1]
      if (!isMarker(entry) && last?.kind === 'compaction' && last.summary === null) last.summary = entry
      else rows.push({ kind: 'compaction', key: `compaction-${entry.index}`, marker: isMarker(entry) ? entry : null, summary: isMarker(entry) ? null : entry })
      current = null
      continue
    }
    if (entry.kind === 'user' || current === null) {
      current = {
        kind: 'turn',
        key: `turn-${entry.index}`,
        prompt: entry.kind === 'user' ? entry : null,
        entries: entry.kind === 'user' ? [] : [entry],
        turn: turnAt.get(entry.index) ?? null,
      }
      rows.push(current)
      continue
    }
    current.entries.push(entry)
  }
  return rows
}

/** The pieces of a turn: its words, and runs of tool work folded together. */
export function piecesOf(entries: HistoryEntry[]): ({ kind: 'entry'; entry: HistoryEntry } | { kind: 'tools'; entries: HistoryEntry[] })[] {
  const pieces: ({ kind: 'entry'; entry: HistoryEntry } | { kind: 'tools'; entries: HistoryEntry[] })[] = []
  for (const entry of entries) {
    const last = pieces[pieces.length - 1]
    if (FOLDED.has(entry.kind)) {
      if (last?.kind === 'tools') last.entries.push(entry)
      else pieces.push({ kind: 'tools', entries: [entry] })
    } else pieces.push({ kind: 'entry', entry })
  }
  return pieces
}

/** A tool call's name and the file it names, from its entry's `Name {json}` text. */
export function toolOf(text: string): { name: string; file: string | null } {
  const space = text.indexOf(' ')
  const name = space < 0 ? text : text.slice(0, space)
  let file: string | null = null
  try {
    const input = JSON.parse(text.slice(space + 1)) as Record<string, unknown>
    const path = input.file_path ?? input.path ?? input.notebook_path
    if (typeof path === 'string') file = path
  } catch {
    // Not every tool takes JSON; then it names no file.
  }
  return { name, file }
}

const EDITS = new Set(['Edit', 'MultiEdit', 'Write', 'NotebookEdit', 'edit', 'write'])

/** What a run of tool work did, as its folded summary says it: `Ran 4 tools · read a.rs · edited b.rs`. */
export function toolSummary(entries: HistoryEntry[]): string {
  const calls = entries.filter((entry) => entry.kind === 'tool_call').map((entry) => toolOf(entry.text))
  const base = (file: string) => file.split('/').pop() ?? file
  const edited = [...new Set(calls.filter((call) => EDITS.has(call.name) && call.file).map((call) => base(call.file!)))]
  const read = [...new Set(calls.filter((call) => !EDITS.has(call.name) && call.file).map((call) => base(call.file!)))]
  const thought = entries.some((entry) => entry.kind === 'thinking')
  return [
    calls.length > 0 ? `Ran ${calls.length} ${calls.length === 1 ? 'tool' : 'tools'}` : thought ? 'Thought it through' : 'Tool output',
    read.length > 0 && `read ${read.join(', ')}`,
    edited.length > 0 && `edited ${edited.join(', ')}`,
  ]
    .filter(Boolean)
    .join(' · ')
}

/** How long a turn took, from its prompt to its last entry, as `1m 12s`. */
export function duration(from: number, to: number): string | null {
  const seconds = Math.round((to - from) / 1000)
  if (seconds <= 0) return null
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`
}
