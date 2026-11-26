import { expect, test } from 'vitest'
import type { AgentStatus, ThreadSummary } from './api'
import { threadDoing, threadWhere, waitKind } from './threadLook'

const thread = (overrides: Partial<ThreadSummary>) => ({ agent_state: null, review: false, ...overrides }) as ThreadSummary
const status = (overrides: Partial<AgentStatus>) =>
  ({ state: 'awaiting_permission', permissions: [], questions: [], ...overrides }) as AgentStatus

test('a waiting thread is sorted by what it waits for, the agent’s status first', () => {
  const asking = thread({ agent_state: 'awaiting_permission' })
  expect(waitKind(asking, status({ questions: [{ id: 1, message: 'What?', fields: [] }] }))).toBe('question')
  expect(
    waitKind(asking, status({ permissions: [{ id: 1, title: 'Plan', options: [], plan: '# Plan', tool_call: null }] })),
  ).toBe('plan')
  expect(
    waitKind(asking, status({ permissions: [{ id: 1, title: 'Edit src/main.rs', options: [], plan: null, tool_call: null }] })),
  ).toBe('permission')
  // Before its status arrives, the thread's own state says enough.
  expect(waitKind(asking, null)).toBe('permission')
  expect(waitKind(thread({ agent_state: 'awaiting_input' }), null)).toBe('plan')
  expect(waitKind(thread({ agent_state: 'error' }), null)).toBe('stopped')
  expect(waitKind(thread({ review: true }), null)).toBe('review')
})

test('a thread elsewhere names the machine holding it, when the server said which', () => {
  expect(threadDoing(thread({ location: 'remote', machine: 'homelab' }))).toBe('on homelab')
  expect(threadDoing(thread({ location: 'remote', machine: null }))).toBe('not on this machine')
  expect(threadDoing(thread({ location: 'moved', machine: 'home' }))).toBe('moved to home')
  expect(threadDoing(thread({ location: 'moved', machine: null }))).toBe('moved to another device')
  expect(threadWhere(thread({ location: 'remote', machine: 'homelab' })).label).toBe('On homelab')
  expect(threadWhere(thread({ location: 'remote', machine: null })).label).toBe('On the server')
  expect(threadWhere(thread({ location: 'moved', machine: 'home' })).label).toBe('Moved to home')
})

test('an agent working in a terminal says so, as its terminal answers it rather than the page', () => {
  expect(threadDoing(thread({ agent_state: 'working', terminal: true }))).toBe('working in a terminal')
  expect(threadDoing(thread({ agent_state: 'working', terminal: false }))).toBe('working')
})
