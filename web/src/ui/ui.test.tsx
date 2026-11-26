import { act, fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { expect, test, vi } from 'vitest'
import { Avatar, Banner, ContextMenu, IconButton, type MenuAt, Menu, Tabs, Toaster, Tooltip, toast, useMenuAt } from '.'

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

function RightClickable({ onPin }: { onPin: () => void }) {
  const [at, setAt] = useState<MenuAt>()
  return (
    <div
      onContextMenu={(event) => {
        event.preventDefault()
        setAt({ x: event.clientX, y: event.clientY })
      }}
    >
      A thread
      {at && (
        <ContextMenu
          label="actions"
          items={[
            { label: 'Pin', onSelect: onPin },
            { label: 'Settle', onSelect: () => {} },
          ]}
          at={at}
          onClose={() => setAt(undefined)}
        />
      )}
    </div>
  )
}

test("a right-click opens the page's own menu in place of the browser's, at the pointer", () => {
  const onPin = vi.fn()
  render(<RightClickable onPin={onPin} />)
  const opened = fireEvent.contextMenu(screen.getByText('A thread'), { clientX: 40, clientY: 60 })
  expect(opened).toBe(false)
  const menu = screen.getByRole('menu', { name: 'actions' })
  expect(menu).toHaveStyle({ left: '40px', top: '60px' })
  expect(screen.getByRole('menuitem', { name: 'Pin' })).toHaveFocus()
  fireEvent.keyDown(menu, { key: 'ArrowDown' })
  expect(screen.getByRole('menuitem', { name: 'Settle' })).toHaveFocus()
  fireEvent.keyDown(document, { key: 'Escape' })
  expect(screen.queryByRole('menu')).toBeNull()

  fireEvent.contextMenu(screen.getByText('A thread'), { clientX: 40, clientY: 60 })
  fireEvent.click(screen.getByRole('menuitem', { name: 'Pin' }))
  expect(onPin).toHaveBeenCalledOnce()
  expect(screen.queryByRole('menu')).toBeNull()
})

function Pressable({ onOpen }: { onOpen: () => void }) {
  const menu = useMenuAt()
  return (
    <div {...menu.handlers}>
      <button type="button" onClick={onOpen}>
        A thread
      </button>
      {menu.at && (
        <ContextMenu label="actions" items={[{ label: 'Pin', onSelect: () => {} }]} at={menu.at} onClose={menu.close} />
      )}
    </div>
  )
}

test('a long press opens the same menu on a touch screen, and the tap ending it follows no link', () => {
  vi.useFakeTimers()
  const onOpen = vi.fn()
  render(<Pressable onOpen={onOpen} />)
  const row = screen.getByRole('button', { name: 'A thread' })
  fireEvent.touchStart(row, { touches: [{ clientX: 30, clientY: 50 }] })
  act(() => vi.advanceTimersByTime(499))
  expect(screen.queryByRole('menu')).toBeNull()
  act(() => vi.advanceTimersByTime(1))
  expect(screen.getByRole('menu', { name: 'actions' })).toHaveStyle({ left: '30px', top: '50px' })
  fireEvent.touchEnd(row)
  fireEvent.click(row)
  expect(onOpen).not.toHaveBeenCalled()

  // A touch that moves is a scroll, and a short one a tap.
  fireEvent.keyDown(document, { key: 'Escape' })
  fireEvent.touchStart(row, { touches: [{ clientX: 30, clientY: 50 }] })
  fireEvent.touchMove(row, { touches: [{ clientX: 30, clientY: 80 }] })
  act(() => vi.advanceTimersByTime(600))
  expect(screen.queryByRole('menu')).toBeNull()
  fireEvent.touchStart(row, { touches: [{ clientX: 30, clientY: 50 }] })
  fireEvent.touchEnd(row)
  fireEvent.click(row)
  expect(onOpen).toHaveBeenCalledOnce()
  vi.useRealTimers()
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

test("a toast with the same key takes the last one's place", () => {
  vi.useFakeTimers()
  const { container } = render(<Toaster />)
  act(() => toast('Zoom 110%', 'ok', 'zoom'))
  act(() => toast('Zoom 120%', 'ok', 'zoom'))
  act(() => toast('Saved', 'ok'))
  expect([...container.querySelectorAll('[role="status"]')].map((shown) => shown.textContent)).toEqual(['Zoom 120%', 'Saved'])
  act(() => vi.advanceTimersByTime(6000))
  vi.useRealTimers()
})
