import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import type { Review, ReviewState } from '../review'
import { FinishReview } from './FinishReview'

afterEach(cleanup)

const THREAD = '0123456789abcdef0123456789abcdef'

function reviewOf(review: Review): ReviewState {
  return { review, add: vi.fn(), edit: vi.fn(), remove: vi.fn(), setNote: vi.fn(), clear: vi.fn() }
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
