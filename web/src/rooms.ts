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
    return [...this.peers.values()].some(({ peer, at }) => peer.state !== null && now - at > PRESENCE_MS && then - at <= PRESENCE_MS)
  }
}

const TOKEN_KEY = 'silt-token'

/** The address a room's socket opens at, here or on the server hosting the app. */
function roomUrl(): string {
  const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws'
  const path = hosted() ? apiUrl('/app/api/rooms') : `${apiUrl('/api/rooms')}?token=${sessionStorage.getItem(TOKEN_KEY) ?? ''}`
  return `${scheme}://${window.location.host}${path}`
}

export class RoomSocket {
  private socket: WebSocket
  private queue: string[] = []
  private listeners = new Set<(heard: Heard) => void>()
  private book = new PeerBook()
  private lapse: ReturnType<typeof setInterval>
  /** This view's own peer id, once joined. */
  me: string | null = null

  constructor(thread: string, artifact: string, name = '') {
    this.socket = new WebSocket(roomUrl())
    this.socket.onopen = () => {
      this.socket.send(JSON.stringify({ op: 'join', thread, artifact, name }))
      for (const message of this.queue.splice(0)) this.socket.send(message)
    }
    this.socket.onmessage = (message: MessageEvent<string>) => {
      const heard = JSON.parse(message.data) as Heard
      if (heard.event === 'joined') this.me = heard.peer
      // Listeners hear everyone whenever anyone changes, as the room sends only what changed.
      if (this.book.heard(heard, Date.now())) this.tell({ event: 'peers', peers: this.book.list(Date.now()) })
      else if (heard.event !== 'presence') this.tell(heard)
    }
    let checked = Date.now()
    this.lapse = setInterval(() => {
      const now = Date.now()
      if (this.book.lapsed(checked, now)) this.tell({ event: 'peers', peers: this.book.list(now) })
      checked = now
    }, 5000)
  }

  private tell(heard: Heard) {
    for (const listener of this.listeners) listener(heard)
  }

  private send(message: unknown) {
    const text = JSON.stringify(message)
    if (this.socket.readyState === WebSocket.OPEN) this.socket.send(text)
    else if (this.socket.readyState === WebSocket.CONNECTING) this.queue.push(text)
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
    clearInterval(this.lapse)
    this.listeners.clear()
    this.socket.close()
  }
}
