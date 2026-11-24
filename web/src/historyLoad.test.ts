import { describe, expect, it } from 'vitest'
import type { HistoryEntry, HistoryPage } from './api'
import { type Ask, FIRST_PAGE, type Loaded, PAGE, loadEarlier, loadHistory, withEarlier } from './historyLoad'

const entry = (index: number, text = `entry ${index}`, kind = index % 10 === 0 ? 'user' : 'assistant'): HistoryEntry =>
  ({ index, thread: 't', origin: 'own', kind, text, time_ms: index }) as HistoryEntry

/** The start of the turn holding entry `at`, as the daemon finds it for a tail. */
function turnStart(entries: HistoryEntry[], at: number): number {
  let from = Math.max(0, at)
  while (from > 0 && entries[from].kind !== 'user') from--
  return from
}

/** A daemon holding `entries` at `version`, which changed from `changedFrom` since version `before`. */
function daemon(entries: HistoryEntry[], version: number | null, before?: { version: number; changedFrom: number }) {
  const asked: Ask[] = []
  const tasksBefore = (from: number) => entries.slice(0, from).filter((held) => held.task)
  const fetch = async (ask: Ask): Promise<HistoryPage> => {
    asked.push(ask)
    if ('tail' in ask) {
      const end = Math.min(ask.before ?? entries.length, entries.length)
      const from = turnStart(entries, end - ask.tail)
      const page = entries.slice(from, end)
      return { thread: 't', total: entries.length, entries: page, from, version, tasks: tasksBefore(from) }
    }
    const { offset, limit, known } = ask
    let from = offset
    if (version !== null && known !== null && known !== version)
      from = known === before?.version ? Math.min(offset, before.changedFrom) : 0
    from = Math.min(from, entries.length)
    return {
      thread: 't',
      total: entries.length,
      entries: entries.slice(from, from + limit),
      from: version === null ? offset : from,
      version,
      tasks: tasksBefore(from),
    }
  }
  return { fetch, asked }
}

const held = (all: HistoryEntry[], start: number, version: number | null): Loaded => ({
  thread: 't',
  total: all.length,
  version,
  start,
  entries: all.slice(start),
  tasks: all.slice(0, start).filter((entry) => entry.task),
})

