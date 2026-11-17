import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import { api } from './api'
import { HiddenNote, HideGitBox, HideOthersBox } from './components/DiffPanel'
import { setHideOthers, useHideGit, useHideOthers } from './hideGit'

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  setHideOthers(false)
})

const THREAD = '0123456789abcdef0123456789abcdef'

function Boxes({ disabled = false }: { disabled?: boolean }) {
  const [hideGit, setHideGit] = useHideGit()
  const [hideOthers, setOthers] = useHideOthers()
  return (
    <>
      <HideGitBox on={hideGit} onChange={setHideGit} disabled={disabled} />
      <HideOthersBox on={hideOthers} onChange={setOthers} disabled={disabled} />
      <HiddenNote hidden={{ hideGit: hideGit && !disabled, hideOthers: hideOthers && !disabled }} />
    </>
  )
}

test("other agents' changes are hidden apart from git's, and the choice is kept", () => {
  render(<Boxes />)
  const others = screen.getByRole('checkbox', { name: 'Hide other agents’ changes' })
  expect(others).not.toBeChecked()
  expect(screen.queryByRole('note')).toBeNull()
  fireEvent.click(others)
  expect(others).toBeChecked()
  expect(screen.getByRole('checkbox', { name: 'Hide git’s changes' })).not.toBeChecked()
  expect(localStorage.getItem('silt.hide-others')).toBe('true')
  expect(screen.getByRole('note')).toHaveTextContent('Without other agents’ edits in this clone')
})

test('against main neither can be hidden', () => {
  setHideOthers(true)
  render(<Boxes disabled />)
  const others = screen.getByRole('checkbox', { name: 'Hide other agents’ changes' })
  expect(others).toBeDisabled()
  expect(others).not.toBeChecked()
  expect(screen.queryByRole('note')).toBeNull()
})

test("reads without other agents' changes say so", async () => {
  const asked: string[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) => {
      asked.push(path)
      return new Response(JSON.stringify([]))
    }),
  )
  await api.comments(THREAD, undefined, { hideOthers: true })
  await api.mergeRequest('mr1', { hideGit: true, hideOthers: true })
  await api.mergeRequest('mr1')
  await api.diffStats(THREAD, { turn: 2, hideOthers: true })
  expect(asked).toEqual([
    `/api/threads/${THREAD}/comments?others=hide`,
    '/api/mrs/mr1?git=hide&others=hide',
    '/api/mrs/mr1',
    `/api/threads/${THREAD}/diff/stats?turn=2&others=hide`,
  ])
})
