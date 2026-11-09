import { expect, test } from 'vitest'
import { recorded, unrecorded } from './unrecorded'

const indexed = (entries: { kind: string; text: string }[]) => entries.map((entry, index) => ({ ...entry, index }))
const user = (text: string) => ({ kind: 'user', text })
const said = (text: string) => ({ kind: 'assistant', text })
const thought = (text: string) => ({ kind: 'thinking', text })

test('words stay shown until the history has them', () => {
  const before = [user('fix it'), thought('look at main'), said('Looking.')]
  const words = { text: 'Fixed it.\n', thought: '', textAfter: 3, thoughtAfter: 3 }
  expect(unrecorded(words, indexed(before)).text).toBe('Fixed it.\n')
  expect(unrecorded(words, indexed([...before, said('Fixed it.')])).text).toBe('')
})

test('a thought stays shown until the history has it', () => {
  const words = { text: '', thought: 'look at main', textAfter: 1, thoughtAfter: 1 }
  expect(unrecorded(words, indexed([user('fix it')])).thought).toBe('look at main')
  expect(unrecorded(words, indexed([user('fix it'), thought('look at main')])).thought).toBe('')
})

test('the same words in an earlier turn are not these', () => {
  expect(recorded('Done.', 'assistant', indexed([user('a'), said('Done.'), user('b')]), 3)).toBe(false)
  expect(recorded('Done.', 'assistant', indexed([user('a'), said('Done.'), user('b'), said('Done.')]), 3)).toBe(true)
})

test('words are recorded though a queued prompt already follows them', () => {
  expect(recorded('Done.', 'assistant', indexed([user('a'), said('Done.'), user('queued')]), 1)).toBe(true)
})

test('words are recorded though the history had them before they were seen to begin', () => {
  expect(recorded('Done.', 'assistant', indexed([user('a'), said('Done.')]), 2)).toBe(true)
})

test('words and a later thought are each looked for from where they began', () => {
  const history = indexed([user('a'), thought('hm'), said('Done.'), user('queued')])
  expect(unrecorded({ text: 'Done.', thought: 'hm', textAfter: 1, thoughtAfter: 4 }, history)).toMatchObject({ text: '', thought: 'hm' })
})
