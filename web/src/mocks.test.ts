import { expect, test } from 'vitest'
import doc from '../../docs/ui-mocks.md?raw'
import { MOCKS } from './mocks'

/** The ids the doc lists, from its table's first column. */
function documented(): string[] {
  return [...doc.matchAll(/^\| `([a-z0-9.-]+)` \|/gm)].map((match) => match[1])
}

test('every mock in the code is in docs/ui-mocks.md, and the doc lists no other', () => {
  expect(documented().sort()).toEqual(Object.keys(MOCKS).sort())
})
