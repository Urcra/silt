// A room (ADR 0081): one socket to the daemon or the server per room joined, carrying moments and presence between the
// people viewing a page now. Nothing in it persists.

import { apiUrl, hosted } from './served'

export type PeerInfo = { peer: string; user: string; kind: string }
export type PeerPresence = PeerInfo & { state: unknown }
export type Heard =
  | { event: 'moment'; topic: string; data: unknown; from: PeerInfo }
  | { event: 'peers'; peers: PeerPresence[] }
  | { event: 'presence'; peer: PeerPresence }
  | { event: 'refused'; reason: string }
  | { event: 'joined'; peer: string }

/** How long a peer's presence holds without word from them, as the room's own. */
export const PRESENCE_MS = 15_000

/**
 * Everyone in a room as one view knows them: the list when someone comes or goes, then each peer's presence as it
 * changes, which lapses when they've said nothing for [`PRESENCE_MS`].
 */
export class PeerBook {
  private peers = new Map<string, { peer: PeerPresence; at: number }>()

  /** Takes in what was heard, and answers whether the list changed. */
  heard(heard: Heard, now: number): boolean {
    if (heard.event === 'peers') {
      this.peers = new Map(heard.peers.map((peer) => [peer.peer, { peer, at: now }]))
      return true
    }
    if (heard.event === 'presence') {
      if (!this.peers.has(heard.peer.peer)) return false
      this.peers.set(heard.peer.peer, { peer: heard.peer, at: now })
      return true
    }
    if (heard.event === 'moment') {
      const known = this.peers.get(heard.from.peer)
      if (known) known.at = now
    }
    return false
  }

  /** Everyone, presence that lapsed by `now` left out. */
  list(now: number): PeerPresence[] {
    return [...this.peers.values()].map(({ peer, at }) => (now - at > PRESENCE_MS ? { ...peer, state: null } : peer))
  }

  /** Whether a presence still shown at `then` has lapsed by `now`. */
  lapsed(then: number, now: number): boolean {
    return [...this.peers.values()].some(
      ({ peer, at }) => peer.state !== null && now - at > PRESENCE_MS && then - at <= PRESENCE_MS,
    )
  }
}

const TOKEN_KEY = 'silt-token'

/** The address a room's socket opens at, here or on the server hosting the app. */
function roomUrl(): string {
  const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws'
  const path = hosted() ? apiUrl('/app/api/rooms') : `${apiUrl('/api/rooms')}?token=${sessionStorage.getItem(TOKEN_KEY) ?? ''}`
  return `${scheme}://${window.location.host}${path}`
}

/** How long a dropped socket waits before it opens again, doubling up to [`REJOIN_MAX_MS`]. */
export const REJOIN_MS = 500
export const REJOIN_MAX_MS = 10_000
/** What is kept to send while the socket opens again: presence and moments are worth little once stale. */
const QUEUED = 256

export class RoomSocket {
  private socket!: WebSocket
  private queue: string[] = []
  private listeners = new Set<(heard: Heard) => void>()
  private book = new PeerBook()
  private lapse: ReturnType<typeof setInterval>
  private rejoin: ReturnType<typeof setTimeout> | undefined
  private wait = REJOIN_MS
  /** Closed by this view, or refused for good: no socket opens again. */
  private done = false
  private join: string
  /** This view's own peer id, once joined. */
  me: string | null = null

  constructor(thread: string, artifact: string, name = '') {
    this.join = JSON.stringify({ op: 'join', thread, artifact, name })
    this.open()
    let checked = Date.now()
    this.lapse = setInterval(() => {
      const now = Date.now()
      if (this.book.lapsed(checked, now)) this.tell({ event: 'peers', peers: this.book.list(now) })
      checked = now
    }, 5000)
  }

  /**
   * Opens the socket and joins. One that drops, as when the daemon loses the server it relays the room to, opens
   * again: otherwise this view would neither hear the others nor be heard until the page is opened again.
   */
  private open() {
    const socket = new WebSocket(roomUrl())
    this.socket = socket
    socket.onopen = () => {
      this.wait = REJOIN_MS
      socket.send(this.join)
      for (const message of this.queue.splice(0)) socket.send(message)
    }
    socket.onmessage = (message: MessageEvent<string>) => {
      const heard = JSON.parse(message.data) as Heard
      if (heard.event === 'joined') this.me = heard.peer
      // Refused for a reason other than the server being out of reach, joining again would be refused again.
      if (heard.event === 'refused' && !heard.reason.startsWith('unavailable')) this.done = true
      // Listeners hear everyone whenever anyone changes, as the room sends only what changed.
      if (this.book.heard(heard, Date.now())) this.tell({ event: 'peers', peers: this.book.list(Date.now()) })
      else if (heard.event !== 'presence') this.tell(heard)
    }
    socket.onclose = () => {
      if (this.done || this.socket !== socket) return
      this.me = null
      this.rejoin = setTimeout(() => this.open(), this.wait)
      this.wait = Math.min(this.wait * 2, REJOIN_MAX_MS)
    }
  }

  private tell(heard: Heard) {
    for (const listener of this.listeners) listener(heard)
  }

  private send(message: unknown) {
    const text = JSON.stringify(message)
    if (this.socket.readyState === WebSocket.OPEN) this.socket.send(text)
    else if (!this.done) this.queue = [...this.queue, text].slice(-QUEUED)
  }

  emit(topic: string, data: unknown) {
    this.send({ op: 'emit', topic, data })
  }

  presence(state: unknown) {
    this.send({ op: 'presence', state })
  }

  listen(fn: (heard: Heard) => void): () => void {
    this.listeners.add(fn)
    return () => this.listeners.delete(fn)
  }

  close() {
    this.done = true
    clearInterval(this.lapse)
    clearTimeout(this.rejoin)
    this.listeners.clear()
    this.socket.close()
  }
}
