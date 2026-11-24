// The browser's event bus (ADR 0021): one WebSocket per tab to the daemon,
// subscribed to the topics the mounted views need. Events say what changed;
// the views' queries fetch it again.

import { type QueryClient, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { abortReads, token, type BusEvent } from './api'
import { keys } from './query'
import { apiUrl, hosted } from './served'

const RECONNECT_MS = 1000
const HOSTED_REFRESH_MS = 3000
/** How long an open socket may say nothing before it counts as dead; the daemon beats every 10 s. */
export const QUIET_MS = 25_000
/** How often the socket's quiet is checked. */
const CHECK_MS = 5000
/** How long the page may be hidden before what it was reading is read again as it shows. */
export const ASLEEP_MS = 5000

type Listener = (event: BusEvent) => void

/**
 * One socket per tab to the daemon. A phone that sleeps or changes networks can leave it open to a peer long gone,
 * hearing nothing and never closing, so it is given up once quiet for longer than the daemon's beat allows.
 */
export class Bus {
  private socket: WebSocket | null = null
  private topics = new Map<string, number>()
  private listeners = new Set<Listener>()
  private retry: ReturnType<typeof setTimeout> | null = null
  /** The socket closed, or never opened, and hasn't opened since: not before the first try ends. */
  private lost = false
  private watchers = new Set<() => void>()
  private resumers = new Set<() => void>()
  /** When the open socket last said anything. */
  private heardAt = 0
  /** Whether any socket opened yet, so the next to open follows a lost one. */
  private openedOnce = false
  private hiddenAt: number | null = null
  private checking: ReturnType<typeof setInterval> | null = null

  /** Whether the page hears the daemon, or hasn't yet found it doesn't; hosted pages, which poll, count as connected. */
  connected = (): boolean => !this.lost || hosted()

  /** Hears the socket open and close, until the returned function is called. */
  watch = (watcher: () => void): (() => void) => {
    this.watchers.add(watcher)
    return () => this.watchers.delete(watcher)
  }

  /**
   * Hears the page come back, woken or with its socket open again after losing it, until the returned function is
   * called: what it was reading then may wait on a dead connection, and what changed meanwhile went unheard.
   * Listeners then hear `lagged`.
   */
  onResume(resumer: () => void): () => void {
    this.resumers.add(resumer)
    return () => this.resumers.delete(resumer)
  }

  private setOpen(open: boolean) {
    this.lost = !open
    for (const watcher of this.watchers) watcher()
  }

  private resume() {
    for (const resumer of this.resumers) resumer()
    for (const listener of this.listeners) listener({ event: 'lagged' })
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

  /** Gives up an open socket that has said nothing for too long; answers whether it did. */
  check(): boolean {
    const socket = this.socket
    if (!socket || socket.readyState !== WebSocket.OPEN || Date.now() - this.heardAt <= QUIET_MS) return false
    socket.onopen = socket.onmessage = socket.onclose = null
    socket.close()
    this.closed()
    return true
  }

  /** The page was hidden or shown. */
  shown(visible: boolean) {
    if (!visible) {
      this.hiddenAt ??= Date.now()
      return
    }
    const hiddenAt = this.hiddenAt
    this.hiddenAt = null
    // A socket given up opens again, and resumes then.
    if (this.check()) return
    if (hiddenAt !== null && Date.now() - hiddenAt > ASLEEP_MS) this.resume()
  }

  /** The network came back: connections opened before may be dead. */
  online() {
    if (!this.check()) this.resume()
  }

  private watchPage() {
    if (this.checking || typeof document === 'undefined') return
    this.checking = setInterval(() => this.check(), CHECK_MS)
    document.addEventListener('visibilitychange', () => this.shown(document.visibilityState === 'visible'))
    window.addEventListener('online', () => this.online())
  }

  private closed() {
    this.socket = null
    this.setOpen(false)
    this.retry = setTimeout(() => {
      this.retry = null
      this.connect()
    }, RECONNECT_MS)
  }

  private connect() {
    this.watchPage()
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
    socket.onopen = () => {
      this.heardAt = Date.now()
      this.setOpen(true)
      this.send({ subscribe: [...this.topics.keys()] })
      // Whatever happened while no socket was open went unheard.
      if (this.openedOnce) this.resume()
      this.openedOnce = true
    }
    socket.onmessage = (message: MessageEvent<string>) => {
      this.heardAt = Date.now()
      const event = JSON.parse(message.data) as BusEvent
      if (event.event === 'beat') return
      for (const listener of this.listeners) listener(event)
    }
    socket.onclose = () => this.closed()
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

/**
 * Gives up the reads under way and fetches everything again: one waiting on a dead connection would otherwise keep
 * what it reads from ever refreshing, as [`Refetcher`] lets a fetch under way finish.
 */
export async function readAgain(queries: QueryClient) {
  const cancelled = queries.cancelQueries()
  abortReads()
  await cancelled
  await queries.invalidateQueries()
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
  useEffect(
    () =>
      bus.onResume(() => {
        // Its `lagged` follows at once; everything is fetched again here instead, once the reads under way are given up.
        laggedAt.current = Date.now()
        void readAgain(queries)
      }),
    [queries],
  )
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
      case 'asks':
        refetch(keys.asks)
        break
      case 'approvals':
        refetch(['approvals'])
        break
      case 'appearance':
        refetch(keys.themes)
        break
      case 'lan':
        refetch(keys.lan)
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
