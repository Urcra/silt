import { expect, test } from 'vitest'
import type { HistoryEntry } from './api'
import { piecesOf, rowsOf, toolSummary } from './transcript'

const entry = (index: number, kind: string, text: string, origin = 'own'): HistoryEntry =>
  ({ index, thread: 't', origin, kind, text, time_ms: index * 1000, rewound: false }) as HistoryEntry

test('history reads as turns between compaction and origin dividers, with tool work folded', () => {
  const entries = [
    entry(0, 'user', 'start', 'inherited'),
    entry(1, 'assistant', 'ok', 'inherited'),
    entry(2, 'user', 'add a farewell'),
    entry(3, 'tool_call', 'Read {"file_path":"src/lib.rs"}'),
    entry(4, 'tool_result', 'fn x() {}'),
    entry(5, 'tool_call', 'Edit {"file_path":"src/main.rs"}'),
    entry(6, 'assistant', 'Done.'),
    entry(7, 'compaction', '(context compacted)'),
    entry(8, 'compaction', 'Summary: farewell added.'),
  ]
  const rows = rowsOf(entries, [{ number: 1, entry: 2, prompt: 'add a farewell' }])
  expect(rows.map((row) => row.kind)).toEqual(['origin', 'turn', 'turn', 'compaction'])
  const turn = rows[2]
  if (turn.kind !== 'turn') throw new Error('a turn')
  expect(turn.turn?.number).toBe(1)
  expect(piecesOf(turn.entries).map((piece) => piece.kind)).toEqual(['tools', 'entry'])
  expect(toolSummary(turn.entries)).toBe('Ran 2 tools · read lib.rs · edited main.rs')
  const compaction = rows[3]
  expect(compaction.kind === 'compaction' && compaction.summary?.text).toBe('Summary: farewell added.')
})
