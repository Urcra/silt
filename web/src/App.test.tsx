import { render, screen } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import App from './App'
import type { ThreadSummary } from './api'
import { lineClass } from './diffStyle'
import { legacyPath } from './legacy'
import { projectName, projects } from './projects'

afterEach(() => {
  vi.unstubAllGlobals()
})

test('lists threads from the daemon', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) => {
      const body = path.startsWith('/api/threads')
        ? [{ id: '0123456789abcdef0123456789abcdef', repo: 'github.com/u/r', title: 'Fix the lexer', agent: 'claude', created_ms: 1, messages: 2, edits: 3, location: 'here', forked_from: null }]
        : []
      return new Response(JSON.stringify(body), { status: 200 })
    }),
  )
  render(<App />)
  expect(await screen.findByRole('link', { name: 'Silt' })).toBeInTheDocument()
  expect(await screen.findAllByText('Fix the lexer')).toHaveLength(2)
  expect(screen.getByText('456789abcdef')).toBeInTheDocument()
  const project = screen.getByRole('region', { name: 'github.com/u/r' })
  expect(project).toHaveTextContent('r1Fix the lexer')
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
  })
  const grouped = projects([thread('a', 'github.com/u/b', 1), thread('b', '', 2), thread('c', 'github.com/u/b', 3)])
  expect(grouped.map(([repo, threads]) => [repo, threads.map((each) => each.id)])).toEqual([
    ['github.com/u/b', ['c', 'a']],
    ['no remote', ['b']],
  ])
  expect(projectName('github.com/u/silt')).toBe('silt')
})

test('old hash links move to paths, and diff lines are understood', () => {
  expect(legacyPath('#/t/abc')).toBe('/t/abc')
  expect(legacyPath('#/mr/12ab.4')).toBe('/mr/12ab.4')
  expect(legacyPath('#/inbox')).toBe('/inbox')
  expect(legacyPath('#/')).toBe('/')
  expect(legacyPath('#token=1')).toBeUndefined()
  expect(lineClass('+added')).toContain('ok-tint')
  expect(lineClass('-removed')).toContain('danger-tint')
  expect(lineClass('+++ b/x')).toContain('fg-3')
})
