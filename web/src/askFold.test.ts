import { expect, test } from 'vitest'
import type { ThreadSummary } from './api'
import type { AskView } from './api.generated'
import { ASKS_SHOWN, byCaller, foldListed, foldSaid } from './askFold'

const thread = (id: string, agent_state: ThreadSummary['agent_state'] = null): ThreadSummary =>
  ({ id, title: id, agent_state, review: false, mine: { settled: false } }) as unknown as ThreadSummary

const ask = (asked: string, caller: string, started_ms: number, state: AskView['state'] = 'done'): AskView =>
  ({ id: `ask-${asked}`, caller, asked, agent: 'antigravity-acp', agent_name: 'Antigravity', state, started_ms }) as AskView

test('asked threads fold under the thread at the top of their callers, newest first, and the rest keep their order', () => {
  const threads = [thread('first'), thread('older-ask'), thread('caller'), thread('newer-ask'), thread('its-own'), thread('last')]
  const asks = [ask('older-ask', 'caller', 1), ask('newer-ask', 'caller', 2), ask('its-own', 'newer-ask', 3)]
  const callers = byCaller(threads, asks)
  expect(callers.map(({ thread }) => thread.id)).toEqual(['first', 'caller', 'last'])
  expect(callers[1]!.asked.map(({ thread }) => thread.id)).toEqual(['its-own', 'newer-ask', 'older-ask'])
})

test("an asked thread whose caller isn't listed stands alone, as every thread does before the asks load", () => {
  const threads = [thread('asked'), thread('other')]
  expect(byCaller(threads, [ask('asked', 'elsewhere', 1)]).map(({ asked }) => asked.length)).toEqual([0, 0])
  expect(byCaller(threads, []).map(({ thread }) => thread.id)).toEqual(['asked', 'other'])
})

test('a fold says what is most pressing: one waiting on the person, then failed, then working, else reported', () => {
  const asked = (id: string, state: AskView['state'], agent_state: ThreadSummary['agent_state'] = null) => ({
    thread: thread(id, agent_state),
    ask: ask(id, 'caller', 1, state),
  })
  expect(foldSaid([asked('a', 'done'), asked('b', 'done')])).toEqual({ kind: 'reported' })
  expect(foldSaid([asked('a', 'done'), asked('b', 'running')])).toEqual({ kind: 'working', count: 1 })
  expect(foldSaid([asked('a', 'failed', 'error'), asked('b', 'running')])).toEqual({ kind: 'failed', count: 1 })
  expect(foldSaid([asked('a', 'failed'), asked('b', 'running', 'awaiting_permission')])).toEqual({ kind: 'needs_you', count: 1 })
  expect(foldSaid([asked('a', 'done'), asked('b', 'stopped')])).toBeNull()
})

test('a folded fold still lists what waits on the person and the thread open; an opened one lists the first few', () => {
  const asked = Array.from({ length: ASKS_SHOWN + 3 }, (_, at) => ({
    thread: thread(`t${at}`, at === 1 ? 'awaiting_input' : null),
    ask: ask(`t${at}`, 'caller', 100 - at),
  }))
  const ids = (rows: { thread: ThreadSummary }[]) => rows.map(({ thread }) => thread.id)
  expect(foldListed(asked, false, false, 't7')).toEqual({ rows: [asked[1], asked[7]], more: 0 })
  const opened = foldListed(asked, true, false, 't7')
  expect(ids(opened.rows)).toEqual(['t0', 't1', 't2', 't3', 't4', 't7'])
  expect(opened.more).toBe(2)
  expect(foldListed(asked, true, true, null)).toEqual({ rows: asked, more: 0 })
})