describe('loadHistory', () => {
  const all = Array.from({ length: 1200 }, (_, index) => entry(index))

  it('opens on the last page, from the start of its turn, in one read', async () => {
    const { fetch, asked } = daemon(all, 7)
    const shown: number[] = []
    const loaded = await loadHistory(fetch, undefined, (partial) => shown.push(partial.entries.length))
    expect(asked).toEqual([{ tail: FIRST_PAGE }])
    expect(loaded.start).toBe(1150)
    expect(loaded.entries).toEqual(all.slice(1150))
    expect(loaded.entries[0].kind).toBe('user')
    expect(loaded.version).toBe(7)
    expect(shown).toEqual([])
  })

  it('keeps the news of tasks started before its last page', async () => {
    const started = {
      ...entry(12),
      task: { id: 'b1', call: 'c1', kind: 'background', status: 'running', side: null, summary: null },
    }
    const withTask = all.map((old) => (old.index === 12 ? started : old))
    const loaded = await loadHistory(daemon(withTask, 7).fetch, undefined, () => undefined)
    expect(loaded.tasks).toEqual([started])
  })

  it('answers an unchanged history with what it had, after one small question', async () => {
    const before = held(all, 1150, 7)
    const { fetch, asked } = daemon(all, 7)
    expect(await loadHistory(fetch, before, () => undefined)).toBe(before)
    expect(asked).toEqual([{ offset: 1200, limit: PAGE, known: 7 }])
  })

  it('reads only what was added to the page it holds, keeping the entries it had', async () => {
    const before = held(all, 1150, 7)
    const grown = [...all, entry(1200), entry(1201)]
    const { fetch, asked } = daemon(grown, 9, { version: 7, changedFrom: 1200 })
    const loaded = await loadHistory(fetch, before, () => undefined)
    expect(loaded.start).toBe(1150)
    expect(loaded.entries).toHaveLength(52)
    expect(loaded.entries[3]).toBe(before.entries[3])
    expect(asked).toEqual([{ offset: 1200, limit: PAGE, known: 7 }])
  })

  it('reads again from an entry that changed within what it holds, as a revert rewinds', async () => {
    const before = held(all, 900, 7)
    const rewound = all.map((old, index) => (index >= 1000 ? { ...old, rewound: true } : old))
    const { fetch } = daemon(rewound, 8, { version: 7, changedFrom: 1000 })
    const loaded = await loadHistory(fetch, before, () => undefined)
    expect(loaded.start).toBe(900)
    expect(loaded.entries).toEqual(rewound.slice(900))
    expect(loaded.entries[99]).toBe(all[999])
    expect(loaded.entries[100].rewound).toBe(true)
  })

  it('reads its page again from the end when what changed came before it', async () => {
    const before = held(all, 1150, 7)
    const changed = all.map((old) => ({ ...old, text: `${old.text}!` }))
    const { fetch, asked } = daemon(changed, 8, { version: 7, changedFrom: 400 })
    const loaded = await loadHistory(fetch, before, () => undefined)
    expect(asked[asked.length - 1]).toEqual({ tail: 50 })
    expect(loaded.entries).toEqual(changed.slice(1150))
  })

  it('reads its page again when the daemon no longer knows the version held', async () => {
    const before = held(all, 1150, 3)
    const changed = all.map((old) => ({ ...old, text: `${old.text}!` }))
    const { fetch } = daemon(changed, 9, { version: 7, changedFrom: 1000 })
    expect((await loadHistory(fetch, before, () => undefined)).entries).toEqual(changed.slice(1150))
  })

  it("reads what it holds of a server's history again whole, as it names no version", async () => {
    const server = [entry(0), entry(1), entry(2)]
    const before: Loaded = { ...held(server, 1, null), entries: [entry(1, 'old'), entry(2)] }
    const { fetch, asked } = daemon(server, null)
    const loaded = await loadHistory(fetch, before, () => undefined)
    expect(loaded.entries.map((shown) => shown.text)).toEqual(['entry 1', 'entry 2'])
    expect(asked).toEqual([{ offset: 1, limit: Number.MAX_SAFE_INTEGER, known: null }])
  })

  it('shrinks with a history that did', async () => {
    const before = held(all, 1150, 7)
    const { fetch } = daemon(all.slice(0, 10), 2)
    const loaded = await loadHistory(fetch, before, () => undefined)
    expect(loaded.start + loaded.entries.length).toBeLessThanOrEqual(10)
  })
})

describe('loadEarlier', () => {
  const all = Array.from({ length: 1200 }, (_, index) => entry(index))

  it('reads the page before what it holds, from the start of a turn, and puts it first', async () => {
    const before = held(all, 1150, 7)
    const { fetch, asked } = daemon(all, 7)
    const loaded = await loadEarlier(fetch, before)
    expect(asked).toEqual([{ tail: PAGE, before: 1150 }])
    expect(loaded.start).toBe(650)
    expect(loaded.entries).toEqual(all.slice(650))
    expect(loaded.entries[500]).toBe(before.entries[0])
  })

  it('reads nothing once it holds the history from its start', async () => {
    const before = held(all, 0, 7)
    const { fetch, asked } = daemon(all, 7)
    expect(await loadEarlier(fetch, before)).toBe(before)
    expect(asked).toEqual([])
  })

  it('takes the news of tasks from before its new start', async () => {
    const started = {
      ...entry(3),
      task: { id: 'b1', call: 'c1', kind: 'background', status: 'running', side: null, summary: null },
    }
    const withTask = all.map((old) => (old.index === 3 ? started : old))
    const before = held(withTask, 600, 7)
    expect(before.tasks).toEqual([started])
    const loaded = await loadEarlier(daemon(withTask, 7).fetch, before)
    expect(loaded.start).toBe(100)
    expect(loaded.tasks).toEqual([started])
  })
})

describe('withEarlier', () => {
  const all = Array.from({ length: 1200 }, (_, index) => entry(index))

  it('keeps the earlier page read while an update ran', () => {
    const updated = { ...held(all, 1150, 8) }
    const earlier = held(all, 650, 7)
    const merged = withEarlier(updated, earlier)
    expect(merged.start).toBe(650)
    expect(merged.entries).toHaveLength(550)
    expect(merged.version).toBe(8)
  })

  it('leaves an update alone that holds as much, or what holds a gap before it', () => {
    const updated = held(all, 1150, 8)
    expect(withEarlier(updated, held(all, 1150, 7))).toBe(updated)
    const gap = { ...held(all, 600, 7), entries: all.slice(600, 900) }
    expect(withEarlier(updated, gap)).toBe(updated)
  })
})
