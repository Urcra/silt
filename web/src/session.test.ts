import { describe, expect, it } from 'vitest'
import type { AgentStatus, BusEvent } from './api'
import { type Live, heard } from './session'
import { NOTHING_SAID } from './unrecorded'

const status = (streaming: string, thinking = ''): AgentStatus =>
  ({ agent: 'fake', state: 'working', streaming, thinking, permissions: [], questions: [] }) as unknown as AgentStatus

const start: Live = {
  changes: 0,
  deltas: null,
  agent: status(''),
  preset: 'ask_before_commands',
  tool: { id: null, text: '' },
  said: NOTHING_SAID,
  began: 0,
}

const text = (kind: 'message' | 'thought', said: string): BusEvent => ({
  event: 'agent_text',
  thread: 't',
  kind,
  text: said,
  tool_use_id: null,
  restart: false,
})

describe('heard', () => {
  it('adds streamed words up between whole statuses, which replace them', () => {
    let live = [text('thought', 'Hm'), text('thought', 'm.'), text('message', 'Hello'), text('message', ' there')].reduce(
      (state, event) => heard(state, event, 4),
      start,
    )
    expect(live.agent?.thinking).toBe('Hmm.')
    expect(live.agent?.streaming).toBe('Hello there')
    expect(live.began).toBe(4)
    live = heard(live, { event: 'agent', thread: 't', status: status('Hello there, all') }, 4)
    expect(live.agent?.streaming).toBe('Hello there, all')
    live = heard(live, text('message', '!'), 4)
    expect(live.agent?.streaming).toBe('Hello there, all!')
  })

  it('keeps what was said once a status drops it as recorded', () => {
    const live = heard(heard(start, text('message', 'Done.'), 2), { event: 'agent', thread: 't', status: status('') }, 2)
    expect(live.said.text).toBe('Done.')
    expect(live.agent?.streaming).toBe('')
  })

  it('ignores words with no agent to say them', () => {
    const none = { ...start, agent: null }
    expect(heard(none, text('message', 'x'), 0)).toBe(none)
  })
})
