// The browser's event bus (ADR 0021): one WebSocket per tab to the daemon,
// subscribed to the topics the mounted views need. Events say what changed;
// the views' queries fetch it again.

import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef } from 'react'
import { token, type BusEvent } from './api'
import { keys } from './query'
import { apiUrl, hosted } from './served'

const RECONNECT_MS = 1000
const HOSTED_REFRESH_MS = 3000

type Listener = (event: BusEvent) => void

class Bus {
  private socket: WebSocket | null = null
  private topics = new Map<string, number>()
  private listeners = new Set<Listener>()
  private retry: ReturnType<typeof setTimeout> | null = null

  /** Hears every event, until the returned function is called. */
  listen(listener: Listener): () => void {
    this.listeners.add(listener)
    this.connect()
    return () => this.listeners.delete(listener)
  }

  /** Subscribes to `topic` until the returned function is called. */
  subscribe(topic: string): () => void {
    const count = this.topics.get(topic) ?? 0
    this.topics.set(topic, count + 1)
    if (count === 0) this.send({ subscribe: [topic] })
    this.connect()
    return () => {
      const left = (this.topics.get(topic) ?? 1) - 1
      if (left > 0) {
        this.topics.set(topic, left)
        return
      }
      this.topics.delete(topic)
      this.send({ unsubscribe: [topic] })
    }
  }

  private send(message: { subscribe?: string[]; unsubscribe?: string[] }) {
    if (this.socket?.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify(message))
  }

  private connect() {
    if (hosted()) {
      // The server has no event socket yet, so everything shown is fetched
      // again every few seconds instead.
      this.retry ??= setInterval(() => {
        for (const listener of this.listeners) listener({ event: 'lagged' })
      }, HOSTED_REFRESH_MS)
      return
    }
    if (this.socket || this.retry || typeof WebSocket === 'undefined') return
    const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws'
    const socket = new WebSocket(`${scheme}://${window.location.host}${apiUrl('/api/events')}?token=${token()}`)
    this.socket = socket
    let opened = false
    socket.onopen = () => {
      opened = true
      this.send({ subscribe: [...this.topics.keys()] })
    }
    socket.onmessage = (message: MessageEvent<string>) => {
      const event = JSON.parse(message.data) as BusEvent
      for (const listener of this.listeners) listener(event)
    }
    socket.onclose = () => {
      this.socket = null
      this.retry = setTimeout(() => {
        this.retry = null
        this.connect()
      }, RECONNECT_MS)
      // Whatever happened while it was closed went unheard.
      if (opened) for (const listener of this.listeners) listener({ event: 'lagged' })
    }
  }
}

export const bus = new Bus()

/** Subscribes to `topic` while the calling component is mounted. */
export function useTopic(topic: string | null) {
  useEffect(() => (topic ? bus.subscribe(topic) : undefined), [topic])
}

/** Calls `listener` for every event while the calling component is mounted. */
export function useBusEvents(listener: Listener) {
  const latest = useRef(listener)
  useEffect(() => {
    latest.current = listener
  })
  useEffect(() => bus.listen((event) => latest.current(event)), [])
}

/** Fetches again what an event says changed (ADR 0021). */
export function useBusInvalidation() {
  const queries = useQueryClient()
  useBusEvents((event) => {
    switch (event.event) {
      case 'threads':
        void queries.invalidateQueries({ queryKey: keys.threads })
        void queries.invalidateQueries({ queryKey: keys.projects })
        break
      case 'thread':
        void queries.invalidateQueries({ queryKey: keys.thread(event.thread) })
        break
      case 'queue':
        void queries.invalidateQueries({ queryKey: keys.queue(event.thread) })
        break
      case 'inbox':
        void queries.invalidateQueries({ queryKey: ['inbox'] })
        break
      case 'mrs':
        void queries.invalidateQueries({ queryKey: keys.mrs })
        void queries.invalidateQueries({ queryKey: keys.projects })
        break
      case 'lagged':
        void queries.invalidateQueries()
        break
      default:
        break
    }
  })
}
