import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import { type AgentUsage, api } from '../api'
import { UsageRing } from './UsageRing'

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const MAX: AgentUsage = {
  agent: 'Claude Code',
  plan: 'Max',
  limits: [
    { name: '5-hour limit', used_percent: 32, resets_at_ms: Date.now() + 3_600_000 },
    { name: 'Weekly · all models', used_percent: 6.4, resets_at_ms: null },
  ],
  page: 'https://claude.ai/settings/usage',
  unavailable: null,
  recorded: false,
  read_ms: 0,
}

function shown(agent: string, thread?: string) {
  const queries = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={queries}>
      <UsageRing agent={agent} thread={thread} />
    </QueryClientProvider>,
  )
}

test("the ring fills with the fullest limit and opens on each limit's use", async () => {
  const usage = vi.spyOn(api, 'usage').mockResolvedValue(MAX)
  shown('claude-acp', 'abc')
  const ring = await screen.findByRole('button', { name: 'plan usage: 32% of the fullest limit used' })
  expect(usage).toHaveBeenCalledWith('claude-acp', 'abc')
  fireEvent.click(ring)
  expect(screen.getByText('Plan usage limits · Max')).toBeInTheDocument()
  expect(screen.getByRole('meter', { name: '5-hour limit' })).toHaveAttribute('aria-valuenow', '32')
  expect(screen.getByRole('meter', { name: 'Weekly · all models' })).toHaveAttribute('aria-valuenow', '6')
  expect(screen.getByText(/Resets at .* 32%/)).toBeInTheDocument()
  expect(screen.getByRole('link', { name: 'See all →' })).toHaveAttribute('href', 'https://claude.ai/settings/usage')
})

test('an agent with no plan limits to read shows nothing, and others are never asked', async () => {
  const usage = vi
    .spyOn(api, 'usage')
    .mockResolvedValue({ ...MAX, plan: null, limits: [], unavailable: 'Claude Code uses an API key here' })
  shown('claude-code')
  await vi.waitFor(() => expect(usage).toHaveBeenCalled())
  expect(screen.queryByRole('button')).toBeNull()
  usage.mockClear()
  cleanup()
  shown('gemini-cli')
  expect(usage).not.toHaveBeenCalled()
  expect(screen.queryByRole('button')).toBeNull()
})
