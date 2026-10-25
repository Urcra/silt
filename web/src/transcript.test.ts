import { expect, test } from 'vitest'
import type { HistoryEntry } from './api'
import { foldInherited, imageSize, piecesOf, rowsOf, shortCommand, stepsOf, tailOf, testsOf, toolSummary } from './transcript'

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
  expect(rows.map((row) => row.kind)).toEqual(['origin', 'turn', 'origin', 'turn', 'compaction'])
  const turn = rows[3]
  if (turn.kind !== 'turn') throw new Error('a turn')
  expect(turn.turn?.number).toBe(1)
  expect(piecesOf(turn.entries).map((piece) => piece.kind)).toEqual(['tools', 'entry'])
  expect(toolSummary(turn.entries)).toBe('Read 1 file, edited 1 file')
  const compaction = rows[4]
  expect(compaction.kind === 'compaction' && compaction.summary?.text).toBe('Summary: farewell added.')

  const folded = foldInherited(rows)
  expect(folded.map((row) => row.kind)).toEqual(['origin', 'origin', 'turn', 'compaction'])
  expect(folded[0].kind === 'origin' && folded[0].folded).toBe(1)
  expect(folded[1].kind === 'origin' && folded[1].origin).toBe('own')
})

const call = (index: number, id: string, kind: string, subject: string): HistoryEntry =>
  ({ ...entry(index, 'tool_call', `${kind} {}`), tool: { id, name: kind, kind, subject, detail: null }, outcome: null }) as HistoryEntry
const result = (index: number, id: string, text: string, failed = false): HistoryEntry =>
  ({ ...entry(index, 'tool_result', text), tool: null, outcome: { call: id, failed, exit_code: failed ? 101 : null } }) as HistoryEntry

test('each call gets the result that answers it, and the run is summed up by kind', () => {
  const entries = [
    call(0, 'a', 'search', 'progress'),
    call(1, 'b', 'execute', 'cargo test'),
    result(2, 'b', 'test result: FAILED', true),
    result(3, 'a', 'src/upload.rs:4'),
    call(4, 'c', 'execute', 'cargo test'),
    result(5, 'c', 'ok'),
    call(6, 'd', 'edit', 'src/upload.rs'),
  ]
  const steps = stepsOf(entries)
  expect(steps.map((step) => [step.call?.tool?.id, step.result?.index ?? null])).toEqual([
    ['a', 3],
    ['b', 2],
    ['c', 5],
    ['d', null],
  ])
  expect(toolSummary(entries)).toBe('Searched 1 pattern, ran 2 commands, edited 1 file')
  expect(toolSummary([...entries, call(7, 'e', 'edit', 'src/upload.rs')])).toBe('Searched 1 pattern, ran 2 commands, edited 1 file')
  expect(tailOf('one\ntwo\nthree\nfour\nfive\n', 2)).toEqual({ lines: ['four', 'five'], hidden: 3 })
})

test("a call's update replaces it, and a result joins its call's fold across a command", () => {
  const announced = { ...call(0, 'a', 'execute', ''), text: 'Terminal {}' } as HistoryEntry
  const updated = { ...call(1, 'a', 'execute', 'python3 -m unittest'), text: '`python3 -m unittest` {"command":"python3 -m unittest"}' } as HistoryEntry
  const entries = [announced, updated, call(2, 'r', 'read', 'calc.py'), call(3, 'b', 'execute', 'ls'), result(4, 'r', 'def add'), result(5, 'b', 'README.md'), result(6, 'a', 'OK')]
  const steps = stepsOf(entries)
  expect(steps.map((step) => [step.call?.index, step.result?.index])).toEqual([
    [1, 6],
    [2, 4],
    [3, 5],
  ])
  const pieces = piecesOf(entries)
  expect(pieces.map((piece) => (piece.kind === 'tools' ? piece.entries.map((each) => each.index) : piece.kind))).toEqual(['step', [2, 4], 'step'])
})

test('a turn shows its plan once, as it stood last', () => {
  const plan = (index: number, status: string): HistoryEntry =>
    ({ ...entry(index, 'plan', 'Fix it'), plan: [{ text: 'Fix it', status }] }) as HistoryEntry
  const todo = { ...call(2, 'p', 'other', ''), plan: [{ text: 'Fix it', status: 'completed' }] } as HistoryEntry
  const entries = [plan(0, 'pending'), entry(1, 'assistant', 'On it.'), todo, result(3, 'p', 'Todos have been modified'), plan(4, 'in_progress')]
  const pieces = piecesOf(entries)
  expect(pieces.map((piece) => piece.kind)).toEqual(['entry', 'plan'])
  expect(pieces[1].kind === 'plan' && pieces[1].entry.index).toBe(4)
  expect(piecesOf(entries.slice(0, 4)).map((piece) => piece.kind)).toEqual(['entry', 'plan'])
})

test('a call still waiting for its result says what it is doing, while its turn runs', () => {
  const entries = [call(0, 'a', 'read', 'src/lib.rs'), result(1, 'a', 'fn x() {}'), call(2, 'b', 'edit', 'src/main.rs')]
  expect(toolSummary(entries, true)).toBe('Read 1 file · now editing 1 file')
  expect(toolSummary(entries)).toBe('Read 1 file, edited 1 file')
})

test('a command says how its tests went, and an image how big it is', () => {
  expect(testsOf('running 2 tests\ntest result: ok. 2 passed; 0 failed; 0 ignored')).toEqual({ passed: 2, failed: 0 })
  expect(testsOf('test result: FAILED. 1 passed; 1 failed')).toEqual({ passed: 1, failed: 1 })
  expect(testsOf('==== 1 failed, 3 passed in 0.12s ====')).toEqual({ passed: 3, failed: 1 })
  expect(testsOf('Capturing screenshot into upload.png')).toBeNull()
  expect(imageSize('A'.repeat(4 * 1024))).toBe('3 KB')
})

test('a long command shrinks its URLs to their last segment', () => {
  expect(shortCommand('npx playwright screenshot http://localhost:5173/upload upload.png')).toBe('npx playwright screenshot …/upload upload.png')
  expect(shortCommand('cargo test upload')).toBe('cargo test upload')
})
