import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import type { SignInAsked } from '../api'
import { SignInCard } from './Agent'

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

const THREAD = '0123456789abcdef0123456789abcdef'

function posts() {
  const sent: [string, unknown][] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string, init?: RequestInit) => {
      sent.push([path, init?.body ? JSON.parse(String(init.body)) : null])
      return new Response(JSON.stringify('ok'), { status: 200 })
    }),
  )
  return sent
}

const methods: SignInAsked['methods'] = [
  { id: 'claude-ai-login', name: 'Claude Subscription', description: 'Use Claude subscription', by_agent: false },
  { id: 'console-login', name: 'Anthropic Console', description: null, by_agent: false },
]

test('offers each way the agent lists to sign in, and says why the last one failed', async () => {
  const sent = posts()
  render(
    <SignInCard
      thread={THREAD}
      agent="Claude"
      asked={{ methods, running: null, failed: 'the sign-in ended without signing Claude in: Invalid code' }}
    />,
  )
  expect(screen.getByText('Claude needs you to sign in')).toBeInTheDocument()
  expect(screen.getByText(/Invalid code/)).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Anthropic Console' }))
  await waitFor(() => expect(sent).toContainEqual([`/api/threads/${THREAD}/agent/sign-in`, { method: 'console-login' }]))
})

test("shows the sign-in's page and takes the code it gives back", async () => {
  const sent = posts()
  const running = {
    method: 'claude-ai-login',
    link: 'https://claude.com/cai/oauth/authorize?state=x',
    code: null,
    asks_code: true,
    code_sent: false,
    tail: 'Paste code here if prompted >',
  }
  render(<SignInCard thread={THREAD} agent="Claude" asked={{ methods, running, failed: null }} />)
  const page = screen.getByRole('link', { name: 'Open claude.com' })
  expect(page).toHaveAttribute('href', running.link)
  expect(page).toHaveAttribute('target', '_blank')
  fireEvent.change(screen.getByLabelText('code from the sign-in page'), { target: { value: '  abc#123 ' } })
  fireEvent.click(screen.getByRole('button', { name: 'Sign in' }))
  await waitFor(() => expect(sent).toContainEqual([`/api/threads/${THREAD}/agent/sign-in/code`, { code: 'abc#123' }]))
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
  await waitFor(() => expect(sent.map(([path]) => path)).toContain(`/api/threads/${THREAD}/agent/sign-in/cancel`))
})

test("shows a device flow's code to enter, and offers no link but https", () => {
  posts()
  const device = {
    method: 'copilot-login',
    link: 'https://github.com/login/device',
    code: 'WDJB-MJH7',
    asks_code: false,
    code_sent: false,
    tail: '',
  }
  const { rerender } = render(<SignInCard thread={THREAD} agent="Copilot" asked={{ methods, running: device, failed: null }} />)
  expect(screen.getByTestId('sign-in-code')).toHaveTextContent('WDJB-MJH7')
  expect(screen.getByText('Waiting for the sign-in to finish')).toBeInTheDocument()
  expect(screen.queryByLabelText('code from the sign-in page')).not.toBeInTheDocument()
  rerender(
    <SignInCard
      thread={THREAD}
      agent="Copilot"
      asked={{ methods, running: { ...device, link: 'javascript:alert(1)' }, failed: null }}
    />,
  )
  expect(screen.queryByRole('link')).not.toBeInTheDocument()
})
