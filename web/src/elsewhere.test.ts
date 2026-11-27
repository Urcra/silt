import { expect, test } from 'vitest'
import type { ThreadSummary } from './api'
import { newThreadLink, openLink, stateLink, viewLink } from './desktop'
import { countsOf, merged } from './elsewhere'

const thread = (id: string, overrides: Partial<ThreadSummary> = {}) =>
  ({ id, location: 'here', agent_state: null, review: false, background: 0, ...overrides }) as ThreadSummary

test('other machines’ threads join this one’s, each from the machine that holds it', () => {
  const local = [thread('a'), thread('b', { location: 'remote' })]
  const listings = [
    { id: 'ssh:nuc', state: 'ready' as const, threads: [thread('b'), thread('c'), thread('a', { location: 'remote' })] },
    { id: 'wsl:Ubuntu', state: 'lost' as const, threads: [thread('d')] },
    { id: 'native', state: 'ready' as const, threads: [thread('e')] },
  ]
  const listed = merged(local, listings, 'native')
  expect(listed.map((each) => [each.id, (each as { backend?: string }).backend])).toEqual([
    ['a', undefined],
    ['b', 'ssh:nuc'],
    ['c', 'ssh:nuc'],
  ])
})

test('a machine’s counts are of the threads it holds', () => {
  const listing = {
    id: 'ssh:nuc',
    state: 'ready' as const,
    threads: [
      thread('a', { agent_state: 'working' }),
      thread('b', { agent_state: 'awaiting_input' }),
      thread('c', { agent_state: 'working', location: 'remote' }),
    ],
  }
  expect(countsOf(listing)).toEqual({ running: 1, needsYou: 1 })
  expect(countsOf({ ...listing, state: 'lost' })).toEqual({ running: 0, needsYou: 0 })
  expect(countsOf(undefined)).toEqual({ running: 0, needsYou: 0 })
})

test('the links to the desktop app carry its key and only what they name', () => {
  expect(viewLink('', { list: 'all' }, 'k')).toBe('silt://view/?list=all&key=k')
  expect(viewLink('colour', { backend: 'ssh:nuc', colour: 'rose' }, 'k')).toBe(
    'silt://view/colour?backend=ssh%3Anuc&colour=rose&key=k',
  )
  const change = { pinned: true, snooze: null, settled: null, seen: false, reviewed_turn: null }
  const link = new URL(stateLink('ssh:nuc', 'a81cee7abcb7', change, 'k'))
  expect(link.host).toBe('threads')
  expect(JSON.parse(link.searchParams.get('change') ?? '')).toEqual(change)
  expect(openLink('ssh:nuc', 'a81cee7abcb7')).toBe('silt://open/?backend=ssh%3Anuc&thread=a81cee7abcb7')
  expect(newThreadLink('ssh:nuc', 'fix it', 'github.com/u/r')).toBe(
    'silt://open/?backend=ssh%3Anuc&new=1&origin=github.com%2Fu%2Fr&draft=fix+it',
  )
  expect(newThreadLink('native', '', '')).toBe('silt://open/?backend=native&new=1')
})
