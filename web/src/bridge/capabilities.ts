// The capabilities this view serves an artifact's page, by name (ADR 0072). A page that asks for one not here gets
// null.

import { api, type ArtifactView, type PageViewer } from '../api'
import { bus } from '../bus'
import { hosted } from '../served'
import { type CapabilityHandler, Refusal } from './broker'
import { RoomSocket } from '../rooms'
import { dbHandler } from './db'
import { dictationHandler, dictationServed, mcpHandler, ttsHandler, ttsServed } from './voiceAndTools'

/** What a capability asks of the page holding the frame: the person's say before something acts for them. */
export type Holder = {
  /** Asks the person in the holding page itself, never in a browser dialog, and answers whether they agreed. */
  confirm: (ask: { title: string; detail: string; action: string }) => Promise<boolean>
  /** Opens the artifact's comment composer, quoting `text`. */
  openComposer: (text: string) => void
  /** Comments as the viewer on the version shown, quoting `quote`, and answers the comment's id. */
  addComment: (text: string, quote: string | null) => Promise<string>
  /** The ids of the comments on this artifact, which a page may reply to and resolve. */
  commentIds: () => string[]
  /** Shows what a page's reply or resolve changed. */
  commentsChanged: () => void
}

export function handlersFor(thread: string, artifact: ArtifactView, holder: Holder): Record<string, CapabilityHandler> {
  return {
    db: dbHandler(thread, artifact),
    artifact: artifactHandler(thread, artifact),
    user: userHandler(thread),
    downloads: downloadsHandler(holder),
    comments: commentsHandler(thread, artifact, holder),
    sample: sampleHandler(thread, artifact, holder),
    assets: assetsHandler(thread, artifact),
    files: filesHandler(thread, artifact, holder),
    ...(dictationServed() ? { dictation: dictationHandler(thread, artifact, holder) } : {}),
    ...(ttsServed() ? { tts: ttsHandler() } : {}),
    mcp: mcpHandler(thread, artifact, holder),
    room: roomHandler(thread, artifact),
    permissions: permissionsHandler(thread, artifact, holder),
  }
}

/** The capabilities that ask the viewer before they act, which `permissions` reports on. */
const ASKED = ['sample', 'dictation', 'mcp', 'comments']

/** `permissions`, built in: what the viewer allowed this page, asked for at once, or forgotten so it asks again. */
function permissionsHandler(thread: string, artifact: ArtifactView, holder: Holder): CapabilityHandler {
  const declared = () => ASKED.filter((name) => Boolean(artifact.capabilities?.[name]) && !(name === 'comments' && composerOnly(artifact)))
  const states = async () => {
    const grants = await api.artifactGrants(thread, artifact.id).catch(() => ({}) as Record<string, string>)
    return Object.fromEntries(declared().map((name) => [name, grants[name] === 'granted' ? 'granted' : grants[name] === 'refused' ? 'denied' : 'prompt']))
  }
  // A refusal holds for the page's load: asking again shows nothing until it opens anew.
  const asked = new Set<string>()
  return {
    methods: ['state', 'request', 'manage'],
    perMinute: 20,
    call: async (method, args) => {
      const given = (args ?? {}) as { name?: unknown; names?: unknown }
      if (method === 'state') {
        const all = await states()
        return typeof given.name === 'string' ? (all[given.name] ?? 'unavailable') : all
      }
      if (method === 'request') {
        const wanted = Array.isArray(given.names) ? given.names.map(String) : declared()
        const all = await states()
        const pending = wanted.filter((name) => all[name] === 'prompt' && !asked.has(name))
        if (pending.length > 0) {
          pending.forEach((name) => asked.add(name))
          const agreed = await holder.confirm({ title: `Let ${artifact.title} use ${pending.join(', ')}?`, detail: 'You can change this later from the page.', action: 'Allow' })
          for (const name of pending) await api.setArtifactGrant(thread, artifact.id, name, agreed ? 'granted' : 'refused').catch(() => undefined)
        }
        const now = await states()
        return Object.fromEntries(wanted.map((name) => [name, now[name] ?? 'unavailable']))
      }
      const forget = await holder.confirm({ title: `Forget what you allowed ${artifact.title}?`, detail: 'It asks again the next time it needs something.', action: 'Forget' })
      if (forget) for (const name of declared()) await api.setArtifactGrant(thread, artifact.id, name, null).catch(() => undefined)
      return null
    },
  }
}

