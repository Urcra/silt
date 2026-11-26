import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, test } from 'vitest'
import { resetFolds, useFold } from './folds'

afterEach(() => {
  cleanup()
  resetFolds()
})

function Fold({ name, initially }: { name: string; initially: boolean }) {
  const [open, setOpen] = useFold(name, initially)
  return (
    <button type="button" aria-label={name} aria-expanded={open} onClick={() => setOpen(!open)}>
      {name}
    </button>
  )
}

test('a fold starts as asked, and keeps what the person chose in this browser', () => {
  render(
    <>
      <Fold name="chats" initially />
      <Fold name="project:/old" initially={false} />
    </>,
  )
  const chats = screen.getByRole('button', { name: 'chats' })
  expect(chats).toHaveAttribute('aria-expanded', 'true')
  expect(screen.getByRole('button', { name: 'project:/old' })).toHaveAttribute('aria-expanded', 'false')
  fireEvent.click(chats)
  expect(chats).toHaveAttribute('aria-expanded', 'false')
  expect(JSON.parse(localStorage.getItem('silt.folds') ?? '{}')).toEqual({ chats: false })
})

test('every place showing a fold follows it', () => {
  render(
    <>
      <Fold name="chats" initially />
      <Fold name="chats" initially />
    </>,
  )
  const [one, other] = screen.getAllByRole('button', { name: 'chats' })
  fireEvent.click(one)
  expect(other).toHaveAttribute('aria-expanded', 'false')
})
