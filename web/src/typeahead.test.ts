import { expect, test } from 'vitest'
import { typeahead } from './typeahead'

const names = ['delta', 'silt', 'scratch', 'notes', 'Sandbox']

test('a letter typed again steps through the names it starts, round to the first', () => {
  expect(typeahead(names, -1, 's')).toBe(1)
  expect(typeahead(names, 1, 'ss')).toBe(2)
  expect(typeahead(names, 2, 'sss')).toBe(4)
  expect(typeahead(names, 4, 'ssss')).toBe(1)
})

test('several letters find the first name they start, staying on one that still matches', () => {
  expect(typeahead(names, -1, 'sc')).toBe(2)
  expect(typeahead(names, 2, 'scr')).toBe(2)
  expect(typeahead(names, 1, 'SA')).toBe(4)
})

test('nothing matches, nothing moves', () => {
  expect(typeahead(names, 0, 'x')).toBeUndefined()
  expect(typeahead([], -1, 's')).toBeUndefined()
  expect(typeahead(names, 0, '')).toBeUndefined()
})
