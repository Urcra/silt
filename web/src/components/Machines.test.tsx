import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { type Backend, type MachineTest, switchBackend } from '../desktop'
import { machineProblem } from '../machines'
import { MachinesSettings } from './Machines'

vi.mock('../desktop', async (original) => ({ ...(await original<typeof import('../desktop')>()), switchBackend: vi.fn() }))

const THIS: Backend = { id: 'native', label: 'This computer', kind: 'native' }
const VPS: Backend = { id: 'ssh:vps', label: 'vps', kind: 'ssh', destination: 'me@vps.example.com', port: null, state: 'idle' }

beforeEach(() => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response('{}', { status: 503 })),
  )
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  delete window.__siltBackends
  delete window.__siltBackend
})

function show(backends: Backend[], current: string) {
  window.__siltBackends = backends
  window.__siltBackend = current
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MachinesSettings />
    </QueryClientProvider>,
  )
}

function report(test: MachineTest) {
  act(() => {
    window.dispatchEvent(new CustomEvent('silt-machine-test', { detail: test }))
  })
}

test('adding a machine checks the form, then shows each step of its test, and what to do when one fails', () => {
  show([THIS], 'native')
  fireEvent.click(screen.getByRole('button', { name: 'Add a machine' }))
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'vps' } })
  fireEvent.change(screen.getByLabelText('Destination'), { target: { value: '-oProxyCommand=evil' } })
  fireEvent.click(screen.getByRole('button', { name: 'Test and add' }))
  expect(screen.getByRole('alert')).toHaveTextContent('Write the destination as user@host')

  fireEvent.change(screen.getByLabelText('Destination'), { target: { value: 'me@vps.example.com' } })
  fireEvent.click(screen.getByRole('button', { name: 'Test and add' }))
  expect(screen.getByRole('button', { name: 'Testing…' })).toBeDisabled()

  const fields = { name: 'vps', destination: 'me@vps.example.com', port: null }
  report({
    ...fields,
    steps: [
      { state: 'done', text: 'Signed in to `vps.example.com` as `urcra`' },
      { state: 'failed', text: '`silt` isn’t installed on `vps.example.com`.' },
    ],
    done: true,
    ok: false,
  })
  const steps = screen.getByRole('list', { name: 'testing the connection' })
  expect(within(steps).getAllByRole('listitem')).toHaveLength(2)
  expect(within(steps).getByText('silt').tagName).toBe('CODE')
  expect(screen.getByRole('button', { name: 'Try again' })).toBeEnabled()
  expect(screen.getByRole('button', { name: 'Add anyway' })).toBeEnabled()

  fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
  report({ ...fields, steps: [{ state: 'done', text: 'Forwarded' }], done: true, ok: true })
  expect(screen.queryByRole('region', { name: 'Add a machine' })).toBeNull()
})

test('a machine that couldn’t connect says why, and offers to retry or edit it', () => {
  show([THIS, { ...VPS, state: 'failed', reason: 'Run `ssh me@vps.example.com` once, then retry.' }], 'native')
  const row = screen.getByRole('region', { name: 'vps' })
  expect(within(row).getByText('couldn’t connect')).toBeInTheDocument()
  expect(within(row).getByText('ssh me@vps.example.com').tagName).toBe('CODE')
  expect(within(row).getByRole('button', { name: 'Retry' })).toBeInTheDocument()
  fireEvent.click(within(row).getByRole('button', { name: 'Edit' }))
  expect(screen.getByLabelText('Destination')).toHaveValue('me@vps.example.com')
})

test('a machine’s own page lists the machines but can’t change them, though it can retry opening one', () => {
  const down: Backend = { ...VPS, id: 'ssh:down', label: 'down', state: 'failed', reason: 'Couldn’t reach it.' }
  show([THIS, { ...VPS, state: 'connected' }, down], 'ssh:vps')
  expect(screen.getByRole('status')).toHaveTextContent('This is vps’s Silt, shown over SSH')
  expect(screen.queryByRole('button', { name: /options/ })).toBeNull()
  expect(screen.queryByRole('button', { name: 'Add a machine' })).toBeNull()
  expect(within(screen.getByRole('region', { name: 'This computer' })).getByRole('button', { name: 'Open' })).toBeInTheDocument()
  const row = screen.getByRole('region', { name: 'down' })
  expect(within(row).queryByRole('button', { name: 'Edit' })).toBeNull()
  fireEvent.click(within(row).getByRole('button', { name: 'Retry' }))
  expect(switchBackend).toHaveBeenCalledWith('ssh:down')
})

test('the form refuses what the desktop app would', () => {
  const ok = { name: 'vps', destination: 'me@vps.example.com', port: '' }
  expect(machineProblem(ok, [])).toBeUndefined()
  expect(machineProblem({ ...ok, name: 'home gpü' }, [])).toBeUndefined()
  expect(machineProblem(ok, ['vps'])).toBe('Another machine has this name')
  expect(machineProblem({ ...ok, name: 'a/b' }, [])).toMatch(/letters, digits/)
  expect(machineProblem({ ...ok, destination: 'urcra@-oProxyCommand=x' }, [])).toMatch(/user@host/)
  expect(machineProblem({ ...ok, destination: 'a b' }, [])).toMatch(/user@host/)
  expect(machineProblem({ ...ok, port: '0x16' }, [])).toMatch(/port/)
  expect(machineProblem({ ...ok, port: '2222' }, [])).toBeUndefined()
})
