import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import type { SeasonalShow } from '../api'
import { SeasonalHat, SeasonalLights, SnowDrift } from './Seasonal'

function decorations(show: SeasonalShow) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () =>
      Response.json({
        themes: [],
        problems: [],
        appearance: {
          mode: 'system',
          light: 'nous-light',
          dark: 'nous-dark',
          text: {},
          seasonal: { show, snow: true },
        },
      }),
    ),
  )
  render(
    <QueryClientProvider client={new QueryClient()}>
      <header className="relative">
        <SeasonalHat />
        <SeasonalLights />
      </header>
      <div className="relative">
        <SnowDrift />
      </div>
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(new Date(2026, 11, 24, 12))
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.unstubAllGlobals()
  delete window.__siltDesktop
})

test('in December the desktop app hangs lights, puts on the hat and lets snow settle, hidden from screen readers', async () => {
  window.__siltDesktop = {}
  decorations('auto')
  for (const id of ['seasonal-lights', 'seasonal-hat', 'seasonal-drift']) {
    const shown = await screen.findByTestId(id)
    expect(shown.closest('[aria-hidden="true"]')).not.toBeNull()
  }
})

test('a browser never shows them, nor the desktop app when they are off', async () => {
  decorations('always')
  await new Promise((done) => setTimeout(done, 20))
  expect(screen.queryByTestId('seasonal-lights')).toBeNull()
  cleanup()
  window.__siltDesktop = {}
  decorations('off')
  await new Promise((done) => setTimeout(done, 20))
  expect(screen.queryByTestId('seasonal-hat')).toBeNull()
})
