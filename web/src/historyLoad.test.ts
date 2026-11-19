import { describe, expect, it } from 'vitest'
import type { HistoryEntry, HistoryPage } from './api'
import { FIRST_PAGE, type Loaded, loadHistory } from './historyLoad'

const entry = (index: number, text = `entry ${index}`): HistoryEntry =>
  ({ index, thread: 't', origin: 'own', kind: 'assistant', text, time_ms: index }) as HistoryEntry

/** A daemon holding `entries` at `version`, which changed from `changedFrom` since version `before`. */
function daemon(entries: HistoryEntry[], version: number | null, before?: { version: number; changedFrom: number }) {
  const asked: [number, number, number | null][] = []
  const fetch = async (offset: number, limit: number, known: number | null): Promise<HistoryPage> => {
    asked.push([offset, limit, known])
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
    }
  }
  return { fetch, asked }
}

describe('loadHistory', () => {
  const all = Array.from({ length: 1200 }, (_, index) => entry(index))

  it('reads a first small page, shows it, then reads the rest', async () => {
    const { fetch, asked } = daemon(all, 7)
    const shown: number[] = []
    const loaded = await loadHistory(fetch, undefined, (partial) => shown.push(partial.entries.length))
    expect(loaded.entries).toEqual(all)
    expect(loaded.version).toBe(7)
    expect(asked.map(([offset]) => offset)).toEqual([0, FIRST_PAGE, FIRST_PAGE + 500, FIRST_PAGE + 1000])
    expect(shown).toEqual([50, 550, 1050])
  })

  it('answers an unchanged history with what it had, after one small question', async () => {
    const before: Loaded = { thread: 't', total: all.length, version: 7, entries: all }
    const { fetch, asked } = daemon(all, 7)
    expect(await loadHistory(fetch, before, () => undefined)).toBe(before)
    expect(asked).toEqual([[1200, 500, 7]])
  })

  it('reads only what was added, keeping the entries it had', async () => {
    const before: Loaded = { thread: 't', total: all.length, version: 7, entries: all }
    const grown = [...all, entry(1200), entry(1201)]
    const { fetch, asked } = daemon(grown, 9, { version: 7, changedFrom: 1200 })
    const loaded = await loadHistory(fetch, before, () => undefined)
    expect(loaded.entries).toHaveLength(1202)
    expect(loaded.entries[3]).toBe(all[3])
    expect(asked).toEqual([[1200, 500, 7]])
  })

  it('reads again from an entry that changed earlier, as a revert rewinds', async () => {
    const before: Loaded = { thread: 't', total: all.length, version: 7, entries: all }
    const rewound = all.map((old, index) => (index >= 1000 ? { ...old, rewound: true } : old))
    const { fetch } = daemon(rewound, 8, { version: 7, changedFrom: 1000 })
    const loaded = await loadHistory(fetch, before, () => undefined)
    expect(loaded.entries).toEqual(rewound)
    expect(loaded.entries[999]).toBe(all[999])
    expect(loaded.entries[1000].rewound).toBe(true)
  })

  it('reads everything again when the daemon no longer knows the version held', async () => {
    const before: Loaded = { thread: 't', total: all.length, version: 3, entries: all }
    const changed = all.map((old) => ({ ...old, text: `${old.text}!` }))
    const { fetch } = daemon(changed, 9, { version: 7, changedFrom: 1000 })
    expect((await loadHistory(fetch, before, () => undefined)).entries).toEqual(changed)
  })

  it("reads a server's history whole each time, as it names no version", async () => {
    const before: Loaded = { thread: 't', total: 2, version: null, entries: [entry(0), entry(1, 'old')] }
    const { fetch, asked } = daemon([entry(0), entry(1)], null)
    const loaded = await loadHistory(fetch, before, () => undefined)
    expect(loaded.entries[1].text).toBe('entry 1')
    expect(asked).toEqual([[0, FIRST_PAGE, null]])
  })

  it('shrinks with a history that did', async () => {
    const before: Loaded = { thread: 't', total: all.length, version: 7, entries: all }
    const { fetch } = daemon(all.slice(0, 10), 2)
    expect((await loadHistory(fetch, before, () => undefined)).entries).toHaveLength(10)
  })
})
