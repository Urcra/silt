import { expect, test } from 'vitest'
import type { ThreadSummary } from './api'
import { section, sections, unread } from './sections'

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
