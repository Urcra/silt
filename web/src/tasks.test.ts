import { expect, test } from 'vitest'
import type { HistoryEntry, TaskView } from './api'
import { mayRun, QUIET_MS, running, tasksOf } from './tasks'
import { callsById, piecesOf, rowsOf, stepsOf } from './transcript'

const entry = (index: number, kind: string, text: string, more: Partial<HistoryEntry> = {}): HistoryEntry =>
  ({
    index,
    thread: 't',
    origin: 'own',
    kind,
    text,
    time_ms: index * 1000,
    rewound: false,
    side: null,
    task: null,
    ...more,
  }) as HistoryEntry

const news = (index: number, task: Partial<TaskView> & { id: string }, side: string | null = null): HistoryEntry =>
  entry(index, 'task', task.summary ?? '', {
    side,
    task: { call: null, kind: null, status: 'running', side: null, summary: null, ...task },
  })

const call = (index: number, id: string, name: string, kind: string, subject: string): HistoryEntry =>
  entry(index, 'tool_call', `${name} {}`, { tool: { id, name, kind, subject, detail: null, subagent: null, questions: [] } })

test("a subagent's entries leave the thread's turns and fold under the call that started it", () => {
  const entries = [
    entry(0, 'user', 'look into it'),
    call(1, 't1', 'Agent', 'subagent', 'Investigate'),
    news(2, { id: 'agent-a', call: 't1', kind: 'subagent', side: 'agent-a' }),
    entry(3, 'user', 'Investigate the lexer', { side: 'agent-a' }),
    entry(4, 'assistant', 'The lexer is fine.', { side: 'agent-a' }),
    entry(5, 'tool_result', 'The lexer is fine.', { outcome: { call: 't1', failed: false, exit_code: null, answers: [] } }),
    news(6, { id: 'a', call: 't1', kind: 'subagent', status: 'completed', side: 'agent-a' }),
    entry(7, 'assistant', 'Done.'),
  ]
  const rows = rowsOf(entries, [{ number: 1, entry: 0, prompt: 'look into it' }])
  expect(rows).toHaveLength(1)
  const turn = rows[0]
  if (turn.kind !== 'turn') throw new Error('a turn')
  expect(turn.entries.some((candidate) => candidate.side)).toBe(false)
  const tasks = tasksOf(entries)
  const task = tasks.byCall.get('t1')
  expect(task).toMatchObject({ kind: 'subagent', status: 'completed', side: 'agent-a', ids: ['agent-a', 'a'], ended_ms: 6000 })
  expect(tasks.sides.get('agent-a')?.map((side) => side.index)).toEqual([3, 4])
  // The call stands as its own card, the news hidden.
  expect(piecesOf(turn.entries, tasks).map((piece) => piece.kind)).toEqual(['step', 'entry'])
})

test('a background command runs until its news says it ended, and news never undoes an end', () => {
  const entries = [
    call(0, 'b1', 'Bash', 'execute', 'npm run dev'),
    news(1, { id: 'bg', call: 'b1', kind: 'background' }),
    news(2, { id: 'bg', status: 'failed', summary: 'exited with code 1' }),
    news(3, { id: 'bg', status: 'running' }),
    news(4, { id: 'loose', kind: 'background', status: 'completed' }),
  ]
  const tasks = tasksOf(entries)
  expect(tasks.byCall.get('b1')).toMatchObject({ status: 'failed', summary: 'exited with code 1', ended_ms: 2000 })
  expect(tasks.loose.get('loose')?.status).toBe('completed')
  expect(running(tasks)).toEqual([])
  expect(running(tasksOf(entries.slice(0, 2))).map((task) => task.call)).toEqual(['b1'])
})

test('tasks may run only while their session does', () => {
  expect(mayRun(0, true, QUIET_MS * 2)).toBe(true)
  expect(mayRun(QUIET_MS, false, QUIET_MS + 1000)).toBe(true)
  expect(mayRun(0, false, QUIET_MS + 1)).toBe(false)
})

test("a call's later record that lost its kind keeps what the call runs", () => {
  const started = call(1, 'b1', 'sleep 600', 'execute', 'sleep 600')
  const updated = entry(2, 'tool_call', 'sleep 600 {}', {
    tool: { id: 'b1', name: 'sleep 600', kind: 'other', subject: null, detail: null, subagent: null, questions: [] },
  })
  const renamed = call(3, 'b1', 'sleep 900', 'execute', 'sleep 900')
  expect(callsById([started, updated]).get('b1')).toBe(started)
  expect(stepsOf([started, updated]).map((step) => step.call)).toEqual([started])
  expect(callsById([started, updated, renamed]).get('b1')).toBe(renamed)
})
