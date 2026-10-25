import { expect, test } from 'vitest'
import type { GraphRow } from './api'
import { byFamily, layout } from './lanes'

function row(id: string, thread: string, kind: string, overrides: Partial<GraphRow> = {}): GraphRow {
  return {
    id,
    kind,
    thread,
    family: 'f',
    time_ms: 0,
    turn: null,
    text: '',
    other: null,
    status: null,
    owner: 'alice',
    agent: null,
    settled: false,
    commit: null,
    ...overrides,
  }
}

test('a fork leaves its source in a lane of its own and merges back', () => {
  // Newest first: the merge, the request, the fork's turn, the fork, the source's turn.
  const rows = [
    row('5', 'main', 'merge', { other: 'fork' }),
    row('4', 'fork', 'request', { other: 'main', status: 'approved' }),
    row('3', 'fork', 'turn'),
    row('2', 'fork', 'fork', { other: 'main' }),
    row('1', 'main', 'turn'),
  ]
  const { lanes, edges, width } = layout(rows)
  expect(lanes).toEqual([0, 1, 1, 1, 0])
  expect(width).toBe(2)
  expect(edges).toEqual([
    { row: 0, from: 1, to: 0, kind: 'merge', dashed: false },
    { row: 1, from: 1, to: 0, kind: 'request', dashed: false },
    { row: 3, from: 0, to: 1, kind: 'fork', dashed: false },
  ])
})

test('an open request is dashed, and a declined fork is abandoned', () => {
  const open = layout([row('2', 'fork', 'request', { other: 'main', status: 'open' }), row('1', 'main', 'turn')])
  expect(open.edges[0].dashed).toBe(true)
  const declined = layout([row('2', 'fork', 'request', { other: 'main', status: 'declined' }), row('1', 'fork', 'fork', { other: 'main' })])
  expect(declined.threads.get('fork')?.abandoned).toBe(true)
  expect(declined.threads.get('main')?.abandoned).toBe(false)
  const settled = layout([row('1', 'main', 'turn', { settled: true })])
  expect(settled.threads.get('main')?.abandoned).toBe(true)
})

test('a lane is reused once its thread is past', () => {
  // `c` has only its newest row, so `b` takes the lane right after it, and
  // `a`, alongside `b`, gets the next.
  const rows = [row('4', 'c', 'turn'), row('3', 'b', 'turn'), row('2', 'a', 'turn'), row('1', 'b', 'turn')]
  const { lanes, width } = layout(rows)
  expect(lanes).toEqual([0, 0, 1, 0])
  expect(width).toBe(2)
})

test("unrelated projects' threads don't run through each other", () => {
  const rows = [row('4', 'a', 'turn'), row('3', 'x', 'turn', { family: 'g' }), row('2', 'x', 'turn', { family: 'g' }), row('1', 'a', 'turn')]
  const grouped = byFamily(rows)
  expect(grouped.map((shown) => shown.id)).toEqual(['4', '1', '3', '2'])
  const { threads } = layout(grouped)
  expect(threads.get('a')).toMatchObject({ top: 0, bottom: 1 })
  expect(threads.get('x')).toMatchObject({ top: 2, bottom: 3 })
})
