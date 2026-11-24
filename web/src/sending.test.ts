import { expect, test } from 'vitest'
import { arrived } from './sending'

const user = (text: string) => ({ kind: 'user', text })
const said = (text: string) => ({ kind: 'assistant', text })
/** Entries as a history starting at `from` holds them. */
const at = (entries: { kind: string; text: string }[], from = 0) =>
  entries.map((entry, index) => ({ ...entry, index: from + index }))

test('a prompt arrives once the history has it after the entries it followed', () => {
  const sent = [{ id: 1, text: 'fix it\n', files: [], after: 2 }]
  expect(arrived(sent, at([user('fix it'), said('done')]))).toEqual([])
  expect(arrived(sent, at([user('fix it'), said('done'), user('fix it')]))).toEqual([1])
})

test('the same words sent twice wait for two prompts', () => {
  const sent = [
    { id: 1, text: 'go on', files: [], after: 0 },
    { id: 2, text: 'go on', files: [], after: 0 },
  ]
  expect(arrived(sent, at([user('go on'), said('ok')]))).toEqual([1])
  expect(arrived(sent, at([user('go on'), said('ok'), user('go on')]))).toEqual([1, 2])
})

test('a prompt recorded with more around it still counts', () => {
  const sent = [{ id: 3, text: 'look at this', files: ['shot.png'], after: 0 }]
  expect(arrived(sent, at([user('look at this\n\n[shot.png]')]))).toEqual([3])
})

test('a prompt the agent was told otherwise takes the next prompt left', () => {
  const sent = [
    { id: 1, text: '/review', files: [], after: 1 },
    { id: 2, text: 'and then this', files: [], after: 1 },
  ]
  expect(arrived(sent, at([user('old')]))).toEqual([])
  expect(arrived(sent, at([user('old'), user('and then this')]))).toEqual([2])
  expect(arrived(sent, at([user('old'), user('and then this'), user('Review the changes on this branch')]))).toEqual([1, 2])
})

test('a prompt arrives by the indexes of the entries, in a history held from part way', () => {
  const sent = [{ id: 1, text: 'go\n', files: [], after: 1002 }]
  expect(arrived(sent, at([user('go'), said('ok')], 1000))).toEqual([])
  expect(arrived(sent, at([user('go'), said('ok'), user('go')], 1000))).toEqual([1])
})
