import { act, fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { expect, test, vi } from 'vitest'
import { Avatar, Banner, IconButton, Menu, Tabs, Toaster, Tooltip, toast } from '.'

function TwoTabs() {
  const [tab, setTab] = useState<'diff' | 'chat'>('chat')
  return (
    <Tabs
      label="view"
      value={tab}
      onChange={setTab}
      tabs={[
        { id: 'chat', label: 'Chat' },
        { id: 'diff', label: 'Diff' },
      ]}
    />
  )
}

test('arrow keys move between tabs and select them', () => {
  render(<TwoTabs />)
  const chat = screen.getByRole('tab', { name: 'Chat' })
  expect(chat).toHaveAttribute('aria-selected', 'true')
  expect(screen.getByRole('tab', { name: 'Diff' })).toHaveAttribute('tabindex', '-1')
  fireEvent.keyDown(chat, { key: 'ArrowRight' })
  const diff = screen.getByRole('tab', { name: 'Diff' })
  expect(diff).toHaveAttribute('aria-selected', 'true')
  expect(diff).toHaveFocus()
  fireEvent.keyDown(diff, { key: 'ArrowRight' })
  expect(screen.getByRole('tab', { name: 'Chat' })).toHaveFocus()
})

test('a menu opens on its first item, moves with arrows and closes on Escape', () => {
  const fork = vi.fn()
  render(
    <Menu
      label="thread actions"
      trigger="…"
      items={[
        { label: 'Fork', onSelect: fork },
        { label: 'Trash', onSelect: () => undefined },
      ]}
    />,
  )
  const button = screen.getByRole('button', { name: 'thread actions' })
  fireEvent.click(button)
  expect(button).toHaveAttribute('aria-expanded', 'true')
  const first = screen.getByRole('menuitem', { name: 'Fork' })
  expect(first).toHaveFocus()
  fireEvent.keyDown(first, { key: 'ArrowDown' })
  expect(screen.getByRole('menuitem', { name: 'Trash' })).toHaveFocus()
  fireEvent.keyDown(document, { key: 'Escape' })
  expect(screen.queryByRole('menu')).toBeNull()
  expect(button).toHaveFocus()
  fireEvent.click(button)
  fireEvent.click(screen.getByRole('menuitem', { name: 'Fork' }))
  expect(fork).toHaveBeenCalledOnce()
})

test('a tooltip describes its element while focused', () => {
  render(<Tooltip text="Copies the command">{(props) => <IconButton icon="copy" label="copy" {...props} />}</Tooltip>)
  const button = screen.getByRole('button', { name: 'copy' })
  const tip = screen.getByRole('tooltip', { hidden: true })
  expect(button).toHaveAttribute('aria-describedby', tip.id)
  expect(tip).toHaveClass('hidden')
  fireEvent.focus(button)
  expect(tip).not.toHaveClass('hidden')
})

test('toasts come and go, and banners and avatars say what they are', () => {
  vi.useFakeTimers()
  render(
    <>
      <Toaster />
      <Banner tone="danger" urgent>
        The daemon stopped
      </Banner>
      <Avatar name="ada lovelace" />
    </>,
  )
  act(() => toast('could not fork', 'danger'))
  expect(screen.getAllByRole('alert').map((alert) => alert.textContent)).toContain('could not fork')
  act(() => vi.advanceTimersByTime(6000))
  expect(screen.queryByText('could not fork')).toBeNull()
  expect(screen.getByRole('alert')).toHaveTextContent('The daemon stopped')
  expect(screen.getByRole('img', { name: 'ada lovelace' })).toHaveTextContent('AL')
  vi.useRealTimers()
})
