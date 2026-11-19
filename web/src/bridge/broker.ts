// The holding page's side of the bridge (ADR 0072): each call a frame makes is checked here, against the version's
// declared capabilities, its size and the capability's rate, before anything acts on the viewer's behalf.

import type { ArtifactView } from '../api'

/** Why a call was refused, as a page reads it from the rejection's `code`. */
export type RefusalCode =
  | 'not_granted'
  | 'rate_limited'
  | 'conflict'
  | 'too_large'
  | 'refused'
  | 'unavailable'
  | 'busy'
  | 'viewer_stopped'

export class Refusal extends Error {
  code: RefusalCode
  constructor(code: RefusalCode, message: string) {
    super(message)
    this.code = code
  }
}

/** What a capability's handler knows of the call it answers. */
export type CallContext = { thread: string; artifact: ArtifactView; emit: (cap: string, topic: string, data: unknown) => void }

/** A capability as the holding page serves it: its functions' names, and what answers them. */
export type CapabilityHandler = {
  methods: string[]
  call: (method: string, args: unknown, context: CallContext) => Promise<unknown>
  /** Calls allowed per minute; 120 when unset. */
  perMinute?: number
  /** The largest call it takes, when more than [`MESSAGE_BYTES`], as a download's file may be. */
  callBytes?: number
  /** The largest answer it gives, when more than [`MESSAGE_BYTES`], as a file read or a query may be. */
  answerBytes?: number
  /** Ends what the handler started, such as watches, when the frame goes. */
  dispose?: () => void
}

/** The largest message a frame may send, or be sent. */
export const MESSAGE_BYTES = 1 << 20

export type Call = { id: number; cap: string; method: string; args: unknown }

/** A frame's message, if it's a call. */
export function callOf(data: unknown): Call | null {
  const said = data as { silt?: unknown; id?: unknown; cap?: unknown; method?: unknown; args?: unknown } | null
  if (said?.silt !== 'call' || typeof said.id !== 'number' || typeof said.cap !== 'string' || typeof said.method !== 'string')
    return null
  return { id: said.id, cap: said.cap, method: said.method, args: said.args ?? null }
}

/** The size of `value`: bytes and Blobs by their length, the rest as JSON; Infinity when it doesn't serialize. */
export function sizeOf(value: unknown): number {
  if (value instanceof Blob) return value.size
  if (value instanceof ArrayBuffer || ArrayBuffer.isView(value)) return value.byteLength
  if (Array.isArray(value)) return value.reduce((sum: number, item) => sum + sizeOf(item) + 1, 2)
  if (value && typeof value === 'object') {
    let sum = 2
    for (const [key, item] of Object.entries(value)) sum += key.length + 4 + sizeOf(item)
    return sum
  }
  try {
    return JSON.stringify(value ?? null).length
  } catch {
    return Infinity
  }
}

export class Broker {
  private calls = new Map<string, number[]>()
  private readonly context: CallContext
  private readonly handlers: Record<string, CapabilityHandler>
  private readonly now: () => number

  constructor(context: CallContext, handlers: Record<string, CapabilityHandler>, now: () => number = Date.now) {
    this.context = context
    this.handlers = handlers
    this.now = now
  }

  /** Ends every handler's work, as the frame goes. */
  dispose() {
    for (const handler of Object.values(this.handlers)) handler.dispose?.()
  }

  /** Whether the version declared `name` and this view serves it. */
  available(name: string): boolean {
    // `permissions` is built in: every page may ask what it was allowed.
    if (name === 'permissions') return name in this.handlers
    return Boolean(this.context.artifact.capabilities?.[name]) && name in this.handlers
  }

  /** The answer to one call, or the refusal it gets. */
  async answer(call: Call): Promise<{ ok: true; value: unknown } | { ok: false; error: { code: RefusalCode; message: string } }> {
    try {
      return { ok: true, value: await this.run(call) }
    } catch (failure) {
      const refusal =
        failure instanceof Refusal
          ? failure
          : new Refusal('refused', failure instanceof Error ? failure.message : String(failure))
      return { ok: false, error: { code: refusal.code, message: refusal.message } }
    }
  }

  private async run(call: Call): Promise<unknown> {
    const most = Math.max(MESSAGE_BYTES, this.handlers[call.cap]?.callBytes ?? 0)
    if (sizeOf(call.args) > most) throw new Refusal('too_large', `a call is at most ${most >> 20} MiB`)
    if (call.cap === 'silt' && call.method === 'use') {
      const name = String((call.args as { name?: unknown } | null)?.name ?? '')
      return this.available(name) ? { available: true, methods: this.handlers[name].methods } : { available: false }
    }
    if (!this.available(call.cap))
      throw new Refusal('not_granted', `this page didn't declare ${call.cap}, or it isn't served here`)
    const handler = this.handlers[call.cap]
    if (!handler.methods.includes(call.method)) throw new Refusal('refused', `${call.cap} has no ${call.method}`)
    this.spend(call.cap, handler.perMinute ?? 120)
    const value = await handler.call(call.method, call.args, this.context)
    if (sizeOf(value) > Math.max(MESSAGE_BYTES, handler.answerBytes ?? 0))
      throw new Refusal('too_large', 'the answer is too large to send to the page')
    return value
  }

  private spend(cap: string, perMinute: number) {
    const now = this.now()
    const recent = (this.calls.get(cap) ?? []).filter((at) => now - at < 60_000)
    if (recent.length >= perMinute) throw new Refusal('rate_limited', `${cap} takes at most ${perMinute} calls a minute`)
    recent.push(now)
    this.calls.set(cap, recent)
  }
}
