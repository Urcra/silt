import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PeerBook, PRESENCE_MS, REJOIN_MAX_MS, REJOIN_MS, RoomSocket } from './rooms'

const peer = (name: string, state: unknown = null) => ({ peer: name, user: `u-${name}`, kind: 'person', state })

describe("a room's peers, as one view knows them", () => {
  it('take the list when someone comes or goes, then one presence at a time', () => {
    const book = new PeerBook()
    expect(book.heard({ event: 'peers', peers: [peer('ann'), peer('bob')] }, 0)).toBe(true)
    expect(book.heard({ event: 'presence', peer: peer('bob', { cursor: [1, 2] }) }, 10)).toBe(true)
    expect(book.list(10)).toEqual([peer('ann'), peer('bob', { cursor: [1, 2] })])
    expect(book.heard({ event: 'presence', peer: peer('carol', 1) }, 20)).toBe(false)
    expect(book.list(20).map((one) => one.peer)).toEqual(['ann', 'bob'])
  })

  it('lapse when a peer has said nothing for a while', () => {
    const book = new PeerBook()
    book.heard({ event: 'peers', peers: [peer('ann')] }, 0)
    book.heard({ event: 'presence', peer: peer('ann', 'here') }, 0)
    expect(book.lapsed(0, PRESENCE_MS - 1)).toBe(false)
    expect(book.lapsed(PRESENCE_MS - 1, PRESENCE_MS + 1)).toBe(true)
    expect(book.list(PRESENCE_MS + 1)).toEqual([peer('ann')])
    book.heard({ event: 'moment', topic: 't', data: null, from: peer('ann') }, PRESENCE_MS + 2)
    expect(book.list(PRESENCE_MS + 3)).toEqual([peer('ann', 'here')])
  })
})

/** A stand-in for the browser's WebSocket: every one opened, each opening or closing when the test says. */
class FakeSocket {
  static CONNECTING = 0
  static OPEN = 1
  static opened: FakeSocket[] = []
  readyState = FakeSocket.CONNECTING
  sent: string[] = []
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
    this.readyState = 3
  }
  open() {
    this.readyState = FakeSocket.OPEN
    this.onopen?.()
  }
  drop() {
    this.readyState = 3
    this.onclose?.()
  }
  hear(heard: unknown) {
    this.onmessage?.({ data: JSON.stringify(heard) })
  }
}

describe("a room's socket", () => {
  beforeEach(() => {
    FakeSocket.opened = []
    vi.useFakeTimers()
    vi.stubGlobal('WebSocket', FakeSocket)
  })
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('opens again once dropped, joins again and sends what was said meanwhile', () => {
    const room = new RoomSocket('t1', 'board')
    FakeSocket.opened[0].open()
    FakeSocket.opened[0].drop()
    room.presence({ cursor: [1, 2] })
    expect(FakeSocket.opened).toHaveLength(1)
    vi.advanceTimersByTime(REJOIN_MS)
    expect(FakeSocket.opened).toHaveLength(2)
    FakeSocket.opened[1].open()
    expect(FakeSocket.opened[1].sent.map((text) => JSON.parse(text).op)).toEqual(['join', 'presence'])
    room.close()
  })

  it('waits longer each time it is dropped again before it opens', () => {
    const room = new RoomSocket('t1', 'board')
    FakeSocket.opened[0].drop()
    vi.advanceTimersByTime(REJOIN_MS)
    FakeSocket.opened[1].drop()
    vi.advanceTimersByTime(REJOIN_MS)
    expect(FakeSocket.opened).toHaveLength(2)
    vi.advanceTimersByTime(REJOIN_MS)
    expect(FakeSocket.opened).toHaveLength(3)
    room.close()
  })

  it('stays closed when its view closes it, or the room refuses it', () => {
    const closed = new RoomSocket('t1', 'board')
    closed.close()
    FakeSocket.opened[0].drop()
    const refused = new RoomSocket('t1', 'secret')
    FakeSocket.opened[1].open()
    FakeSocket.opened[1].hear({ event: 'refused', reason: 'forbidden: not yours' })
    FakeSocket.opened[1].drop()
    vi.advanceTimersByTime(REJOIN_MAX_MS)
    expect(FakeSocket.opened).toHaveLength(2)
    refused.close()
  })

  it('opens again when the server it was relayed to is out of reach', () => {
    const room = new RoomSocket('t1', 'board')
    FakeSocket.opened[0].open()
    FakeSocket.opened[0].hear({ event: 'refused', reason: "unavailable: the server can't be reached" })
    FakeSocket.opened[0].drop()
    vi.advanceTimersByTime(REJOIN_MS)
    expect(FakeSocket.opened).toHaveLength(2)
    room.close()
  })
})
