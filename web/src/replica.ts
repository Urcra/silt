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
  private syncing: Promise<void> = Promise.resolve()
  private broken = false

  private constructor(thread: string, replica: Replica) {
    this.thread = thread
    this.replica = replica
  }

  /** A trap leaves the replica unusable, so later calls say so plainly. */
  private use<T>(call: (replica: Replica) => T): T {
    if (this.broken) throw new Error('The editor stopped after an internal error; reload the page.')
    try {
      return call(this.replica)
    } catch (error) {
      if (error instanceof WebAssembly.RuntimeError) {
        this.broken = true
        throw new Error('The editor stopped after an internal error (see the console); reload the page.')
      }
      throw error
    }
  }

  static async open(thread: string): Promise<LiveReplica> {
    loaded ??= init()
    await loaded
    const live = new LiveReplica(thread, new Replica(thread, randomActor()))
    await live.sync()
    return live
  }

  /** Takes in whatever the daemon has that this replica hasn't, one sync at a time. */
  sync(): Promise<void> {
    const next = this.syncing.then(() => this.syncOnce())
    this.syncing = next.catch(() => undefined)
    return next
  }

  /** Takes the daemon's deltas a page at a time, until it has them all. */
  private async syncOnce(): Promise<void> {
    for (;;) {
      const after = this.use((replica) => replica.received())
      const reply = await api.replica(this.thread, after)
      this.writable = reply.writable
      await this.fill(this.use((replica) => replica.receive(after, fromBase64(reply.deltas))))
      const now = this.use((replica) => replica.received())
      if (now >= reply.total || now === after) return
    }
  }

  private async fill(needed: string | undefined): Promise<void> {
    while (needed) {
      const text = await api.gitText(this.thread, needed)
      const base = needed
      needed = this.use((replica) => replica.add_base(base, text))
    }
  }

  paths(): string[] {
    return this.use((replica) => replica.paths())
  }

  /** Fetches the git text the file still needs, so `showNow` can read it. */
  async prepare(path: string): Promise<void> {
    await this.fill(this.use((replica) => replica.needed_base(path)))
  }

  async text(path: string): Promise<string | undefined> {
    await this.prepare(path)
    return this.use((replica) => replica.text(path))
  }

  /** The file's text, to show in the editor: later edits are made against it. */
  async show(path: string): Promise<string | undefined> {
    await this.prepare(path)
    return this.showNow(path)
  }

  /** `show` for a prepared file, at once. */
  showNow(path: string): string | undefined {
    return this.use((replica) => replica.show(path))
  }

  /** A doc artifact's text as the replica has it, to show in the editor: later edits are made against it. */
  pageShow(artifact: string): string {
    return this.use((replica) => replica.page_show(artifact))
  }

  /** The editor's text of the doc is now `text`: made at once, against what it was shown, then sent to the daemon. */
  pageEdit(artifact: string, text: string): Promise<void> {
    const made = this.use((replica) => replica.page_edit(artifact, text, Date.now()))
    return made.length > 1 ? api.sendDeltas(this.thread, toBase64(made)).then(() => undefined) : Promise.resolve()
  }

  /**
   * The editor's text of the file is now `text`. The edits are made at once,
   * against what the editor was shown; sending them to the daemon follows.
   */
  edit(path: string, text: string): Promise<void> {
    const made = this.use((replica) => replica.edit(path, text, Date.now()))
    return made.length > 1 ? api.sendDeltas(this.thread, toBase64(made)).then(() => undefined) : Promise.resolve()
  }
}
