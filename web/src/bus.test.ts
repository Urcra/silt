import { QueryClient, QueryObserver } from '@tanstack/react-query'
import { afterEach, beforeEach, describe, expect, it, test, vi } from 'vitest'
import type { BusEvent } from './api'
import { ASLEEP_MS, Bus, QUIET_MS, readAgain, Refetcher } from './bus'

test('a change announced while its fetch is under way is fetched once that fetch ends', async () => {
  const queries = new QueryClient()
  let stored = 'before'
  const pending: (() => void)[] = []
  const queryFn = vi.fn(() => {
    const read = stored
    return new Promise<string>((resolve) => pending.push(() => resolve(read)))
  })
  const observer = new QueryObserver(queries, { queryKey: ['thread', 't', 'history'], queryFn })
  const unsubscribe = observer.subscribe(() => {})
  const refetcher = new Refetcher(queries)
  const stop = refetcher.start()
  const answer = async () => {
    await vi.waitFor(() => expect(pending.length).toBeGreaterThan(0))
    pending.shift()!()
  }
  await answer()
  await vi.waitFor(() => expect(observer.getCurrentResult().data).toBe('before'))

  refetcher.refetch(['thread', 't'])
  await vi.waitFor(() => expect(queryFn).toHaveBeenCalledTimes(2))
  stored = 'after'
  refetcher.refetch(['thread', 't'])
  expect(queryFn).toHaveBeenCalledTimes(2)
  await answer()
  await answer()
  await vi.waitFor(() => expect(observer.getCurrentResult().data).toBe('after'))
  expect(queryFn).toHaveBeenCalledTimes(3)

  stop()
  unsubscribe()
})

test('a read that never ends is given up and read again once the page comes back', async () => {
  const queries = new QueryClient()
  let calls = 0
  const queryFn = vi.fn(({ signal }: { signal: AbortSignal }) => {
    calls += 1
    if (calls === 1) return Promise.resolve('before')
    // A read sent as the phone slept: it waits on a connection that's gone, and only its signal ends it.
    if (calls === 2) return new Promise<string>((_, reject) => signal.addEventListener('abort', () => reject(signal.reason)))
    return Promise.resolve('after')
  })
  const observer = new QueryObserver(queries, { queryKey: ['thread', 't', 'history'], queryFn })
  const unsubscribe = observer.subscribe(() => {})
  const refetcher = new Refetcher(queries)
  const stop = refetcher.start()
  await vi.waitFor(() => expect(observer.getCurrentResult().data).toBe('before'))

  refetcher.refetch(['thread', 't'])
  await vi.waitFor(() => expect(queryFn).toHaveBeenCalledTimes(2))
  refetcher.refetch(['thread', 't'])
  expect(queryFn).toHaveBeenCalledTimes(2)

  await readAgain(queries)
  await vi.waitFor(() => expect(observer.getCurrentResult().data).toBe('after'))
  expect(observer.getCurrentResult().error).toBeNull()

  stop()
  unsubscribe()
})

/** A stand-in for the browser's WebSocket: every one opened, each opening, speaking or closing when the test says. */
class FakeSocket {
  static CONNECTING = 0
  static OPEN = 1
  static opened: FakeSocket[] = []
  readyState = FakeSocket.CONNECTING
  sent: string[] = []
  closed = false
  onopen: (() => void) | null = null
  onmessage: ((message: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  constructor() {
    FakeSocket.opened.push(this)
  }
  send(text: string) {
    this.sent.push(text)
  }
  close() {
    this.closed = true
  }
  open() {
    this.readyState = FakeSocket.OPEN
    this.onopen?.()
  }
  hear(heard: unknown) {
    this.onmessage?.({ data: JSON.stringify(heard) })
  }
}

describe("the bus's socket", () => {
  let heard: BusEvent[]
  let resumed: number
  let bus: Bus
  beforeEach(() => {
    FakeSocket.opened = []
    vi.useFakeTimers()
    vi.stubGlobal('WebSocket', FakeSocket)
    heard = []
    resumed = 0
    bus = new Bus()
    bus.listen((event) => heard.push(event))
    bus.onResume(() => (resumed += 1))
    FakeSocket.opened[0].open()
  })
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('is given up once quiet past the beat, and the page reads again once another opens', () => {
    vi.advanceTimersByTime(QUIET_MS + 5000)
    expect(FakeSocket.opened[0].closed).toBe(true)
    expect(bus.connected()).toBe(false)
    vi.advanceTimersByTime(1000)
    expect(FakeSocket.opened).toHaveLength(2)
    expect(resumed).toBe(0)
    FakeSocket.opened[1].open()
    expect(resumed).toBe(1)
    expect(heard).toEqual([{ event: 'lagged' }])
  })

  it('stays open while it beats, and the beats go no further', () => {
    for (let beat = 0; beat < 6; beat += 1) {
      vi.advanceTimersByTime(10_000)
      FakeSocket.opened[0].hear({ event: 'beat' })
    }
    expect(FakeSocket.opened).toHaveLength(1)
    expect(FakeSocket.opened[0].closed).toBe(false)
    expect(heard).toEqual([])
  })

  it('reads again when shown after sleeping, not after a glance away', () => {
    bus.shown(false)
    vi.advanceTimersByTime(ASLEEP_MS - 1000)
    FakeSocket.opened[0].hear({ event: 'beat' })
    bus.shown(true)
    expect(resumed).toBe(0)

    bus.shown(false)
    vi.advanceTimersByTime(ASLEEP_MS + 1000)
    FakeSocket.opened[0].hear({ event: 'beat' })
    bus.shown(true)
    expect(resumed).toBe(1)
    expect(heard).toEqual([{ event: 'lagged' }])
  })

  it('woken with a socket that died as it slept, opens another first', () => {
    bus.shown(false)
    // Timers don't run while a phone sleeps; the clock does.
    vi.setSystemTime(Date.now() + 10 * 60_000)
    bus.shown(true)
    expect(FakeSocket.opened[0].closed).toBe(true)
    expect(resumed).toBe(0)
    vi.advanceTimersByTime(1000)
    FakeSocket.opened[1].open()
    expect(resumed).toBe(1)
  })
})
