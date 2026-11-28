import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { type AgentUsage, api } from '../api'
import { UsageRing } from './UsageRing'

// A morning, so a reset an hour on is the same day, worded "Resets at", whenever the test runs.
const MORNING = new Date(2026, 10, 17, 9, 0).getTime()

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(MORNING)
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  vi.useRealTimers()
})

const MAX: AgentUsage = {
  agent: 'Claude Code',
  plan: 'Max',
  limits: [
    { name: '5-hour limit', used_percent: 32, resets_at_ms: MORNING + 3_600_000 },
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

test('antigravity asks for plan usage and shows its quota buckets', async () => {
  const usage = vi.spyOn(api, 'usage').mockResolvedValue({
    agent: 'Antigravity',
    plan: 'Google AI Pro',
    limits: [
      { name: 'Gemini · 5-hour', used_percent: 10, resets_at_ms: MORNING + 3_600_000 },
      { name: 'Gemini · Weekly', used_percent: 20, resets_at_ms: null },
    ],
    page: 'https://antigravity.google/docs/plans',
    unavailable: null,
    recorded: false,
    read_ms: 0,
  })
  shown('antigravity-acp', 'def')
  const ring = await screen.findByRole('button', { name: 'plan usage: 20% of the fullest limit used' })
  expect(usage).toHaveBeenCalledWith('antigravity-acp', 'def')
  fireEvent.click(ring)
  expect(screen.getByText('Plan usage limits · Google AI Pro')).toBeInTheDocument()
  expect(screen.getByRole('meter', { name: 'Gemini · 5-hour' })).toHaveAttribute('aria-valuenow', '10')
  expect(screen.getByRole('meter', { name: 'Gemini · Weekly' })).toHaveAttribute('aria-valuenow', '20')
})
