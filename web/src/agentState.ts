import type { AgentState } from './api'
import type { Tone } from './ui'

/** How a state reads after the agent's name (ADR 0022). */
export const STATE_PHRASE: Record<AgentState, string> = {
  starting: 'is starting',
  idle: 'is ready',
  working: 'is working',
  awaiting_permission: 'is waiting for your permission',
  awaiting_input: 'is waiting for you',
  error: 'stopped with an error',
  stopped: 'stopped',
}

/** A short label for lists, and how much attention it asks for. */
export const STATE_TAG: Record<AgentState, { label: string; tone: Tone }> = {
  starting: { label: 'starting', tone: 'neutral' },
  idle: { label: 'ready', tone: 'neutral' },
  working: { label: 'working', tone: 'info' },
  awaiting_permission: { label: 'needs you', tone: 'accent' },
  awaiting_input: { label: 'needs you', tone: 'accent' },
  error: { label: 'error', tone: 'danger' },
  stopped: { label: 'stopped', tone: 'neutral' },
}

export function running(state: AgentState): boolean {
  return state !== 'error' && state !== 'stopped'
}