/** `room`: moments between the people viewing the page now, one socket per room joined (ADR 0081). */
function roomHandler(thread: string, artifact: ArtifactView): CapabilityHandler {
  const sockets = new Map<string, RoomSocket>()
  const open = (name: string, context: Parameters<CapabilityHandler['call']>[2]) => {
    let socket = sockets.get(name)
    if (!socket) {
      socket = new RoomSocket(thread, artifact.id, name)
      socket.listen((heard) => {
        if (heard.event === 'moment') context.emit('room', 'moment', { room: name, topic: heard.topic, data: heard.data, from: heard.from })
        else if (heard.event === 'peers')
          context.emit('room', 'peers', { room: name, peers: heard.peers.map((peer) => ({ ...peer, you: peer.peer === socket!.me })) })
        else if (heard.event === 'refused') context.emit('room', 'refused', { room: name, reason: heard.reason })
      })
      sockets.set(name, socket)
    }
    return socket
  }
  return {
    methods: ['emit', 'presence', 'join', 'leave'],
    perMinute: 1800,
    dispose: () => {
      for (const socket of sockets.values()) socket.close()
      sockets.clear()
    },
    call: async (method, args, context) => {
      const given = (args ?? {}) as { room?: unknown; topic?: unknown; data?: unknown; state?: unknown }
      const name = typeof given.room === 'string' ? given.room.slice(0, 64) : ''
      if (method === 'leave') {
        sockets.get(name)?.close()
        sockets.delete(name)
        return null
      }
      const socket = open(name, context)
      if (method === 'emit') socket.emit(String(given.topic ?? ''), given.data ?? null)
      if (method === 'presence') socket.presence(given.state ?? null)
      return null
    },
  }
}

/** The most an added file may be. */
export const ASSET_BYTES = 20 << 20

async function base64Of(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer())
  let text = ''
  for (let at = 0; at < bytes.length; at += 0x8000) text += String.fromCharCode(...bytes.subarray(at, at + 0x8000))
  return btoa(text)
}

/** `assets`: files people add to the page, kept with the thread and served beside the page at `_assets/<id>`. */
function assetsHandler(thread: string, artifact: ArtifactView): CapabilityHandler {
  const ask = (asked: Parameters<typeof api.pageAssets>[2]) =>
    api.pageAssets(thread, artifact.id, asked).catch((failure: unknown) => {
      throw refusalOf(failure)
    })
  return {
    methods: ['upload', 'list', 'delete'],
    perMinute: 30,
    callBytes: ASSET_BYTES + 4096,
    call: async (method, args) => {
      const given = (args ?? {}) as { blob?: unknown; name?: unknown; id?: unknown }
      if (method === 'list') {
        const assets = (await ask({ op: 'list' })) as { size: number }[]
        return { assets, usage: { bytes: assets.reduce((sum, asset) => sum + asset.size, 0) } }
      }
      if (method === 'delete') return ask({ op: 'delete', id: String(given.id ?? '') })
      if (!(given.blob instanceof Blob)) throw new Refusal('refused', 'upload takes a Blob or a File')
      if (given.blob.size > ASSET_BYTES) throw new Refusal('too_large', `a file is at most ${ASSET_BYTES >> 20} MiB`)
      const name = typeof given.name === 'string' ? given.name : given.blob instanceof File ? given.blob.name : 'file'
      const made = (await ask({ op: 'add', name, media_type: given.blob.type || 'application/octet-stream', data: await base64Of(given.blob) })) as {
        id: string
        url: string
        size: number
        media_type: string
      }
      return { id: made.id, url: made.url, sizeBytes: made.size, contentType: made.media_type }
    },
  }
}

