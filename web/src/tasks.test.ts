import { expect, test } from 'vitest'
import type { HistoryEntry, TaskView } from './api'
import { mayRun, QUIET_MS, runOf, running, tasksOf } from './tasks'
import { callsById, piecesOf, rowsOf, stepsOf, taskEntries } from './transcript'

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

test('a message that resumes a finished subagent runs it again as its own card, with what it did since', () => {
  const started = entry(1, 'tool_call', 'Agent {}', {
    tool: {
      id: 't1',
      name: 'Agent',
      kind: 'subagent',
      subject: 'Hearth',
      detail: 'general-purpose',
      subagent: 'hearth',
      questions: [],
    },
  })
  const entries = [
    started,
    news(2, { id: 'a', call: 't1', kind: 'subagent', side: 'agent-a' }),
    entry(3, 'user', 'Build the hearth', { side: 'agent-a' }),
    news(4, { id: 'a', call: 't1', status: 'completed', summary: 'Hearth built' }),
    call(10, 's1', 'SendMessage', 'other', 'Bigger fire'),
    // The subagent's resumed prompt is written before the message's result.
    entry(11, 'user', 'Make the fire bigger', { side: 'agent-a' }),
    news(12, { id: 'a', call: 's1', kind: 'subagent', side: 'agent-a' }),
    entry(13, 'assistant', 'Fire is bigger.', { side: 'agent-a' }),
  ]
  const tasks = tasksOf(taskEntries(entries))
  const first = tasks.byCall.get('t1')!
  const again = tasks.byCall.get('s1')!
  expect(first).toMatchObject({ status: 'completed', resumes: null })
  expect(again).toMatchObject({ kind: 'subagent', status: 'running', side: 'agent-a', started_ms: 10000 })
  expect(again.resumes).toBe(started)
  expect(running(tasks)).toEqual([again])
  expect(runOf(tasks, first)).toEqual({ from_ms: null, until_ms: 10000 })
  expect(runOf(tasks, again)).toEqual({ from_ms: 10000, until_ms: null })

  const reported = tasksOf(taskEntries([...entries, news(14, { id: 'a', call: 's1', status: 'completed' })]))
  expect(running(reported)).toEqual([])
})

test('a message queued for a subagent that finished first resumes it when its news names the message', () => {
  const entries = [
    call(1, 't1', 'Agent', 'subagent', 'Hearth'),
    news(2, { id: 'a', call: 't1', kind: 'subagent', side: 'agent-a' }),
    call(3, 's1', 'SendMessage', 'other', 'Bigger fire'),
    news(4, { id: 'a', call: 't1', status: 'completed' }),
    news(5, { id: 'a', call: 's1', status: 'completed', summary: 'Fire is bigger' }),
  ]
  expect(tasksOf(taskEntries(entries)).byCall.get('s1')).toMatchObject({ kind: 'subagent', side: 'agent-a' })
})

test("a resumed run whose news names a transcript of its own keeps it, as each call's does over ACP", () => {
  const entries = [
    call(1, 't1', 'Agent', 'subagent', 'Hearth'),
    news(2, { id: 'kid', call: 't1', kind: 'subagent', side: 't1' }),
    news(3, { id: 'kid', call: 't1', status: 'completed' }),
    call(4, 't2', 'Agent', 'subagent', 'Bigger fire'),
    news(5, { id: 'kid', call: 't2', kind: 'subagent', side: 't2' }),
  ]
  const again = tasksOf(taskEntries(entries)).byCall.get('t2')
  expect(again).toMatchObject({ side: 't2', status: 'running' })
  expect(again?.resumes?.tool?.id).toBe('t1')
})
