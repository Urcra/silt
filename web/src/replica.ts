// The browser's replica of a thread family (ADR 0014): the CRDT compiled to
// WebAssembly, fed from the daemon, editing files as deltas of its own.
import init, { Replica } from './wasm/silt_wasm.js'
import { api } from './api'

let loaded: Promise<unknown> | undefined

function randomActor(): string {
  const bytes = new Uint8Array(16)
  crypto.getRandomValues(bytes)
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('')
}

function fromBase64(text: string): Uint8Array {
  return Uint8Array.from(atob(text), (char) => char.charCodeAt(0))
}

function toBase64(bytes: Uint8Array): string {
  let text = ''
  for (const byte of bytes) text += String.fromCharCode(byte)
  return btoa(text)
}

export class LiveReplica {
  writable = false
  private readonly thread: string
  private readonly replica: Replica

  private constructor(thread: string, replica: Replica) {
    this.thread = thread
    this.replica = replica
  }

  static async open(thread: string): Promise<LiveReplica> {
    loaded ??= init()
    await loaded
    const live = new LiveReplica(thread, new Replica(thread, randomActor()))
    await live.sync()
    return live
  }

  /** Takes in whatever the daemon has that this replica hasn't. */
  async sync(): Promise<void> {
    const reply = await api.replica(this.thread, this.replica.received())
    this.writable = reply.writable
    await this.fill(this.replica.receive(fromBase64(reply.deltas)))
  }

  private async fill(needed: string | undefined): Promise<void> {
    while (needed) needed = this.replica.add_base(needed, await api.gitText(this.thread, needed))
  }

  paths(): string[] {
    return this.replica.paths().split('\n').filter(Boolean)
  }

  async text(path: string): Promise<string | undefined> {
    await this.fill(this.replica.needed_base(path))
    return this.replica.text(path)
  }

  /** Makes the file hold `text` and sends the edits to the daemon. */
  async edit(path: string, text: string): Promise<void> {
    const made = this.replica.edit(path, text, Date.now())
    if (made.length > 1) await api.sendDeltas(this.thread, toBase64(made))
  }
}
