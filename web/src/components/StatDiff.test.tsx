import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import type { DiffStat } from '../api'
import { FilesByStats } from './StatDiff'

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

const THREAD = '0123456789abcdef0123456789abcdef'

test('a diff too large to read whole lists its files and reads only the one opened', async () => {
  const asked: string[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) => {
      asked.push(path)
      return new Response(JSON.stringify('diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-old\n+fn opened() {}\n'))
    }),
  )
  const stats: DiffStat[] = Array.from({ length: 450 }, (_, index) => ({
    path: index === 0 ? 'src/a.rs' : `.venv/lib/file${String(index).padStart(3, '0')}.py`,
    old_path: null,
    added: 3,
    removed: 0,
    binary: false,
    size: 2048,
  }))
  render(
    <QueryClientProvider client={new QueryClient()}>
      <FilesByStats thread={THREAD} stats={stats} of={{ base: '' }} />
    </QueryClientProvider>,
  )
  expect(screen.getByRole('note')).toHaveTextContent('450 files, 900 KB of changes')
  expect(screen.getAllByRole('button', { name: /^open / })).toHaveLength(200)
  fireEvent.click(screen.getByRole('button', { name: /More files \(250 left\)/ }))
  expect(screen.getAllByRole('button', { name: /^open / })).toHaveLength(400)
  expect(asked).toEqual([])

  fireEvent.click(screen.getByRole('button', { name: 'open src/a.rs' }))
  await waitFor(() => expect(screen.getByText('fn opened() {}')).toBeInTheDocument(), { timeout: 10_000 })
  expect(asked).toEqual([`/api/threads/${THREAD}/diff?path=src%2Fa.rs`])
  // Querying 400 buttons by role, and drawing the opened file, take jsdom
  // seconds under the whole suite's load.
}, 20_000)
