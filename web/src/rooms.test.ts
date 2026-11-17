import { describe, expect, it } from 'vitest'
import { PeerBook, PRESENCE_MS } from './rooms'

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
