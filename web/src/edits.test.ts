import { expect, test } from 'vitest'
import type { HistoryEntry } from './api'
import { changeOf } from './edits'

const call = (text: string, kind: HistoryEntry['kind'] = 'tool_call'): HistoryEntry =>
  ({ index: 0, kind, text, time_ms: 0, author: 'agent', rewound: false }) as unknown as HistoryEntry

test("an edit's diff comes from its recorded input", () => {
  expect(changeOf(call('Edit {"file_path":"src/a.rs","old_string":"fn a() {}\\n","new_string":"fn a() {}\\nfn b() {}\\n"}'))).toEqual({
    verb: 'Edited',
    file: 'src/a.rs',
    removed: ['fn a() {}'],
    added: ['fn a() {}', 'fn b() {}'],
  })
  expect(
    changeOf(call('MultiEdit {"file_path":"b.rs","edits":[{"old_string":"x","new_string":"y"},{"old_string":"1","new_string":"2"}]}')),
  ).toEqual({ verb: 'Edited', file: 'b.rs', removed: ['x', '1'], added: ['y', '2'] })
  expect(changeOf(call('Write {"file_path":"new.md","content":"# New\\n\\nhello\\n"}'))).toEqual({
    verb: 'Wrote',
    file: 'new.md',
    removed: [],
    added: ['# New', '', 'hello'],
  })
})

test('what changes nothing, or says too little, is no edit', () => {
  expect(changeOf(call('Read {"file_path":"src/a.rs"}'))).toBeNull()
  expect(changeOf(call('Edit {"old_string":"x","new_string":"y"}'))).toBeNull()
  expect(changeOf(call('Edit not json'))).toBeNull()
  expect(changeOf(call('Edit {"file_path":"a.rs"}', 'tool_result'))).toBeNull()
  expect(changeOf(call('Edit {"file_path":"a.rs"}')), 'an input without its strings').toBeNull()
})
