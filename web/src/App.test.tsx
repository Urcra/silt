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
              shared: false,
              location: 'here',
              forked_from: null,
              owner: '',
              collaborators: [],
              last_ms: Date.now(),
              merged: false,
              review: false,
              mine: {
                pinned: false,
                snoozed_until: null,
                snoozed_at: null,
                wake_on_activity: false,
                settled: false,
                seen_ms: null,
                reviewed_turn: null,
              },
            },
          ]
        : []
      return new Response(JSON.stringify(body), { status: 200 })
    }),
  )
  render(<App />)
  // The sidebar's title is the workspace it shows (ADR 0115).
  expect(await screen.findByRole('button', { name: /^workspace: / })).toBeInTheDocument()
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
    shared: false,
    location: 'here',
    forked_from: null,
    owner: '',
    collaborators: [],
    last_ms,
    merged: false,
    review: false,
    mine: {
      pinned: false,
      snoozed_until: null,
      snoozed_at: null,
      wake_on_activity: false,
      settled: false,
      seen_ms: null,
      reviewed_turn: null,
    },
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
    shared: false,
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
    mine: {
      pinned: false,
      snoozed_until: null,
      snoozed_at: null,
      wake_on_activity: false,
      settled: false,
      seen_ms: null,
      reviewed_turn: null,
    },
    visibility: null,
    server: null,
    local_only: false,
    workspace: null,
    cloud: null,
    machine: null,
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

test('the sidebar names the workspace at its top, and at its bottom the machine, how it stands, and Settings', async () => {
  const summary = (id: string, title: string, overrides: Partial<ThreadSummary>) => ({
    id,
    repo: 'github.com/u/r',
    title,
    agent: 'claude',
    created_ms: 1,
    messages: 2,
    edits: 0,
    shared: false,
    location: 'here',
    forked_from: null,
    owner: '',
    collaborators: [],
    last_ms: Date.now(),
    merged: false,
    review: false,
    mine: {
      pinned: false,
      snoozed_until: null,
      snoozed_at: null,
      wake_on_activity: false,
      settled: false,
      seen_ms: null,
      reviewed_turn: null,
    },
    machine: null,
    ...overrides,
  })
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) => {
      const body = path.startsWith('/api/threads')
        ? [
            summary('0123456789abcdef0123456789abcdef', 'Fix the lexer', { agent_state: 'working' }),
            summary('fedcba9876543210fedcba9876543210', 'Tidy the parser', { location: 'remote', machine: 'homelab' }),
          ]
        : path.startsWith('/api/status')
          ? {
              servers: [{ url: 'https://silt.example.dev', user: 'ada', device: 'd1', name: 'laptop', sync_error: null }],
              sync_error: null,
            }
          : []
      return new Response(JSON.stringify(body), { status: 200 })
    }),
  )
  render(<App />)
  const workspace = await screen.findByRole('button', { name: 'workspace: everything' })
  await waitFor(() => expect(workspace).toHaveTextContent('Everythingevery server and this machine'))
  const machine = await screen.findByRole('button', { name: 'machine: This computer' })
  await waitFor(() => expect(machine).toHaveTextContent('This computerSynced · 1 running'))
  expect(screen.getByRole('link', { name: 'Settings' })).toHaveAttribute('href', '/settings')
  await waitFor(() => expect(screen.getByText('Tidy the parser').parentElement).toHaveTextContent(/On homelab/))

  fireEvent.click(machine)
  const menu = screen.getByRole('dialog', { name: 'Switch machine' })
  expect(within(menu).getByRole('button', { name: /This computer/ })).toHaveTextContent('laptop · 1 running')
  expect(within(menu).queryByRole('button', { name: 'Add a machine' })).toBeNull()
  expect(within(menu).getByRole('button', { name: /Devices and sync/ })).toHaveTextContent('silt.example.dev')
})

