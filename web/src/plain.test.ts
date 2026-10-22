import { expect, test } from 'vitest'
import { plain } from './plain'

test('inline Markdown reads as its plain words', () => {
  expect(plain('Add an **acp** function, then run `cargo test`.')).toBe('Add an acp function, then run cargo test.')
  expect(plain('See [the docs](https://example.com) and _this_')).toBe('See the docs and this')
  expect(plain('# Title\nmore')).toBe('Title more')
})
