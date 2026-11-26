import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import type { NetworkSettings, NetworkView } from '../api'
import { NetworkSettingsSection } from './Network'

const DEFAULTS: NetworkSettings = { offline: false, update_checks: true, plan_usage: true, agent_registry: true }

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

/** Serves the section's reads from `view`, and records what it saves. */
function serve(view: NetworkView | { error: string }, proxy = true) {
  const saved: unknown[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string, init?: RequestInit) => {
      const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status })
      if (url.endsWith('/api/network-settings') && init?.method === 'PUT') {
        saved.push(JSON.parse(String(init.body)))
        return json('saved')
      }
      if (url.endsWith('/api/network-settings')) return 'error' in view ? json(view, 400) : json(view)
      if (url.endsWith('/api/artifact-settings')) return json({ proxy })
      if (url.endsWith('/api/status')) return json({ servers: [{ url: 'https://silt.acme.dev', user: 'me' }] })
      return json({}, 404)
    }),
  )
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <NetworkSettingsSection />
    </QueryClientProvider>,
  )
  return saved
}

const view = (settings: Partial<NetworkSettings>, env: Partial<NetworkView> = {}): NetworkView => {
  const all = { ...DEFAULTS, ...settings }
  return { settings: all, offline_env: false, no_update_checks_env: false, offline: all.offline, ...env }
}

test('each switch saves its own setting, keeping the others', async () => {
  const saved = serve(view({}))
  const usage = await screen.findByRole('switch', { name: 'Plan usage' })
  expect(usage).toBeChecked()
  fireEvent.click(usage)
  await vi.waitFor(() => expect(saved).toEqual([{ ...DEFAULTS, plan_usage: false }]))
  expect(screen.getByText('silt.acme.dev')).toBeInTheDocument()
})

test('offline turns every switch off without changing them, and pages keep to what Silt kept', async () => {
  const saved = serve(view({ offline: true, plan_usage: false }), false)
  const connections = within(await screen.findByRole('region', { name: 'connections' }))
  for (const name of ['Check for updates', 'Plan usage', 'Agent registry']) {
    const toggle = connections.getByRole('switch', { name })
    expect(toggle).not.toBeChecked()
    expect(toggle).toBeDisabled()
  }
  expect(connections.getByRole('switch', { name: "Load artifacts' libraries and fonts through Silt" })).toBeChecked()
  expect(connections.getByText('Offline: pages get only the files Silt already kept.')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('switch', { name: 'Offline' }))
  await vi.waitFor(() => expect(saved).toEqual([{ ...DEFAULTS, plan_usage: false }]))
})

test('the environment locks its switch and says so', async () => {
  serve(view({}, { offline: true, offline_env: true, no_update_checks_env: true }))
  expect(await screen.findByRole('switch', { name: 'Offline' })).toBeDisabled()
  expect(screen.getByText('SILT_OFFLINE=1')).toBeInTheDocument()
})

test('update checks turned off by the environment are locked off', async () => {
  serve(view({}, { no_update_checks_env: true }))
  const updates = await screen.findByRole('switch', { name: 'Check for updates' })
  expect(updates).not.toBeChecked()
  expect(updates).toBeDisabled()
  expect(screen.getByText('SILT_UPDATE_CHECK=never')).toBeInTheDocument()
})

test('a settings file that cannot be read says so and offers a retry', async () => {
  serve({ error: "network.json isn't valid: EOF while parsing a value at line 1 column 15" })
  expect(await screen.findByRole('alert')).toHaveTextContent("network.json isn't valid")
  expect(screen.getByRole('alert')).toHaveTextContent('Silt stays offline')
  expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument()
})
