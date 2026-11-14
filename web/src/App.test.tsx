import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import App from './App'
import type { ThreadSummary } from './api'
import { legacyPath } from './legacy'
import { projectName, projects } from './projects'
import { queryClient } from './query'

afterEach(() => {
  cleanup()
  queryClient.clear()
  vi.unstubAllGlobals()
})

test('lists threads from the daemon', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) => {
      const body = path.startsWith('/api/threads')
        ? [
            {
              id: '0123456789abcdef0123456789abcdef',
              repo: 'github.com/u/r',
              title: 'Fix the lexer',
              agent: 'claude',
              created_ms: 1,
              messages: 2,
              edits: 3,
              location: 'here',
              forked_from: null,
              owner: '',
              collaborators: [],
              last_ms: Date.now(),
              merged: false,
              review: false,
              mine: { pinned: false, snoozed_until: null, snoozed_at: null, wake_on_activity: false, settled: false, seen_ms: null, reviewed_turn: null },
            },
          ]
        : []
      return new Response(JSON.stringify(body), { status: 200 })
    }),
  )
  render(<App />)
  // The sidebar's wordmark, and the phone header's, which a stylesheet hides on a desktop.
  expect(await screen.findAllByRole('link', { name: 'silt' })).not.toHaveLength(0)
  // With no clone, `/` is the first open's new thread, and the sidebar lists the thread (ADR 0044).
  expect(await screen.findByText('Pick a folder to start in')).toBeInTheDocument()
  await waitFor(() => expect(screen.getAllByText('Fix the lexer')).toHaveLength(1))
  const project = screen.getByRole('region', { name: 'github.com/u/r' })
  expect(project).toHaveTextContent('rFix the lexer')
})

test('a project with nothing from the last two weeks folds into one row, and opens a line at a time', async () => {
  const summary = (id: string, repo: string, title: string, last_ms: number) => ({
    id,
    repo,
    title,
    agent: 'claude',
    created_ms: last_ms,
    messages: 2,
    edits: 3,
    location: 'here',
    forked_from: null,
    owner: '',
    collaborators: [],
    last_ms,
    merged: false,
    review: false,
    mine: { pinned: false, snoozed_until: null, snoozed_at: null, wake_on_activity: false, settled: false, seen_ms: null, reviewed_turn: null },
  })
  const weeksAgo = (weeks: number) => Date.now() - weeks * 7 * 86_400_000
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) => {
      const body = path.startsWith('/api/threads')
        ? [
            summary('a'.repeat(32), 'github.com/u/recent', 'Fresh work', Date.now()),
            summary('b'.repeat(32), 'github.com/u/recent', 'Stale work', weeksAgo(3)),
            summary('c'.repeat(32), 'github.com/u/dusty', 'Long gone', weeksAgo(5)),
          ]
        : []
      return new Response(JSON.stringify(body), { status: 200 })
    }),
  )
  render(<App />)
  await waitFor(() => expect(screen.getByText('Fresh work')).toBeInTheDocument())
  expect(screen.queryByText('Stale work')).toBeNull()
  expect(screen.queryByRole('region', { name: 'github.com/u/dusty' })).toBeNull()

  fireEvent.click(screen.getByRole('button', { name: 'Show 1 older' }))
  expect(screen.getByText('Stale work')).toBeInTheDocument()

  fireEvent.click(screen.getByRole('button', { name: /^Older projects/ }))
  const dusty = screen.getByRole('region', { name: 'github.com/u/dusty' })
  expect(within(dusty).queryByText('Long gone')).toBeNull()
  fireEvent.click(within(dusty).getByRole('button', { name: /dusty/ }))
  expect(within(dusty).getByText('Long gone')).toBeInTheDocument()
  expect(within(dusty).queryByRole('button', { name: /^Show/ })).toBeNull()
})

test('threads are grouped by project, newest first', () => {
  const thread = (id: string, repo: string, created_ms: number): ThreadSummary => ({
    id,
    repo,
    created_ms,
    title: null,
    agent: null,
    messages: 0,
    edits: 0,
    location: 'here',
    forked_from: null,
    owner: '',
    collaborators: [],
    writable: true,
    agent_state: null,
    background: 0,
    last_ms: created_ms,
    merged: false,
    review: false,
    mine: { pinned: false, snoozed_until: null, snoozed_at: null, wake_on_activity: false, settled: false, seen_ms: null, reviewed_turn: null },
    visibility: null,
    server: null,
    local_only: false,
    workspace: null,
    cloud: null,
  })
  const grouped = projects([thread('a', 'github.com/u/b', 1), thread('b', '', 2), thread('c', 'github.com/u/b', 3)])
  expect(grouped.map(([repo, threads]) => [repo, threads.map((each) => each.id)])).toEqual([
    ['github.com/u/b', ['c', 'a']],
    ['no remote', ['b']],
  ])
  expect(projectName('github.com/u/silt')).toBe('silt')
})

test('old hash links move to paths', () => {
  expect(legacyPath('#/t/abc')).toBe('/t/abc')
  expect(legacyPath('#/mr/12ab.4')).toBe('/mr/12ab.4')
  expect(legacyPath('#/inbox')).toBe('/inbox')
  expect(legacyPath('#/')).toBe('/')
  expect(legacyPath('#token=1')).toBeUndefined()
})
