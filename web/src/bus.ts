// The browser's event bus (ADR 0021): one WebSocket per tab to the daemon,
// subscribed to the topics the mounted views need. Events say what changed;
// the views' queries fetch it again.

import { type QueryClient, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
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
  /** The socket closed, or never opened, and hasn't opened since: not before the first try ends. */
  private lost = false
  private watchers = new Set<() => void>()

  /** Whether the page hears the daemon, or hasn't yet found it doesn't; hosted pages, which poll, count as connected. */
  connected = (): boolean => !this.lost || hosted()

  /** Hears the socket open and close, until the returned function is called. */
  watch = (watcher: () => void): (() => void) => {
    this.watchers.add(watcher)
    return () => this.watchers.delete(watcher)
  }

  private setOpen(open: boolean) {
    this.lost = !open
    for (const watcher of this.watchers) watcher()
  }

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
      this.setOpen(true)
      this.send({ subscribe: [...this.topics.keys()] })
    }
    socket.onmessage = (message: MessageEvent<string>) => {
      const event = JSON.parse(message.data) as BusEvent
      for (const listener of this.listeners) listener(event)
    }
    socket.onclose = () => {
      this.socket = null
      this.setOpen(false)
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

/** Whether the page hears the daemon's events now. */
export function useBusConnected(): boolean {
  return useSyncExternalStore(bus.watch, bus.connected)
}

/** Subscribes to `topic` while the calling component is mounted. */
export function useTopic(topic: string | null) {
  const queries = useQueryClient()
  useEffect(() => {
    if (!topic) return undefined
    const unsubscribe = bus.subscribe(topic)
    // A view subscribes once it has what it shows, so whatever changed in
    // between was announced to no one: fetch it again.
    for (const queryKey of topicKeys(topic)) void queries.invalidateQueries({ queryKey })
    return unsubscribe
  }, [topic, queries])
}

/** What an event on `topic` fetches again. */
function topicKeys(topic: string): readonly (readonly string[])[] {
  if (topic.startsWith('thread:')) return [keys.thread(topic.slice('thread:'.length))]
  if (topic === 'threads') return [keys.threads]
  if (topic === 'inbox') return [['inbox']]
  if (topic === 'mrs') return [keys.mrs]
  return []
}

/** Calls `listener` for every event while the calling component is mounted. */
export function useBusEvents(listener: Listener) {
  const latest = useRef(listener)
  useEffect(() => {
    latest.current = listener
  })
  useEffect(() => bus.listen((event) => latest.current(event)), [])
}

/**
 * Fetches again what an event says changed. A fetch under way finishes rather than starting over: a burst of
 * events, such as an import's, would otherwise restart a slow one until requests pile up past what the browser
 * allows. What changed after it began is fetched once more when it ends, else a turn's last words could miss it.
 */
export class Refetcher {
  private readonly again = new Set<string>()
  private readonly queries: QueryClient

  constructor(queries: QueryClient) {
    this.queries = queries
  }

  /** Watches for fetches ending; answers how to stop. */
  start(): () => void {
    return this.queries.getQueryCache().subscribe(({ query }) => {
      if (query.state.fetchStatus === 'fetching' || !this.again.delete(query.queryHash)) return
      void this.queries.invalidateQueries({ queryKey: query.queryKey, exact: true }, { cancelRefetch: false })
    })
  }

  refetch(queryKey?: readonly unknown[]) {
    for (const query of this.queries.getQueryCache().findAll({ queryKey })) {
      if (query.state.fetchStatus === 'fetching') this.again.add(query.queryHash)
    }
    void this.queries.invalidateQueries({ queryKey }, { cancelRefetch: false })
  }
}

/** How long a burst of `lagged` events counts as one. */
const LAGGED_MS = 1000

/** Fetches again what an event says changed (ADR 0021). */
export function useBusInvalidation() {
  const queries = useQueryClient()
  const laggedAt = useRef(0)
  const [refetcher] = useState(() => new Refetcher(queries))
  useEffect(() => refetcher.start(), [refetcher])
  const refetch = (queryKey?: readonly unknown[]) => refetcher.refetch(queryKey)
  useBusEvents((event) => {
    switch (event.event) {
      case 'threads':
        refetch(keys.threads)
        refetch(keys.projects)
        refetch(['graph'])
        break
      case 'thread':
        refetch(keys.thread(event.thread))
        refetch(['graph'])
        break
      case 'queue':
        refetch(keys.queue(event.thread))
        break
      case 'inbox':
        refetch(['inbox'])
        break
      case 'mrs':
        refetch(keys.mrs)
        refetch(keys.projects)
        refetch(['graph'])
        break
      case 'agents':
        refetch(keys.agents)
        break
      case 'approvals':
        refetch(['approvals'])
        break
      case 'appearance':
        refetch(keys.themes)
        break
      case 'lagged':
        if (Date.now() - laggedAt.current < LAGGED_MS) break
        laggedAt.current = Date.now()
        refetch()
        break
      default:
        break
    }
  })
}
