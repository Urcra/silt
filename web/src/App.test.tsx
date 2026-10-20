import { render, screen } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import App from './App'
import { lineClass } from './diffStyle'
import { parseRoute } from './router'

afterEach(() => {
  vi.unstubAllGlobals()
  window.location.hash = ''
})

test('lists threads from the daemon', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) => {
      const body = path.startsWith('/api/threads')
        ? [{ id: '0123456789abcdef0123456789abcdef', repo: 'github.com/u/r', title: 'Fix the lexer', created_ms: 1, messages: 2, edits: 3, location: 'here', forked_from: null }]
        : []
      return new Response(JSON.stringify(body), { status: 200 })
    }),
  )
  render(<App />)
  expect(screen.getByRole('heading', { name: 'Silt' })).toBeInTheDocument()
  expect(await screen.findByText('Fix the lexer')).toBeInTheDocument()
  expect(screen.getByText('456789abcdef')).toBeInTheDocument()
})

test('routes and diff lines are understood', () => {
  expect(parseRoute('#/t/abc')).toEqual({ page: 'thread', id: 'abc' })
  expect(parseRoute('#/mr/12ab.4')).toEqual({ page: 'mr', id: '12ab.4' })
  expect(parseRoute('#token=1')).toEqual({ page: 'threads' })
  expect(lineClass('+added')).toContain('emerald')
  expect(lineClass('+++ b/x')).toContain('neutral-500')
})
