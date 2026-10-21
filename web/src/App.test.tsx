import { render, screen } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import App from './App'
import { lineClass } from './diffStyle'
import { legacyPath } from './legacy'

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
  expect(await screen.findByRole('heading', { name: 'Silt' })).toBeInTheDocument()
  expect(await screen.findByText('Fix the lexer')).toBeInTheDocument()
  expect(screen.getByText('456789abcdef')).toBeInTheDocument()
})

test('old hash links move to paths, and diff lines are understood', () => {
  expect(legacyPath('#/t/abc')).toBe('/t/abc')
  expect(legacyPath('#/mr/12ab.4')).toBe('/mr/12ab.4')
  expect(legacyPath('#/inbox')).toBe('/inbox')
  expect(legacyPath('#/')).toBe('/')
  expect(legacyPath('#token=1')).toBeUndefined()
  expect(lineClass('+added')).toContain('emerald')
  expect(lineClass('+++ b/x')).toContain('neutral-500')
})
