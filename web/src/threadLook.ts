import type { AgentStatus, ThreadSummary } from './api'
import type { DotTone, IconName } from './ui'

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

/** What a thread waiting on the person waits for. */
export type WaitKind = 'sign-in' | 'question' | 'plan' | 'permission' | 'review' | 'stopped'

/** Its kind, from the agent's status where it is known and from the thread's own state else. */
export function waitKind(thread: ThreadSummary, status: AgentStatus | null): WaitKind {
  if (status?.sign_in) return 'sign-in'
  if ((status?.questions.length ?? 0) > 0) return 'question'
  if (thread.agent_state === 'awaiting_input' || status?.state === 'awaiting_input' || status?.permissions[0]?.plan) return 'plan'
  if (thread.agent_state === 'awaiting_permission') return 'permission'
  if (thread.agent_state === 'error') return 'stopped'
  return 'review'
}

/** Each kind's icon, its word in the sidebar and its heading in the queue, in the queue's order. */
export const WAIT_KINDS: Record<WaitKind, { icon: IconName; word: string; heading: string }> = {
  'sign-in': { icon: 'lock', word: 'sign in', heading: 'Sign-ins' },
  question: { icon: 'comment', word: 'asks you', heading: 'Questions' },
  plan: { icon: 'list', word: 'plan to approve', heading: 'Plans' },
  permission: { icon: 'lock', word: 'asks to run', heading: 'Permissions' },
  review: { icon: 'merge', word: 'review', heading: 'Reviews' },
  stopped: { icon: 'alert', word: 'stopped', heading: 'Stopped' },
}
