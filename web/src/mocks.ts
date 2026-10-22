// What the boards draw that the daemon can't tell yet (ADR 0035). Each mock
// is listed here and in docs/ui-mocks.md, with what replacing it needs; a
// build with VITE_SILT_MOCKS=off leaves every mocked part out.

export const MOCKS = {
  'thread.handoff-before': 'The device a thread was on before its last handoff, and when it moved.',
  'thread.synced': 'Whether every entry of the thread has reached its server.',
  'live.agent-editing': 'The agent editing the file open in the live editor.',
  'live.sync-devices': 'How many devices hold the live replica in sync.',
} as const satisfies Record<string, string>

export type MockId = keyof typeof MOCKS

export const mocksOn: boolean = import.meta.env.VITE_SILT_MOCKS !== 'off'

/** `value` standing in for data that doesn't exist yet, or nothing when mocks are off. */
export function mocked<T>(_id: MockId, value: T): T | undefined {
  return mocksOn ? value : undefined
}
