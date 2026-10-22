import type { ThreadSummary } from './api'
import type { DotTone } from './ui'

/** The dot before a thread: what it waits for, or whether its agent works. */
export function threadTone(thread: ThreadSummary): DotTone {
  if (thread.agent_state === 'error') return 'danger'
  if (thread.agent_state === 'awaiting_permission' || thread.agent_state === 'awaiting_input') return 'warn'
  if (thread.review) return 'accent'
  if (thread.agent_state === 'working' || thread.agent_state === 'starting') return 'ok'
  return 'idle'
}

/** What a thread is doing, in a few words, for the line under its title. */
export function threadDoing(thread: ThreadSummary): string | null {
  switch (thread.agent_state) {
    case 'awaiting_permission':
      return 'waiting on you'
    case 'awaiting_input':
      return 'plan to approve'
    case 'error':
      return 'stopped with an error'
    case 'working':
      return 'working'
    case 'starting':
      return 'starting'
  }
  if (thread.review) return 'review requested'
  if (thread.location === 'moved') return 'moved to another device'
  if (thread.location === 'remote') return 'not on this machine'
  return null
}

/** Where a thread's work is, as its chip says it. */
export function threadWhere(thread: ThreadSummary): { label: string; tone: DotTone } {
  const live = ['working', 'starting', 'awaiting_permission', 'awaiting_input'].includes(thread.agent_state ?? '')
  if (thread.location === 'moved') return { label: 'Moved to another device', tone: 'idle' }
  if (thread.location === 'remote') return { label: 'On the server', tone: 'info' }
  return { label: live ? 'Here, live' : 'Here', tone: 'ok' }
}
