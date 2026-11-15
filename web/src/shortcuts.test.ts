import { expect, test } from 'vitest'
import { type Shortcut, matches, pick } from './shortcuts'

const press = (key: string, held: Partial<Record<'metaKey' | 'ctrlKey' | 'shiftKey' | 'altKey', boolean>> = {}) => ({
  key,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...held,
})

test('mod means command on a Mac and control elsewhere, and other modifiers must match', () => {
  expect(matches(press('k', { metaKey: true }), 'mod+k', true)).toBe(true)
  expect(matches(press('k', { ctrlKey: true }), 'mod+k', true)).toBe(false)
  expect(matches(press('k', { ctrlKey: true }), 'mod+k', false)).toBe(true)
  expect(matches(press('K', { ctrlKey: true, shiftKey: true }), 'mod+k', false)).toBe(false)
  expect(matches(press('n'), 'n', false)).toBe(true)
  expect(matches(press('n', { ctrlKey: true }), 'n', false)).toBe(false)
})

const key = (keys: string, label: string, over = false): Shortcut => ({ keys, label, over, run: () => undefined })

test("a view's key registered over the app's wins, whatever came first", () => {
  const app = key('n', 'New thread')
  const review = key('n', 'Next hunk', true)
  expect(pick([app, review], press('n'), false, false)?.label).toBe('Next hunk')
  expect(pick([review, app], press('n'), false, false)?.label).toBe('Next hunk')
  expect(pick([app], press('n'), false, false)?.label).toBe('New thread')
})

test('while writing, only keys with a modifier run', () => {
  const finish = key('mod+enter', 'Finish review', true)
  const next = key('j', 'Next file', true)
  expect(pick([next, finish], press('j'), true, false)).toBeUndefined()
  expect(pick([next, finish], press('Enter', { ctrlKey: true }), true, false)?.label).toBe('Finish review')
})
