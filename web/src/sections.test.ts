import { expect, test } from 'vitest'
import type { ThreadSummary } from './api'
import { old, section, sections, unread } from './sections'

function thread(id: string, overrides: Partial<ThreadSummary> = {}, mine: Partial<ThreadSummary['mine']> = {}): ThreadSummary {
  return {
    id,
    repo: 'github.com/acme/app',
    title: id,
    agent: null,
    created_ms: 0,
    messages: 0,
    edits: 0,
    location: 'here',
    forked_from: null,
    owner: '',
    collaborators: [],
    writable: true,
    agent_state: null,
    background: 0,
    last_ms: 100,
    merged: false,
    review: false,
    mine: { pinned: false, snoozed_until: null, snoozed_at: null, wake_on_activity: false, settled: false, seen_ms: null, ...mine },
    visibility: null,
    server: null,
    local_only: false,
    cloud: null,
    ...overrides,
  }
}

test('a thread waiting on the person shows first, even snoozed or settled', () => {
  expect(section(thread('a', { agent_state: 'awaiting_permission' }, { settled: true }), 0)).toBe('needs_you')
  expect(section(thread('b', { review: true }, { snoozed_until: 999, snoozed_at: 50 }), 0)).toBe('needs_you')
  expect(section(thread('c', {}, { pinned: true }), 0)).toBe('pinned')
  expect(section(thread('d', { agent_state: 'working' }), 0)).toBe('working')
  expect(section(thread('e', { merged: true }), 0)).toBe('settled')
  expect(section(thread('f'), 0)).toBe('active')
})

test('a thread whose agent left a command running in the background is in progress, though its turn ended', () => {
  expect(section(thread('a', { agent_state: 'idle', background: 1 }, { settled: true }), 0)).toBe('working')
  expect(section(thread('b', { agent_state: 'idle', background: 0 }), 0)).toBe('active')
  expect(section(thread('c', { agent_state: 'awaiting_permission', background: 1 }), 0)).toBe('needs_you')
})

test('a snooze ends at its time, or at activity when it wakes on it', () => {
  const quiet = thread('a', { last_ms: 40 }, { snoozed_until: 1000, snoozed_at: 50, wake_on_activity: true })
  expect(section(quiet, 500)).toBe('snoozed')
  expect(section(quiet, 1000)).toBe('active')
  const busy = thread('b', { last_ms: 60 }, { snoozed_until: 1000, snoozed_at: 50, wake_on_activity: true })
  expect(section(busy, 500)).toBe('active')
  const deaf = thread('c', { last_ms: 60 }, { snoozed_until: 1000, snoozed_at: 50 })
  expect(section(deaf, 500)).toBe('snoozed')
})

test('a thread is unread only after the person once looked and something happened since', () => {
  expect(unread(thread('a', { last_ms: 100 }))).toBe(false)
  expect(unread(thread('b', { last_ms: 100 }, { seen_ms: 90 }))).toBe(true)
  expect(unread(thread('c', { last_ms: 100 }, { seen_ms: 100 }))).toBe(false)
  const ordered = sections([thread('old', { last_ms: 1 }), thread('new', { last_ms: 9 })], 0).active.map((one) => one.id)
  expect(ordered).toEqual(['new', 'old'])
})

test('a thread is old after two weeks untouched, unless something runs or waits in it or it is pinned', () => {
  const day = 86_400_000
  const now = 100 * day
  const last_ms = now - 15 * day
  expect(old(thread('a', { last_ms }), now)).toBe(true)
  expect(old(thread('b', { last_ms }, { settled: true }), now)).toBe(true)
  expect(old(thread('c', { last_ms: now - 13 * day }), now)).toBe(false)
  expect(old(thread('d', { last_ms, agent_state: 'working' }), now)).toBe(false)
  expect(old(thread('e', { last_ms, agent_state: 'awaiting_input' }), now)).toBe(false)
  expect(old(thread('f', { last_ms, review: true }), now)).toBe(false)
  expect(old(thread('g', { last_ms }, { pinned: true }), now)).toBe(false)
})
