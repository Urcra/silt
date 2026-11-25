// A project's asked threads folded under the thread that asked them (ADR 0120), so an agent that asks often
// takes one row of the sidebar until it's opened.
import { useSyncExternalStore } from 'react'
import type { ThreadSummary } from './api'
import type { AskView } from './api.generated'
import { askedIn } from './asks'
import { needsYou } from './sections'
import { store, stored } from './stored'

export type Asked = { thread: ThreadSummary; ask: AskView }

/** A thread of the list, with the threads asked for under it, newest first. */
export type Caller = { thread: ThreadSummary; asked: Asked[] }

/** How many asks an opened fold lists before "Show N more". */
export const ASKS_SHOWN = 5

/**
 * The threads in their order, each asked thread under the thread at the top of its chain of callers. An asked
 * thread whose caller isn't in the list stands alone, as do they all while the asks are loading.
 */
export function byCaller(threads: ThreadSummary[], asks: AskView[]): Caller[] {
  const listed = new Map(threads.map((thread) => [thread.id, thread]))
  const callerOf = (id: string) => {
    const caller = askedIn(asks, id)?.caller
    return caller && caller !== id && listed.has(caller) ? caller : undefined
  }
  const topOf = (id: string) => {
    const seen = new Set([id])
    for (let up = callerOf(id); up && !seen.has(up); up = callerOf(up)) {
      seen.add(up)
      id = up
    }
    return id
  }
  const callers = new Map<string, Caller>()
  for (const thread of threads) if (!callerOf(thread.id)) callers.set(thread.id, { thread, asked: [] })
  for (const thread of threads) {
    const ask = askedIn(asks, thread.id)
    if (ask && callerOf(thread.id)) callers.get(topOf(thread.id))?.asked.push({ thread, ask })
  }
  for (const caller of callers.values()) caller.asked.sort((a, b) => b.ask.started_ms - a.ask.started_ms)
  return [...callers.values()]
}

/** An asked thread waiting on the person, for an answer, a permission or a review; a failed one isn't. */
function waitsOnYou(thread: ThreadSummary): boolean {
  return needsYou(thread) && thread.agent_state !== 'error'
}

/** The one thing a fold row says about its asks, the most pressing first. */
export type FoldSaid = { kind: 'needs_you' | 'failed' | 'working'; count: number } | { kind: 'reported' } | null

export function foldSaid(asked: Asked[]): FoldSaid {
  const count = (kind: 'needs_you' | 'failed' | 'working', has: (asked: Asked) => boolean) => {
    const n = asked.filter(has).length
    return n > 0 ? { kind, count: n } : null
  }
  return (
    count('needs_you', ({ thread }) => waitsOnYou(thread)) ??
    count('failed', ({ ask }) => ask.state === 'failed') ??
    count('working', ({ ask }) => ask.state === 'running') ??
    (asked.every(({ ask }) => ask.state === 'done') ? { kind: 'reported' } : null)
  )
}

/**
 * The asked threads a fold lists: when open, the first few or all of them; whether open or not, the ones waiting
 * on the person and the one open in the page. `more` is how many it holds back.
 */
export function foldListed(asked: Asked[], open: boolean, all: boolean, current: string | null): { rows: Asked[]; more: number } {
  const always = ({ thread }: Asked) => waitsOnYou(thread) || thread.id === current
  const rows = asked.filter((one, at) => (open && (all || at < ASKS_SHOWN)) || always(one))
  return { rows, more: open && !all ? asked.length - rows.length : 0 }
}

const KEY = 'silt.asks-open'
const listeners = new Set<() => void>()
let opened: Record<string, true> = stored(KEY, {})

function setOpen(caller: string, open: boolean) {
  const next = { ...opened }
  if (open) next[caller] = true
  else delete next[caller]
  opened = next
  store(KEY, opened)
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** Whether `caller`'s asks are open, folded until the person opens them and kept in this browser. */
export function useAsksOpen(caller: string): [boolean, (open: boolean) => void] {
  const open = useSyncExternalStore(subscribe, () => opened[caller] === true)
  return [open, (next) => setOpen(caller, next)]
}
