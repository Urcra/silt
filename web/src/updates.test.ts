import { expect, test } from 'vitest'
import type { Availability, RunView, UpdateView } from './api'
import { offReason, pillOf, sameCommit, took, updateState } from './updates'

const OLD = 'a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2'
const NEW = 'f00ba12f00ba12f00ba12f00ba12f00ba12f00b'

function view(changes: Partial<UpdateView> = {}): UpdateView {
  return {
    build: { version: '0.0.1', channel: 'nightly', commit: OLD, branch: 'main', dirty: false, source: '/src/silt' },
    availability: { kind: 'up_to_date' },
    checked_ms: 0,
    checking: false,
    run: null,
    output: [],
    output_path: '/data/update/output.log',
    relaunch: false,
    ...changes,
  }
}

function run(outcome: RunView['outcome'], changes: Partial<RunView> = {}): RunView {
  return { outcome, from: OLD, to: NEW, started_ms: 0, elapsed_ms: 1000, expected_ms: null, percent: 48, steps: [], ...changes }
}

const behind: Availability = { kind: 'behind', target: NEW, ahead: 3, commits: [] }

test('an update under way comes before anything a check says', () => {
  expect(updateState(view({ availability: behind, run: run({ kind: 'running' }) }))).toBe('running')
  expect(updateState(view({ run: run({ kind: 'waiting', agents: 2 }) }))).toBe('waiting')
  expect(pillOf(view({ run: run({ kind: 'running' }) }))).toEqual({ kind: 'running', text: 'Updating · 48%' })
})

test('main ahead shows the Update button; up to date or off shows none', () => {
  expect(updateState(view({ availability: behind }))).toBe('behind')
  expect(pillOf(view({ availability: behind }))?.kind).toBe('update')
  expect(pillOf(view())).toBeUndefined()
  expect(pillOf(view({ availability: { kind: 'off', reason: { kind: 'local_changes' } } }))).toBeUndefined()
  expect(pillOf(view({ availability: { kind: 'release', tag: 'v0.3.0', url: 'https://example.com' } }))?.kind).toBe('update')
})

test('a finished update asks for a relaunch only when the app changed', () => {
  const done = run({ kind: 'done' })
  const after = { build: { ...view().build, commit: NEW }, run: done }
  expect(updateState(view({ ...after, relaunch: true }))).toBe('relaunch')
  expect(pillOf(view({ ...after, relaunch: true }))?.kind).toBe('relaunch')
  expect(updateState(view(after))).toBe('done')
  expect(pillOf(view(after))).toBeUndefined()
})

test('a failure shows only while the build it failed to replace runs', () => {
  const failed = run({ kind: 'failed', message: 'Build silt failed: error' })
  expect(updateState(view({ availability: behind, run: failed }))).toBe('failed')
  expect(updateState(view({ build: { ...view().build, commit: NEW }, run: failed }))).toBe('up-to-date')
})

test('commits match by their prefix, and reasons and durations read plainly', () => {
  expect(sameCommit(OLD, 'a1b2c3d')).toBe(true)
  expect(sameCommit(OLD, NEW)).toBe(false)
  expect(sameCommit(null, OLD)).toBe(false)
  expect(offReason({ kind: 'off_main', branch: 'feat/x' })).toBe('Its clone is on `feat/x`, not `main`.')
  expect(offReason({ kind: 'local_commits', count: 1 })).toContain('1 commit its')
  expect(took(2_000)).toBe('2s')
  expect(took(220_000)).toBe('3m 40s')
})
