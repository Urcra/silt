// State a page keeps when a new version of it arrives while it's open (ADR 0072): the holding page asks the frame for
// its snapshot now and then, and hands the last one to the next version's frame.

/** The largest snapshot kept, as JSON. */
export const SNAPSHOT_BYTES = 1 << 20

const snapshots = new Map<string, unknown>()

export function rememberSnapshot(thread: string, artifact: string, data: unknown): void {
  let size = Infinity
  try {
    size = JSON.stringify(data ?? null).length
  } catch {
    size = Infinity
  }
  if (size <= SNAPSHOT_BYTES) snapshots.set(`${thread}:${artifact}`, data ?? null)
}

export function lastSnapshot(thread: string, artifact: string): unknown {
  return snapshots.get(`${thread}:${artifact}`) ?? null
}
