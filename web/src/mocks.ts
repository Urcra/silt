// What the boards draw that the daemon can't tell yet (ADR 0036). Each mock
// is listed here and in docs/ui-mocks.md, with what replacing it needs; a
// build with VITE_SILT_MOCKS=off leaves every mocked part out.

export const MOCKS = {
  'thread.handoff-before': 'The device a thread was on before its last handoff, and when it moved.',
  'thread.synced': 'Whether every entry of the thread has reached its server.',
  'add.discovered': "Clones found on this machine that aren't projects yet.",
  'add.visibility': "Who sees a project's threads, set per project.",
  'add.prepare': "Whether a project's repository has a prepare script new checkouts run.",
  'devices.checkouts': 'How many checkouts a device holds.',
  'devices.version': "A device's daemon version, and whether it is synced.",
  'devices.pair-capabilities': 'Choosing what a phone may do before it is paired.',
  'new.device': 'Choosing which of your devices a new thread starts on.',
  'graph.device': 'The device a turn ran on.',
  'search.saved': 'Searches the person saved, to run again with one click.',
  'reconcile.checkout': 'Which checkout followed the recovery thread when the work moved.',
  'reconcile.agent': "Whether the agent's session moved with the recovery thread.",
  'share.link-opens': 'How often a read-only link was opened.',
  'live.agent-editing': 'The agent editing the file open in the live editor.',
  'live.sync-devices': 'How many devices hold the live replica in sync.',
} as const satisfies Record<string, string>

export type MockId = keyof typeof MOCKS

export const mocksOn: boolean = import.meta.env.VITE_SILT_MOCKS !== 'off'

/** `value` standing in for data that doesn't exist yet, or nothing when mocks are off. */
export function mocked<T>(_id: MockId, value: T): T | undefined {
  return mocksOn ? value : undefined
}