/** `files`: the thread's files, read from its history as the person, within what the version declares. */
function filesHandler(thread: string, artifact: ArtifactView, holder: Holder): CapabilityHandler {
  const ask = (asked: Record<string, unknown>) =>
    api.pageFiles(thread, artifact.id, asked).catch((failure: unknown) => {
      const said = failure instanceof Error ? failure.message : String(failure)
      throw new Refusal(/not_found|no file|had no/i.test(said) ? 'refused' : refusalOf(failure).code, said)
    })
  return {
    methods: ['list', 'read', 'download'],
    perMinute: 120,
    call: async (method, args) => {
      const given = (args ?? {}) as { path?: unknown; recursive?: unknown; cursor?: unknown; at?: unknown }
      const path = typeof given.path === 'string' ? given.path : ''
      if (method === 'list') return ask({ op: 'list', path, recursive: given.recursive === true, cursor: typeof given.cursor === 'string' ? given.cursor : null })
      const read = (await ask({ op: 'read', path, at: typeof given.at === 'string' ? given.at : null })) as { text: string; size: number; revision: string }
      if (method === 'read') return read
      const filename = downloadName(path.split('/').pop())
      if (!filename) throw new Refusal('refused', "that file isn't one a page may save")
      if (!(await holder.confirm({ title: 'Save a file from this thread?', detail: `${path} · ${sizeText(read.size)}`, action: 'Save' })))
        throw new Refusal('not_granted', 'the person chose not to save it')
      const url = URL.createObjectURL(new Blob([read.text], { type: 'text/plain' }))
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = filename
      anchor.click()
      setTimeout(() => URL.revokeObjectURL(url), 1000)
      return { saved: true }
    },
  }
}

/** Why a request failed, as the code a page reads. */
function refusalOf(failure: unknown): Refusal {
  const said = failure instanceof Error ? failure.message : String(failure)
  if (/conflict/i.test(said)) return new Refusal('conflict', said)
  if (/rate limited/i.test(said)) return new Refusal('rate_limited', said)
  if (/owner and collaborators|may not|forbidden/i.test(said)) return new Refusal('not_granted', said)
  return new Refusal('refused', said)
}

/** `artifact`: the page saves a whole new document as the next version, over the one it was opened at. */
function artifactHandler(thread: string, artifact: ArtifactView): CapabilityHandler {
  return {
    methods: ['publish', 'version'],
    perMinute: 12,
    call: async (method, args) => {
      if (method === 'version') return { version: artifact.version }
      const html = (args as { html?: unknown } | null)?.html
      if (typeof html !== 'string' || !/^\s*<!doctype html/i.test(html)) throw new Refusal('refused', 'publish takes a whole document, doctype first')
      try {
        return await api.publishArtifact(thread, artifact.id, html, artifact.version)
      } catch (failure) {
        throw refusalOf(failure)
      }
    },
  }
}

