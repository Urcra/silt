// How a plugin call is told in a line, for the Access tab and a plugin's page.

import type { CallView } from './api'

export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

export function callSaid(call: CallView): { tone: 'ok' | 'warn' | 'danger' | 'neutral'; said: string } {
  const outcome = call.outcome
  if (call.state === 'pending') return { tone: 'warn', said: 'waiting for you' }
  if (call.state === 'running') return { tone: 'warn', said: 'running' }
  if (call.state === 'expired') return { tone: 'neutral', said: 'approval expired' }
  if (!outcome) return { tone: 'neutral', said: call.state }
  switch (outcome.status) {
    case 'done':
      return {
        tone: 'ok',
        said: call.asked ? 'you approved · ran' : call.grant !== null ? 'ran under a grant' : 'ran without asking',
      }
    case 'refused':
      return { tone: 'danger', said: `refused: ${outcome.why}` }
    case 'failed':
      return { tone: 'danger', said: `failed before it was sent: ${outcome.error}` }
    case 'unknown':
      return { tone: 'warn', said: `outcome unknown: ${outcome.why}` }
    case 'pending':
      return { tone: 'warn', said: 'waiting for you' }
  }
}

export const TONE_DOT = { ok: 'bg-ok', warn: 'bg-warn', danger: 'bg-danger', neutral: 'bg-fg-3' } as const
