import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, screen, within } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, expect, test } from 'vitest'
import type { HistoryEntry, ToolView } from '../api'
import { SessionLive, TasksContext, tasksOf } from '../tasks'
import { RunningStrip, TaskCard } from './Tasks'

afterEach(cleanup)

const agentCall = (id: string, tool: Partial<ToolView>): HistoryEntry =>
  ({
    index: 1,
    thread: 't',
    origin: 'own',
    kind: 'tool_call',
    text: 'Agent {}',
    time_ms: 1000,
    rewound: false,
    side: null,
    task: null,
    tool: { id, name: 'Agent', kind: 'subagent', subject: null, detail: null, subagent: null, questions: [], ...tool },
  }) as HistoryEntry

const news = (call: string): HistoryEntry =>
  ({
    index: 2,
    thread: 't',
    origin: 'own',
    kind: 'task',
    text: '',
    time_ms: 2000,
    rewound: false,
    side: null,
    task: { id: `task-${call}`, call, kind: 'subagent', status: 'running', side: null, summary: null },
  }) as HistoryEntry

function shown(entries: HistoryEntry[], children: ReactNode) {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <SessionLive.Provider value>
        <TasksContext.Provider value={tasksOf(entries)}>{children}</TasksContext.Provider>
      </SessionLive.Provider>
    </QueryClientProvider>,
  )
}

test("a named subagent's card shows its name beside its kind and what it was asked", () => {
  const call = agentCall('c1', { subject: 'Map the lexer', detail: 'Explore', subagent: 'lexer-scout' })
  shown([call, news('c1')], <TaskCard step={{ call, result: null, images: [] }} />)
  const card = screen.getByTestId('task')
  expect(within(card).getByTestId('task-name').textContent).toBe('lexer-scout')
  expect(within(card).getByText('Explore')).toBeTruthy()
  expect(within(card).getByText('Map the lexer')).toBeTruthy()
})

test('an unnamed subagent shows no name', () => {
  const call = agentCall('c1', { subject: 'Map the lexer', detail: 'Explore' })
  shown([call, news('c1')], <TaskCard step={{ call, result: null, images: [] }} />)
  expect(screen.queryByTestId('task-name')).toBeNull()
})

test('the running strip names a named subagent', () => {
  const entries = [agentCall('c1', { subject: 'Map the lexer', subagent: 'lexer-scout' }), news('c1')]
  shown(entries, <RunningStrip labels={() => ({ title: 'Map the lexer', name: 'lexer-scout' })} />)
  const strip = screen.getByTestId('running-tasks')
  expect(within(strip).getByText('lexer-scout')).toBeTruthy()
  expect(within(strip).getByText('Map the lexer')).toBeTruthy()
})

test("a resumed subagent's card names it as its first call did, and doesn't take the message's result for its report", () => {
  const first = agentCall('c1', { subject: 'Build the hearth', detail: 'general-purpose', subagent: 'hearth' })
  const message = {
    ...agentCall('s1', { name: 'SendMessage', kind: 'other', subject: 'Bigger fire' }),
    index: 3,
    time_ms: 3000,
  }
  const resumed = { ...news('s1'), index: 4, time_ms: 4000, task: { ...news('s1').task!, id: 'task-c1' } }
  const result = {
    ...news('s1'),
    index: 5,
    kind: 'tool_result',
    text: '{"success":true,"message":"Resuming agent"}',
    task: null,
    outcome: { call: 's1', failed: false, exit_code: null, answers: [] },
  }
  const done = { ...resumed, index: 6, task: { ...resumed.task!, status: 'completed', summary: 'Fire is bigger' } }
  shown([first, news('c1'), message, resumed, done], <TaskCard step={{ call: message, result, images: [] }} />)
  const card = screen.getByTestId('task')
  expect(within(card).getByTestId('task-name').textContent).toBe('hearth')
  expect(within(card).getByText('general-purpose, resumed')).toBeTruthy()
  within(card).getByRole('button', { expanded: false }).click()
  expect(screen.queryByText('Reported back')).toBeNull()
})