/** `user`: who views the page, as an id that's the same on every machine and the server, and the names behind ids. */
function userHandler(thread: string): CapabilityHandler {
  let viewer: Promise<PageViewer> | null = null
  const known = () => (viewer ??= api.pageViewer(thread))
  return {
    methods: ['id', 'me', 'isOwner', 'canEdit', 'can', 'profiles', 'search'],
    call: async (method, args) => {
      const me = await known()
      const name = (id: string) => me.people[id] ?? ''
      switch (method) {
        case 'id':
          return me.id
        case 'me':
          return { id: me.id, name: name(me.id) }
        case 'isOwner':
          return me.level === 'owner'
        case 'canEdit':
          return me.level === 'owner' || me.level === 'collaborator'
        case 'can': {
          const what = String((args as { name?: unknown } | null)?.name ?? '')
          if (what === 'data.write') return me.level !== 'read'
          if (what === 'artifact.publish') return me.level === 'owner' || me.level === 'collaborator'
          return null
        }
        case 'search': {
          // The thread's people whose names start a word with what was typed; everyone when it's empty.
          const typed = String((args as { query?: unknown } | null)?.query ?? '').trim().toLowerCase()
          return Object.entries(me.people)
            .filter(([, person]) => !typed || person.toLowerCase().split(/[\s._-]+/).some((word) => word.startsWith(typed)))
            .slice(0, 20)
            .map(([id, person]) => ({ id, name: person }))
        }
        default: {
          const ids = (args as { ids?: unknown } | null)?.ids
          if (!Array.isArray(ids)) throw new Refusal('refused', 'profiles takes a list of ids')
          return Object.fromEntries(ids.slice(0, 500).map((id) => [String(id), { id: String(id), name: name(String(id)) }]))
        }
      }
    },
  }
}

/** The most a download may be. */
export const DOWNLOAD_BYTES = 16 << 20

/** Types a page may not hand the person, as opening one runs it. */
const RUNNABLE = /\.(exe|msi|bat|cmd|com|scr|pif|ps1|vbs|vbe|jse?|wsf|jar|app|dmg|pkg|deb|rpm|apk|sh|command|lnk|reg|hta|cpl)$/i

