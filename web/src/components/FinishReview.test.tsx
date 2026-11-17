import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import type { Review, ReviewState } from '../review'
import { FinishReview } from './FinishReview'

afterEach(cleanup)

const THREAD = '0123456789abcdef0123456789abcdef'

function reviewOf(review: Review): ReviewState {
  return { review, add: vi.fn(), setAnchor: vi.fn(), edit: vi.fn(), remove: vi.fn(), setNote: vi.fn(), clear: vi.fn() }
}

const PENDING = reviewOf({ note: '', drafts: [{ id: '1', at: { path: 'src/a.rs', start: 2, end: 4 }, text: 'retry it', quote: ['+send(chunk);'] }] })

const props = { thread: THREAD, unsent: [], items: () => null, onSent: async () => undefined }

test("a working agent's review can wait for the turn or stop it", () => {
  render(<FinishReview {...props} review={PENDING} agent="Claude Agent" working />)
  expect(screen.getByRole('group', { name: 'review' })).toHaveTextContent('1 pending')
  fireEvent.click(screen.getByRole('button', { name: 'Finish review' }))
  const finish = screen.getByRole('dialog', { name: 'finish review' })
  expect(finish).toHaveTextContent('src/a.rs:2-4: retry it')
  expect(screen.getByRole('radio', { name: 'Request changes' })).toBeChecked()
  fireEvent.click(screen.getByRole('radio', { name: 'Request changes, stopping the turn' }))
  expect(screen.getByRole('button', { name: 'Request changes now' })).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Discard' })).toBeInTheDocument()
})

test('with no agent running here the review is copied, and an idle agent has no turn to stop', () => {
  render(<FinishReview {...props} review={PENDING} />)
  fireEvent.click(screen.getByRole('button', { name: 'Finish review' }))
  expect(screen.getByRole('radio', { name: "Copy for the agent's terminal" })).toBeChecked()
  expect(screen.getByRole('button', { name: 'Copy 1 comment' })).toBeInTheDocument()
  cleanup()
  render(<FinishReview {...props} review={PENDING} agent="Claude Agent" />)
  fireEvent.click(screen.getByRole('button', { name: 'Finish review' }))
  expect(screen.queryByRole('radio', { name: /stopping the turn/ })).toBeNull()
})

test('nothing pending, nothing to finish', () => {
  render(<FinishReview {...props} review={reviewOf({ note: '', drafts: [] })} agent="Claude Agent" />)
  expect(screen.queryByRole('button', { name: 'Finish review' })).toBeNull()
})

test('with nothing pending, the review can still be marked reviewed through a turn', () => {
  render(<FinishReview {...props} review={reviewOf({ note: '', drafts: [] })} agent="Claude Agent" reviewedTurn={3} />)
  fireEvent.click(screen.getByRole('button', { name: 'Finish review' }))
  expect(screen.getByRole('radio', { name: 'Mark reviewed through turn 3' })).toBeChecked()
  expect(screen.queryByRole('radio', { name: 'Request changes' })).toBeNull()
  expect(screen.getByRole('button', { name: 'Mark reviewed' })).toBeInTheDocument()
})

test("a draft made without other agents' changes is commented on as numbered there", async () => {
  const sent: unknown[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string, init?: RequestInit) => {
      if (path.endsWith('/comments')) sent.push(JSON.parse(String(init?.body)))
      return new Response(JSON.stringify({ id: 'c1' }))
    }),
  )
  const draft = { id: '1', at: { path: 'src/a.rs', start: 2, end: 4 }, text: 'retry it', quote: ['+send(chunk);'], hideOthers: true }
  render(<FinishReview {...props} review={reviewOf({ note: '', drafts: [draft] })} />)
  fireEvent.click(screen.getByRole('button', { name: 'Finish review' }))
  fireEvent.click(screen.getByRole('button', { name: 'Copy 1 comment' }))
  await waitFor(() => expect(sent).toHaveLength(1))
  expect(sent[0]).toMatchObject({ on: { on: 'lines', path: 'src/a.rs', start: 2, end: 4, hide_git: false, hide_others: true }, text: 'retry it' })
  vi.unstubAllGlobals()
})
