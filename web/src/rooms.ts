// A room (ADR 0081): one socket to the daemon or the server per room joined, carrying moments and presence between the
// people viewing a page now. Nothing in it persists.

import { apiUrl, hosted } from './served'

export type PeerInfo = { peer: string; user: string; kind: string }
export type PeerPresence = PeerInfo & { state: unknown }
export type Heard =
  | { event: 'moment'; topic: string; data: unknown; from: PeerInfo }
  | { event: 'peers'; peers: PeerPresence[] }
  | { event: 'refused'; reason: string }
  | { event: 'joined'; peer: string }

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
      for (const listener of this.listeners) listener(heard)
    }
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
    this.listeners.clear()
    this.socket.close()
  }
}
