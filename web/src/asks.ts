// Agents asking other agents (ADR 0117): which call asked which agent, and which thread answers it.
import { useQuery } from '@tanstack/react-query'
import { api } from './api'
import type { AskView, HistoryEntry } from './api.generated'
import { askOfCall } from './askCall'
import { keys } from './query'

export { askOfCall, siltTool, type AskCall } from './askCall'

/** Every ask on this machine, kept fresh by the bus. */
export function useAsks(): AskView[] {
  const asks = useQuery({ queryKey: keys.asks, queryFn: api.asks })
  return asks.data ?? []
}

/** The ask a call of `thread`'s made: the one with its prompt, started after the call. */
export function askFor(asks: AskView[], thread: string, call: HistoryEntry): AskView | undefined {
  const asked = askOfCall(call)
  if (!asked) return undefined
  if (asked.to) return asks.find((ask) => ask.id === asked.to && ask.caller === thread)
  const prompt = asked.prompt.trim()
  const candidates = asks.filter((ask) => ask.caller === thread && ask.prompt === prompt)
  // The call is recorded as it starts; the ask a moment after.
  return candidates.find((ask) => ask.started_ms >= call.time_ms - 5_000) ?? candidates.at(-1)
}

/** The ask `thread` answers, when another agent asked for it. */
export function askedIn(asks: AskView[], thread: string): AskView | undefined {
  return asks.findLast((ask) => ask.asked === thread)
}

export const MODE_LABEL: Record<AskView['mode'], string> = { review: 'review', work: 'work' }
