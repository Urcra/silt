// The history graph's layout: which lane
// each thread runs in, and the curves between lanes, like `git log --graph`.

import type { GraphRow } from './api'

export interface Edge {
  /** The row it's drawn at, by index. */
  row: number
  from: number
  to: number
  kind: 'fork' | 'merge' | 'request'
  dashed: boolean
}

export interface Layout {
  /** Each row's lane, in the rows' order. */
  lanes: number[]
  /** Each thread's lane and the rows it spans, top and bottom. */
  threads: Map<string, { lane: number; top: number; bottom: number; abandoned: boolean }>
  edges: Edge[]
  width: number
}

/** Rows newest first, each family's together, the one worked in last first: threads of unrelated projects
 * never share a lane's line, which would read as one history. */
export function byFamily(rows: GraphRow[]): GraphRow[] {
  const families = new Map<string, GraphRow[]>()
  for (const row of rows) families.set(row.family, [...(families.get(row.family) ?? []), row])
  return [...families.values()].flat()
}

/** Lays out rows given newest first: a lane per thread, reused once its thread's oldest row is past. */
export function layout(rows: GraphRow[]): Layout {
  const touched = new Map<string, { top: number; bottom: number }>()
  const touch = (thread: string, index: number) => {
    const span = touched.get(thread)
    if (span) span.bottom = index
    else touched.set(thread, { top: index, bottom: index })
  }
  rows.forEach((row, index) => {
    touch(row.thread, index)
    // A fork leaves its source, a merge takes in its fork, a request points at its target.
    if (row.other) touch(row.other, index)
  })

  const abandoned = new Set<string>()
  const approved = new Set<string>()
  for (const row of rows) {
    if (row.settled) abandoned.add(row.thread)
    if (row.kind === 'request' && row.status === 'declined') abandoned.add(row.thread)
    if (row.kind === 'request' && row.status === 'approved') approved.add(row.thread)
  }
  for (const thread of approved) abandoned.delete(thread)

  const lane = new Map<string, number>()
  const busy: (string | null)[] = []
  let width = 0
  const lanes = rows.map((row, index) => {
    for (const thread of [row.thread, row.other]) {
      if (!thread || lane.has(thread) || touched.get(thread)?.top !== index) continue
      const free = busy.indexOf(null)
      const chosen = free === -1 ? busy.length : free
      busy[chosen] = thread
      lane.set(thread, chosen)
      width = Math.max(width, chosen + 1)
    }
    const own = lane.get(row.thread) ?? 0
    // Lanes whose threads end here are free from the next row on.
    for (const [thread, span] of touched) {
      if (span.bottom === index) {
        const at = lane.get(thread)
        if (at !== undefined && busy[at] === thread) busy[at] = null
      }
    }
    return own
  })

  const edges: Edge[] = []
  rows.forEach((row, index) => {
    if (!row.other || !lane.has(row.other)) return
    const ownLane = lane.get(row.thread) ?? 0
    const otherLane = lane.get(row.other) ?? 0
    if (row.kind === 'fork') edges.push({ row: index, from: otherLane, to: ownLane, kind: 'fork', dashed: false })
    if (row.kind === 'merge') edges.push({ row: index, from: otherLane, to: ownLane, kind: 'merge', dashed: false })
    if (row.kind === 'request') {
      edges.push({ row: index, from: ownLane, to: otherLane, kind: 'request', dashed: row.status !== 'approved' })
    }
  })

  const threads = new Map<string, { lane: number; top: number; bottom: number; abandoned: boolean }>()
  for (const [thread, span] of touched) {
    threads.set(thread, { lane: lane.get(thread) ?? 0, ...span, abandoned: abandoned.has(thread) })
  }
  return { lanes, threads, edges, width }
}
