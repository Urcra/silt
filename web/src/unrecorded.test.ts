import { expect, test } from 'vitest'
import { recorded, unrecorded } from './unrecorded'

const user = (text: string) => ({ kind: 'user', text })
const said = (text: string) => ({ kind: 'assistant', text })
const thought = (text: string) => ({ kind: 'thinking', text })

test('words stay shown until the history has them', () => {
  const before = [user('fix it'), thought('look at main'), said('Looking.')]
  expect(unrecorded({ text: 'Fixed it.\n', thought: '' }, before)).toEqual({ text: 'Fixed it.\n', thought: '' })
  expect(unrecorded({ text: 'Fixed it.\n', thought: '' }, [...before, said('Fixed it.')])).toEqual({ text: '', thought: '' })
})

test('a thought stays shown until the history has it', () => {
  expect(unrecorded({ text: '', thought: 'look at main' }, [user('fix it')]).thought).toBe('look at main')
  expect(unrecorded({ text: '', thought: 'look at main' }, [user('fix it'), thought('look at main')]).thought).toBe('')
})

test('the same words in an earlier turn are not these', () => {
  expect(recorded('Done.', 'assistant', [user('a'), said('Done.'), user('b')])).toBe(false)
  expect(recorded('Done.', 'assistant', [user('a'), said('Done.'), user('b'), said('Done.')])).toBe(true)
})
