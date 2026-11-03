import type { ThreadSummary } from './api'
import { inProgress } from './threadLook'

// Where a thread shows in the sidebar, derived from its state and this
// person's (ADR 0024).

export type Section = 'needs_you' | 'pinned' | 'working' | 'active' | 'snoozed' | 'settled'

export function needsYou(thread: ThreadSummary): boolean {
  const state = thread.agent_state
  return state === 'awaiting_permission' || state === 'awaiting_input' || state === 'error' || thread.review
}

/** Snoozed and not yet woken, by time or by activity since. */
export function snoozed(thread: ThreadSummary, now: number): boolean {
  const { snoozed_until: until, snoozed_at: at, wake_on_activity: wakes } = thread.mine
  if (until === null || until <= now) return false
  return !(wakes && at !== null && thread.last_ms > at)
}

export function settled(thread: ThreadSummary): boolean {
  return thread.mine.settled || thread.merged
}

/** Something happened since the person last looked. */
export function unread(thread: ThreadSummary): boolean {
  return thread.mine.seen_ms !== null && thread.last_ms > thread.mine.seen_ms
}

/** What waits on the person shows first, whether snoozed, settled or pinned. */
export function section(thread: ThreadSummary, now: number): Section {
  if (needsYou(thread)) return 'needs_you'
  if (thread.mine.pinned) return 'pinned'
  if (inProgress(thread)) return 'working'
  if (settled(thread)) return 'settled'
  if (snoozed(thread, now)) return 'snoozed'
  return 'active'
}

/** The threads of each section, newest activity first. */
export function sections(threads: ThreadSummary[], now: number): Record<Section, ThreadSummary[]> {
  const out: Record<Section, ThreadSummary[]> = { needs_you: [], pinned: [], working: [], active: [], snoozed: [], settled: [] }
  for (const thread of [...threads].sort((a, b) => b.last_ms - a.last_ms)) out[section(thread, now)].push(thread)
  return out
}

/** Nine tomorrow morning, local time. */
export function tomorrowMorning(now: Date): number {
  const next = new Date(now)
  next.setDate(next.getDate() + 1)
  next.setHours(9, 0, 0, 0)
  return next.getTime()
}
