import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, screen } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, expect, test } from 'vitest'
import type { AutoView, HistoryEntry } from '../api'
import { LiveQuestions } from '../liveQuestions'
import { piecesOf, stepsOf } from '../transcript'
import { CommandCard } from './ToolSteps'

afterEach(cleanup)

const call = (id: string, command: string): HistoryEntry =>
  ({
    index: 1,
    thread: 't',
    origin: 'own',
    kind: 'tool_call',
    text: `Run ${command}`,
    time_ms: 1000,
    rewound: false,
    side: null,
    task: null,
    auto: null,
    tool: { id, name: 'Run', kind: 'execute', subject: command, detail: null, subagent: null, questions: [] },
  }) as unknown as HistoryEntry

const verdict = (on: string, auto: Partial<AutoView>, index = 2): HistoryEntry =>
  ({
    index,
    thread: 't',
    origin: 'own',
    kind: 'auto',
    text: auto.reason ?? '',
    time_ms: 2000,
    rewound: false,
    side: null,
    task: null,
    auto: {
      call: on,
      verdict: 'allow',
      reason: 'Runs the tests.',
      judge: 'GitHub Copilot on Claude Haiku 5.5',
      ms: 2100,
      ...auto,
    },
  }) as unknown as HistoryEntry

function shown(children: ReactNode, checking: string[] = []) {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <LiveQuestions.Provider value={{ thread: 't', agent: 'GitHub Copilot', questions: [], awaiting: [], checking }}>
        {children}
      </LiveQuestions.Provider>
    </QueryClientProvider>,
  )
}

test("Auto's verdict shows on the call it's on, not as an entry of its own", () => {
  const entries = [call('c1', 'cargo test'), verdict('c1', { verdict: 'refuse', reason: 'No.' })]
  const steps = stepsOf(entries)
  expect(steps).toHaveLength(1)
  expect(steps[0]!.auto?.verdict).toBe('refuse')
  const pieces = piecesOf(entries)
  expect(pieces.some((piece) => piece.kind === 'entry' && piece.entry.kind === 'auto')).toBe(false)
  // Refused, it never ran: it stays in its run, rather than standing as a failed command does.
  const failed = {
    ...entries[1]!,
    kind: 'tool_result',
    auto: null,
    outcome: { call: 'c1', failed: true, exit_code: null, answers: [] },
  }
  const run = piecesOf([entries[0]!, failed as HistoryEntry, entries[1]!])
  expect(run.map((piece) => piece.kind)).toEqual(['tools'])
  // In a run's fold, whose rows are made from its own entries, the verdict goes along with its call.
  const allowed = [call('c2', 'cargo test'), verdict('c2', {}, 3)]
  const fold = piecesOf(allowed).find((piece) => piece.kind === 'tools')
  expect(fold?.kind === 'tools' && fold.entries.map((entry) => entry.kind)).toEqual(['tool_call', 'auto'])
})

test('a refused command says why under its row, and that the agent was told', () => {
  const entries = [call('c1', 'rm -rf ~/.cache'), verdict('c1', { verdict: 'refuse', reason: 'Deletes a shared cache.' })]
  shown(<CommandCard step={stepsOf(entries)[0]!} />)
  expect(screen.getByText('Auto refused')).toBeTruthy()
  expect(screen.getByRole('img', { name: 'refused' })).toBeTruthy()
  expect(screen.getByTestId('refused').textContent).toBe('Deletes a shared cache. Sent back to the agent.')
  expect(screen.queryByText('running')).toBeNull()
})

test('an allowed command carries its tag, and a call being checked says so', () => {
  const entries = [call('c1', 'cargo test'), verdict('c1', {})]
  shown(<CommandCard step={stepsOf(entries)[0]!} />)
  expect(screen.getByText('Auto allowed')).toBeTruthy()
  expect(screen.queryByTestId('refused')).toBeNull()
  cleanup()
  shown(<CommandCard step={{ call: call('c2', 'npm publish'), result: null, images: [] }} running />, ['c2'])
  expect(screen.getByText('Auto is checking')).toBeTruthy()
})
