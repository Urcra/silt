import { expect, test } from 'vitest'
import type { AskView, HistoryEntry } from './api.generated'
import { askFor, askOfCall, askedIn, siltTool } from './asks'
import { tasked } from './transcript'
import { NO_TASKS } from './tasks'

const call = (name: string, input: object, time_ms = 10_000): HistoryEntry =>
  ({
    index: 3,
    thread: 'caller',
    origin: 'own',
    kind: 'tool_call',
    text: `${name} ${JSON.stringify(input)}`,
    time_ms,
    rewound: false,
    side: null,
    task: null,
    tool: { id: 'c1', name, kind: 'other', subject: null, detail: null, subagent: null, questions: [] },
  }) as unknown as HistoryEntry

const ask = (more: Partial<AskView>): AskView => ({
  id: 'a1',
  caller: 'caller',
  caller_agent: 'Claude Agent',
  caller_agent_id: 'claude-acp',
  asked: 'asked',
  agent: 'antigravity-acp',
  agent_name: 'Antigravity',
  model: 'Gemini 4 Neon',
  effort: null,
  mode: 'review',
  checkout: 'here',
  title: 'Review the retry',
  prompt: 'Review the retry',
  state: 'running',
  report: null,
  why: null,
  started_ms: 10_200,
  ended_ms: null,
  taken: false,
  ...more,
})

test("Silt's tools are known by name however an agent titles them", () => {
  expect(siltTool('mcp__silt__silt_ask')).toBe('silt_ask')
  expect(siltTool('silt_ask (silt MCP Server)')).toBe('silt_ask')
  expect(siltTool('silt_ask_wait')).toBe('silt_ask_wait')
  expect(siltTool('Bash')).toBeNull()
  expect(siltTool(null)).toBeNull()
})

test('a call that asked another agent stands on its own, and finds its ask by its prompt', () => {
  const asking = call('mcp__silt__silt_ask', { agent: 'agy', prompt: '  Review the retry ' })
  expect(askOfCall(asking)?.agent).toBe('agy')
  expect(tasked({ call: asking, result: null, images: [] }, NO_TASKS)).toBe(true)
  expect(askOfCall(call('silt_ask_wait', { ask: 'a1' }))).toBeNull()

  const earlier = ask({ id: 'a0', started_ms: 1_000, state: 'done' })
  const this_one = ask({})
  const elsewhere = ask({ id: 'a2', caller: 'other' })
  expect(askFor([earlier, this_one, elsewhere], 'caller', asking)?.id).toBe('a1')
  expect(askFor([elsewhere], 'caller', asking)).toBeUndefined()

  const follow = call('silt_ask', { to: 'a0', prompt: 'And the backoff?' })
  expect(askFor([earlier, this_one], 'caller', follow)?.id).toBe('a0')
})

test('an asked thread knows which ask it answers', () => {
  expect(askedIn([ask({})], 'asked')?.caller).toBe('caller')
  expect(askedIn([ask({})], 'caller')).toBeUndefined()
})
