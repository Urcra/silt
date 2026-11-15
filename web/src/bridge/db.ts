// `db`, an artifact's page data (ADR 0072): reads and writes go to the daemon or the server, which fold the thread's
// log and check the version's rules; a watch reads again whenever the thread changes, and tells the page what changed.

import { api, type ArtifactView } from '../api'
import { bus } from '../bus'
import { type CapabilityHandler, Refusal } from './broker'

const READS = ['get', 'list', 'query']
const WRITES = ['set', 'update', 'delete', 'batch']

/** The most watches one page keeps. */
const WATCHES = 32

type Watch = { request: Record<string, unknown>; last: string }

export function dbHandler(thread: string, artifact: ArtifactView): CapabilityHandler {
  const watches = new Map<number, Watch>()
  let next = 0
  let unsubscribe: (() => void) | null = null
  let unlisten: (() => void) | null = null
  let emit: ((topic: string, data: unknown) => void) | null = null
  let pending: number | null = null

  const ask = (request: Record<string, unknown>) => api.pageData(thread, artifact.id, request)
  const refresh = () => {
    pending = null
    for (const [id, watch] of watches) {
      void ask(watch.request).then(
        (data) => {
          const text = JSON.stringify(data)
          if (text === watch.last || !watches.has(id)) return
          watch.last = text
          emit?.('snapshot', { watch: id, data })
        },
        () => undefined,
      )
    }
  }
  const listen = () => {
    if (unsubscribe) return
    unsubscribe = bus.subscribe(`thread:${thread}`)
    unlisten = bus.listen((event) => {
      const said = event as { event?: string; thread?: string }
      const ours = said.event === 'lagged' || (said.event === 'thread' && typeof said.thread === 'string' && said.thread.endsWith(thread))
      if (ours && pending === null) pending = window.setTimeout(refresh, 150)
    })
  }

  return {
    methods: [...READS, ...WRITES, 'watch', 'unwatch'],
    perMinute: 600,
    call: async (method, args, context) => {
      const given = (args ?? {}) as Record<string, unknown>
      if (READS.includes(method) || WRITES.includes(method)) return ask({ ...given, op: method })
      if (method === 'watch') {
        if (watches.size >= WATCHES) throw new Refusal('rate_limited', `a page keeps at most ${WATCHES} watches`)
        const request = (given.request ?? {}) as Record<string, unknown>
        if (!READS.includes(String(request.op))) throw new Refusal('refused', 'a watch reads: get, list or query')
        const data = await ask(request)
        const id = ++next
        watches.set(id, { request, last: JSON.stringify(data) })
        emit = (topic, payload) => context.emit('db', topic, payload)
        listen()
        return { watch: id, data }
      }
      if (method === 'unwatch') {
        watches.delete(Number(given.watch))
        return null
      }
      throw new Refusal('refused', `db has no ${method}`)
    },
    dispose: () => {
      watches.clear()
      if (pending !== null) window.clearTimeout(pending)
      unsubscribe?.()
      unlisten?.()
    },
  }
}