test('All machines lists every connected machine’s threads, marked with their machine, and the menu says how each stands', async () => {
  const summary = (id: string, title: string, overrides: Partial<ThreadSummary>) => ({
    id,
    repo: 'github.com/u/r',
    title,
    agent: 'claude',
    created_ms: 1,
    messages: 2,
    edits: 0,
    shared: false,
    location: 'here',
    forked_from: null,
    owner: '',
    collaborators: [],
    last_ms: Date.now(),
    merged: false,
    review: false,
    mine: {
      pinned: false,
      snoozed_until: null,
      snoozed_at: null,
      wake_on_activity: false,
      settled: false,
      seen_ms: null,
      reviewed_turn: null,
    },
    machine: null,
    ...overrides,
  })
  window.__siltBackends = [
    { id: 'native', label: 'This computer', kind: 'native', colour: 'ink' },
    { id: 'ssh:nuc', label: 'nuc', kind: 'ssh', state: 'connected', colour: 'teal' },
    { id: 'ssh:vps', label: 'vps', kind: 'ssh', state: 'idle', colour: 'plum' },
  ]
  window.__siltBackend = 'native'
  window.__siltDesktop = { commit: 'abc', key: 'k', view: { list: 'all', mark: 'chip', markHere: false } }
  window.__siltThreads = [
    {
      id: 'ssh:nuc',
      state: 'ready',
      threads: [
        summary('1111111111111111', 'Faster replica load', { agent_state: 'working' }),
        summary('2222222222222222', 'Retry the TLS check', { agent_state: 'awaiting_permission' }),
      ] as ThreadSummary[],
    },
  ]
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) => {
      const body = path.startsWith('/api/threads')
        ? [summary('0123456789abcdef0123456789abcdef', 'Fix the lexer', {})]
        : path.startsWith('/api/status')
          ? { servers: [], sync_error: null }
          : []
      return new Response(JSON.stringify(body), { status: 200 })
    }),
  )
  try {
    render(<App />)
    const project = await screen.findByRole('region', { name: 'github.com/u/r' })
    await waitFor(() => expect(within(project).getByText('Faster replica load')).toBeInTheDocument())
    expect(within(project).getByText('Fix the lexer')).toBeInTheDocument()
    const there = within(project).getByRole('link', { name: /Faster replica load/ })
    expect(there).toHaveTextContent('nucWorking')
    expect(there).toHaveAccessibleName(/on nuc/)
    expect(within(project).getByRole('link', { name: /Fix the lexer/ })).not.toHaveTextContent('nuc')
    // Needs you gathers from every machine.
    const waiting = screen.getByRole('region', { name: 'needs you' })
    expect(within(waiting).getByText('Retry the TLS check')).toBeInTheDocument()

    const machine = screen.getByRole('button', { name: 'machine: All machines' })
    expect(machine).toHaveTextContent('All machines2 connected · showing This computer')
    fireEvent.click(machine)
    const menu = screen.getByRole('dialog', { name: 'Switch machine' })
    expect(within(menu).getByRole('button', { name: /^All machines/ })).toHaveTextContent('2 connected · 1 needs you · 1 running')
    expect(within(menu).getByRole('button', { name: /^nuc/ })).toHaveTextContent('over SSH · 1 needs you · 1 running')
    expect(within(menu).getByRole('button', { name: /^vps/ })).toHaveTextContent('over SSH · not connected')
  } finally {
    delete window.__siltBackends
    delete window.__siltBackend
    delete window.__siltDesktop
    delete window.__siltThreads
  }
})

test('in All machines, New thread asks which machine to start on, and picking another carries the draft and project there', async () => {
  window.__siltBackends = [
    { id: 'native', label: 'This computer', kind: 'native', colour: 'ink' },
    { id: 'ssh:nuc', label: 'nuc', kind: 'ssh', state: 'connected', colour: 'teal' },
  ]
  window.__siltBackend = 'native'
  window.__siltDesktop = { commit: 'abc', key: 'k', view: { list: 'all', mark: 'chip', markHere: false } }
  const went: string[] = []
  const location = window.location
  vi.stubGlobal('location', {
    ...location,
    pathname: '/',
    get href() {
      return location.href
    },
    set href(to: string) {
      went.push(to)
    },
  })
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) => {
      const body = path.startsWith('/api/agents')
        ? { agents: [], clones: [{ repo: 'github.com/u/silt', path: '/home/me/silt' }] }
        : path.startsWith('/api/status')
          ? { servers: [], sync_error: null }
          : []
      return new Response(JSON.stringify(body), { status: 200 })
    }),
  )
  try {
    render(<App />)
    const picker = await screen.findByRole('button', { name: 'machine: This computer', hidden: false })
    fireEvent.change(screen.getByLabelText('first prompt'), { target: { value: 'Fix the lexer' } })
    fireEvent.click(picker)
    fireEvent.click(within(screen.getByRole('dialog', { name: 'Pick a machine' })).getByRole('menuitemradio', { name: /nuc/ }))
    await waitFor(() =>
      expect(went).toContain('silt://open/?backend=ssh%3Anuc&new=1&origin=github.com%2Fu%2Fsilt&draft=Fix+the+lexer'),
    )
  } finally {
    delete window.__siltBackends
    delete window.__siltBackend
    delete window.__siltDesktop
  }
})

test('a machine that stops answering leaves the list, and the foot says it’s lost while the app reconnects', async () => {
  window.__siltBackends = [
    { id: 'native', label: 'This computer', kind: 'native', colour: 'ink' },
    { id: 'ssh:nuc', label: 'nuc', kind: 'ssh', state: 'connecting', colour: 'teal' },
  ]
  window.__siltBackend = 'native'
  window.__siltDesktop = { commit: 'abc', key: 'k', view: { list: 'all', mark: 'chip', markHere: false } }
  window.__siltThreads = [{ id: 'ssh:nuc', state: 'lost', threads: [] }]
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) =>
      path.startsWith('/api/status')
        ? new Response(JSON.stringify({ servers: [], sync_error: null }), { status: 200 })
        : new Response('[]', { status: 200 }),
    ),
  )
  try {
    render(<App />)
    const machine = await screen.findByRole('button', { name: 'machine: All machines' })
    expect(machine).toHaveTextContent('1 connected · lost nuc')
    fireEvent.click(machine)
    expect(
      within(screen.getByRole('dialog', { name: 'Switch machine' })).getByRole('button', { name: /^nuc/ }),
    ).toHaveTextContent('over SSH · lost')
  } finally {
    delete window.__siltBackends
    delete window.__siltBackend
    delete window.__siltDesktop
    delete window.__siltThreads
  }
})
