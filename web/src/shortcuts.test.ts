import { expect, test } from 'vitest'
import { matches } from './shortcuts'

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