/** A file name with nothing that names a folder, and not too long. */
export function downloadName(name: unknown): string | null {
  const plain = [...String(name ?? '')]
    .map((char) => (char.charCodeAt(0) < 32 ? '_' : char))
    .join('')
    .replace(/[/\\:*?"<>|]/g, '_')
    .replace(/^\.+/, '')
    .trim()
    .slice(0, 128)
  return plain && !RUNNABLE.test(plain) ? plain : null
}

/** `downloads`: a file the page made, saved only once the person agrees. */
function downloadsHandler(holder: Holder): CapabilityHandler {
  return {
    methods: ['save'],
    perMinute: 10,
    callBytes: DOWNLOAD_BYTES + 4096,
    call: async (_method, args) => {
      const given = (args ?? {}) as { filename?: unknown; data?: unknown; type?: unknown }
      const filename = downloadName(given.filename)
      if (!filename) throw new Refusal('refused', "that file name isn't one a page may save")
      const type = typeof given.type === 'string' && given.type.length < 128 ? given.type : 'application/octet-stream'
      const data = given.data
      const blob =
        data instanceof Blob
          ? data
          : typeof data === 'string' || data instanceof ArrayBuffer || ArrayBuffer.isView(data)
            ? new Blob([data as BlobPart], { type })
            : null
      if (!blob) throw new Refusal('refused', 'save takes text, bytes or a Blob')
      if (blob.size > DOWNLOAD_BYTES) throw new Refusal('too_large', `a download is at most ${DOWNLOAD_BYTES >> 20} MiB`)
      const agreed = await holder.confirm({ title: 'Save a file from this page?', detail: `${filename} · ${sizeText(blob.size)}`, action: 'Save' })
      if (!agreed) throw new Refusal('not_granted', 'the person chose not to save it')
      const url = URL.createObjectURL(blob)
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = filename
      anchor.click()
      setTimeout(() => URL.revokeObjectURL(url), 1000)
      return { saved: true }
    },
  }
}

function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1 << 20) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1 << 20)).toFixed(1)} MB`
}

/** `comments`: the page opens the artifact's own comment composer; the comments themselves stay Silt's to show. */
function commentsHandler(thread: string, artifact: ArtifactView, holder: Holder): CapabilityHandler {
  const writes = !composerOnly(artifact)
  let consent: Promise<boolean> | null = null
  const consented = () =>
    (consent ??= (async () => {
      const grants = await api.artifactGrants(thread, artifact.id).catch(() => ({}) as Record<string, string>)
      if (grants.comments === 'granted') return true
      if (grants.comments === 'refused') return false
      const agreed = await holder.confirm({
        title: `Let ${artifact.title} comment as you?`,
        detail: 'Comments it writes are yours, on this artifact, and go to the agent as yours do.',
        action: 'Allow',
      })
      await api.setArtifactGrant(thread, artifact.id, 'comments', agreed ? 'granted' : 'refused').catch(() => undefined)
      return agreed
    })())
  const said = (value: unknown) => {
    const text = typeof value === 'string' ? value.trim() : ''
    if (!text || text.length > 4000) throw new Refusal('refused', 'a comment is 1 to 4,000 characters')
    return text
  }
  const own = (value: unknown) => {
    const id = String(value ?? '')
    if (!holder.commentIds().includes(id)) throw new Refusal('refused', "that isn't a comment on this artifact")
    return id
  }
  return {
    methods: writes ? ['openComposer', 'add', 'reply', 'resolve'] : ['openComposer'],
    perMinute: 20,
    call: async (method, args) => {
      const given = (args ?? {}) as { text?: unknown; quote?: unknown; id?: unknown }
      if (method === 'openComposer') {
        holder.openComposer(typeof given.text === 'string' ? given.text.slice(0, 500) : '')
        return null
      }
      if (!(await consented())) throw new Refusal('not_granted', 'the viewer chose not to let this page comment as them')
      if (method === 'add') return { id: await holder.addComment(said(given.text), typeof given.quote === 'string' ? given.quote.slice(0, 500) : null) }
      const id = own(given.id)
      if (method === 'reply') await api.reply(thread, id, said(given.text))
      else await api.resolveComment(thread, id)
      holder.commentsChanged()
      return { id }
    },
  }
}

/** A page that declares `{comments: {composer_only: true}}` only opens the composer, and asks nothing. */
function composerOnly(artifact: ArtifactView): boolean {
  const declared = artifact.capabilities?.comments as { composer_only?: unknown } | undefined
  return declared?.composer_only === true
}

/** `sample`: a question for an agent, asked once the person agrees, its answer told to the page as it streams. */
function sampleHandler(thread: string, artifact: ArtifactView, holder: Holder): CapabilityHandler {
  let consent: Promise<boolean> | null = null
  const consented = () =>
    (consent ??= (async () => {
      const grants = await api.artifactGrants(thread, artifact.id).catch(() => ({}) as Record<string, string>)
      if (grants.sample === 'granted') return true
      if (grants.sample === 'refused') return false
      const agreed = await holder.confirm({
        title: `Let ${artifact.title} ask an agent?`,
        detail: hosted()
          ? 'One of your computers asks its agent, with no tools, and the page reads the answer.'
          : "This computer's agent answers, with no tools, and the page reads the answer.",
        action: 'Allow',
      })
      await api.setArtifactGrant(thread, artifact.id, 'sample', agreed ? 'granted' : 'refused').catch(() => undefined)
      return agreed
    })())
  return {
    methods: ['ask', 'limits'],
    perMinute: 4,
    call: async (method, args, context) => {
      if (method === 'limits') return { images: false, perMinute: 4 }
      const given = (args ?? {}) as { input?: unknown; ask?: unknown }
      if (!(await consented())) throw new Refusal('not_granted', 'the person chose not to let this page ask')
      const ask = typeof given.ask === 'string' && /^[A-Za-z0-9-]{1,64}$/.test(given.ask) ? given.ask : Math.random().toString(36).slice(2)
      const unsubscribe = bus.subscribe(`sample:${ask}`)
      const unlisten = bus.listen((event) => {
        const said = event as { event?: string; ask?: string; text?: string }
        if (said.event === 'sample' && said.ask === ask) context.emit('sample', 'text', { ask, text: said.text ?? '' })
      })
      try {
        return await api.sample(thread, artifact.id, ask, given.input)
      } catch (failure) {
        throw refusalOf(failure)
      } finally {
        unlisten()
        unsubscribe()
      }
    },
  }
}
