// The capabilities this view serves an artifact's page, by name (ADR 0072). A page that asks for one not here gets
// null.

import { api, ApiError, type ArtifactView, type PageViewer, type SampleTool } from '../api'
import { bus } from '../bus'
import { hosted } from '../served'
import { type CapabilityHandler, Refusal } from './broker'
import { RoomSocket } from '../rooms'
import { consentFor, refusedHere } from './consent'
import { dbHandler } from './db'
import { dictationHandler, dictationServed, mcpHandler, ttsHandler, ttsServed } from './voiceAndTools'

/** What a capability asks of the page holding the frame: the person's say before something acts for them. */
export type Holder = {
  /** Asks the person in the holding page itself, never in a browser dialog, and answers whether they agreed. */
  confirm: (ask: { title: string; detail: string; action: string }) => Promise<boolean>
  /** Opens the artifact's comment composer, quoting `text`, on the page's element `anchor` where it names one. */
  openComposer: (text: string, anchor?: string | null) => void
  /** Comments as the viewer on the version shown, or on the page's element `anchor`, quoting `quote`; answers its id. */
  addComment: (text: string, quote: string | null, anchor?: string | null) => Promise<string>
  /** The ids of the comments on this artifact, which a page may reply to and resolve. */
  commentIds: () => string[]
  /** Shows what a page's reply or resolve changed. */
  commentsChanged: () => void
  /** Handed what shows a comment's element in the page, while a page is open (ADR 0085). */
  revealer?: (reveal: ((anchor: string) => void) | null) => void
  /** Shows, outside the page, that it listens or speaks, with what stops it (ADR 0079). */
  voice?: (now: 'listening' | 'speaking' | 'idle', stop: (() => void) | null) => void
}

export function handlersFor(thread: string, artifact: ArtifactView, holder: Holder): Record<string, CapabilityHandler> {
  const served = {
    db: dbHandler(thread, artifact),
    artifact: artifactHandler(thread, artifact),
    user: userHandler(thread, artifact),
    downloads: downloadsHandler(holder),
    comments: commentsHandler(thread, artifact, holder),
    sample: sampleHandler(thread, artifact, holder),
    assets: assetsHandler(thread, artifact),
    files: filesHandler(thread, artifact, holder),
    ...(dictationServed() ? { dictation: dictationHandler(thread, artifact, holder) } : {}),
    ...(ttsServed() ? { tts: ttsHandler(holder) } : {}),
    mcp: mcpHandler(thread, artifact, holder),
    room: roomHandler(thread, artifact),
  }
  return { ...served, permissions: permissionsHandler(thread, artifact, holder, Object.keys(served)) }
}

/** The capabilities that ask the viewer before they act, which `permissions` reports on. */
const ASKED = ['sample', 'dictation', 'mcp', 'comments', 'files']

/**
 * `permissions`, built in: what the viewer allowed this page, asked for at once, or forgotten so it asks again. A
 * capability this view doesn't serve, as dictation in the hosted app, is `unavailable`; one declared that never asks,
 * such as `db`, is `granted` wherever it's served.
 */
function permissionsHandler(thread: string, artifact: ArtifactView, holder: Holder, served: string[]): CapabilityHandler {
  const declared = () => ASKED.filter((name) => Boolean(artifact.capabilities?.[name]) && !(name === 'comments' && composerOnly(artifact)))
  const states = async () => {
    const grants = await api.artifactGrants(thread, artifact.id).catch(() => ({}) as Record<string, string>)
    const refused = refusedHere(holder)
    const state = (name: string) =>
      !served.includes(name) ? 'unavailable' : grants[name] === 'granted' ? 'granted' : refused.has(name) ? 'denied' : 'prompt'
    const asking = declared()
    const quiet = Object.keys(artifact.capabilities ?? {}).filter((name) => !asking.includes(name) && name !== 'edit' && name !== 'permissions')
    return Object.fromEntries([
      ...asking.map((name) => [name, state(name)]),
      ...quiet.map((name) => [name, served.includes(name) ? 'granted' : 'unavailable']),
    ])
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
          for (const name of pending) {
            if (agreed) await api.setArtifactGrant(thread, artifact.id, name, 'granted', artifact.version).catch(() => undefined)
            else refusedHere(holder).add(name)
          }
        }
        const now = await states()
        return Object.fromEntries(wanted.map((name) => [name, now[name] ?? 'unavailable']))
      }
      const forget = await holder.confirm({ title: `Forget what you allowed ${artifact.title}?`, detail: 'It asks again the next time it needs something.', action: 'Forget' })
      if (forget) {
        for (const name of declared()) await api.setArtifactGrant(thread, artifact.id, name, null).catch(() => undefined)
        refusedHere(holder).clear()
      }
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
  const consented = consentFor(thread, artifact, holder, 'files', () => {
    const paths = (artifact.capabilities?.files as { paths?: unknown } | undefined)?.paths
    const scope = Array.isArray(paths) && paths.length > 0 ? paths.map(String).join(', ') : "all of the thread's files its workspace allows"
    return { title: `Let ${artifact.title} read this thread's files?`, detail: `It reads ${scope}, and can show what it reads to anyone viewing it.`, action: 'Allow' }
  })
  const ask = async (asked: Record<string, unknown>) => {
    if (!(await consented())) throw new Refusal('not_granted', "the person chose not to let this page read the thread's files")
    return api.pageFiles(thread, artifact.id, asked).catch((failure: unknown) => {
      const said = failure instanceof Error ? failure.message : String(failure)
      throw new Refusal(/too_large|over 4 MiB/i.test(said) ? 'too_large' : /not_found|no file|had no/i.test(said) ? 'refused' : refusalOf(failure).code, said)
    })
  }
  return {
    methods: ['list', 'read', 'history', 'download'],
    perMinute: 120,
    // A text file is at most 4 MiB, which its JSON may double.
    answerBytes: FILE_ANSWER_BYTES,
    call: async (method, args) => {
      const given = (args ?? {}) as { path?: unknown; recursive?: unknown; cursor?: unknown; at?: unknown }
      const path = typeof given.path === 'string' ? given.path : ''
      if (method === 'list') return ask({ op: 'list', path, recursive: given.recursive === true, cursor: typeof given.cursor === 'string' ? given.cursor : null })
      if (method === 'history') return ask({ op: 'history', path })
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
  const code = failure instanceof ApiError ? failure.refusal : null
  return new Refusal(code === 'conflict' || code === 'rate_limited' || code === 'not_granted' ? code : 'refused', said)
}

/** The largest answer `files` gives: a text file of 4 MiB, as JSON. */
export const FILE_ANSWER_BYTES = (8 << 20) + 4096
/** The largest page `artifact.publish` takes, as the agent's are (ADR 0072). */
export const PUBLISH_BYTES = 10 << 20

/** `artifact`: the page saves a whole new document as the next version, over the one it was opened at. */
function artifactHandler(thread: string, artifact: ArtifactView): CapabilityHandler {
  return {
    methods: ['publish', 'version'],
    perMinute: 12,
    callBytes: PUBLISH_BYTES + 4096,
    call: async (method, args) => {
      if (method === 'version') return { version: artifact.version }
      const html = (args as { html?: unknown } | null)?.html
      if (typeof html !== 'string' || !/^\s*<!doctype html/i.test(html)) throw new Refusal('refused', 'publish takes a whole document, doctype first')
      if (new Blob([html]).size > PUBLISH_BYTES) throw new Refusal('too_large', 'a page is at most 10 MiB')
      try {
        return await api.publishArtifact(thread, artifact.id, html, artifact.version)
      } catch (failure) {
        throw refusalOf(failure)
      }
    },
  }
}

/** `user`: who views the page, as an id that's the same on every machine and the server, and the names behind ids. */
function userHandler(thread: string, artifact: ArtifactView): CapabilityHandler {
  let viewer: Promise<PageViewer> | null = null
  const known = () => (viewer ??= api.pageViewer(thread))
  // People's names reach a page only when it declares the `profile` scope; Silt keeps no email addresses.
  const scopes = (artifact.capabilities?.user as { scopes?: unknown } | undefined)?.scopes
  const names = Array.isArray(scopes) && scopes.includes('profile')
  return {
    methods: ['id', 'me', 'isOwner', 'canEdit', 'can', 'profiles', ...(names ? ['search'] : [])],
    call: async (method, args) => {
      const me = await known()
      const name = (id: string) => (names ? (me.people[id] ?? '') : '')
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
  const consented = consentFor(thread, artifact, holder, 'comments', () => ({
    title: `Let ${artifact.title} comment as you?`,
    detail: 'Comments it writes are yours, on this artifact, and go to the agent as yours do.',
    action: 'Allow',
  }))
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
      const given = (args ?? {}) as { text?: unknown; quote?: unknown; id?: unknown; anchor?: unknown }
      const anchor = typeof given.anchor === 'string' && given.anchor.trim() ? given.anchor.trim().slice(0, 300) : null
      if (method === 'openComposer') {
        holder.openComposer(typeof given.text === 'string' ? given.text.slice(0, 500) : '', anchor)
        return null
      }
      if (!(await consented())) throw new Refusal('not_granted', 'the viewer chose not to let this page comment as them')
      if (method === 'add')
        return { id: await holder.addComment(said(given.text), typeof given.quote === 'string' ? given.quote.slice(0, 500) : null, anchor) }
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

/** The tools a page offers its question, as the daemon takes them: names, descriptions and input schemas, never code. */
function sampleTools(given: unknown): SampleTool[] {
  if (!Array.isArray(given)) return []
  return given.map((tool: { name?: unknown; description?: unknown; inputSchema?: unknown }) => ({
    name: String(tool?.name ?? ''),
    description: typeof tool?.description === 'string' ? tool.description : '',
    input_schema: tool?.inputSchema && typeof tool.inputSchema === 'object' ? tool.inputSchema : undefined,
  }))
}

/** The most images one of a page's questions shows the agent, as the daemon and server take them. */
const SAMPLE_IMAGES = 4

/** How long a page's question asked again gets the answer it had. */
const SAMPLE_CACHE_MS = 5 * 60_000

/** `sample`: a question for an agent, asked once the person agrees, its answer told to the page as it streams. */
function sampleHandler(thread: string, artifact: ArtifactView, holder: Holder): CapabilityHandler {
  // The page's names for its questions, each to the one minted for it.
  const asks = new Map<string, string>()
  // Answers to questions asked again within five minutes, as they were, unless the page says `cache: false`.
  const answered = new Map<string, { at: number; answer: { text: string; truncated: boolean; modelApplied?: string } }>()
  const takesImages = (artifact.capabilities?.sample as { images?: unknown } | undefined)?.images === true
  const consented = consentFor(thread, artifact, holder, 'sample', () => ({
    title: `Let ${artifact.title} ask an agent?`,
    detail: hosted()
      ? 'One of your computers asks its agent, which gets no tools or settings of yours, only those the page offers, and the page reads the answer.'
      : "This computer's agent answers, with no tools or settings of yours, only those the page offers, and the page reads the answer.",
    action: 'Allow',
  }))
  return {
    methods: ['ask', 'limits', 'toolResult', 'stop'],
    // Each question asks once; its tools' answers are many.
    perMinute: 120,
    call: async (method, args, context) => {
      if (method === 'limits') return { images: takesImages ? SAMPLE_IMAGES : false, perMinute: 4, tools: true }
      if (method === 'stop') {
        // The page stopped asking: this computer's agent stops too; in the hosted app the device's run ends on its own.
        const minted = asks.get(String((args as { ask?: unknown } | null)?.ask ?? ''))
        if (minted && !hosted()) await api.sampleStop(thread, artifact.id, minted).catch(() => undefined)
        return null
      }
      if (method === 'toolResult') {
        const said = (args ?? {}) as { ask?: unknown; call?: unknown; ok?: unknown; value?: unknown; error?: unknown }
        const result = said.ok === true ? { ok: true, value: said.value ?? null } : { ok: false, error: String(said.error ?? 'the tool failed').slice(0, 2000) }
        const minted = asks.get(String(said.ask ?? ''))
        if (!minted) throw new Refusal('refused', 'no question by that name is waiting')
        await api.sampleToolResult(thread, artifact.id, minted, Number(said.call), result)
        return null
      }
      const given = (args ?? {}) as { input?: unknown; ask?: unknown; images?: unknown; tools?: unknown; cache?: unknown; model?: { model_tier?: unknown; model?: unknown } }
      const model = {
        model_tier: typeof given.model?.model_tier === 'string' ? given.model.model_tier : undefined,
        model: typeof given.model?.model === 'string' ? given.model.model : undefined,
      }
      const tools = sampleTools(given.tools)
      const blobs = Array.isArray(given.images) ? given.images : []
      if (blobs.length > 0 && !takesImages) throw new Refusal('refused', 'declare {sample: {images: true}} to show the agent images')
      if (blobs.length > SAMPLE_IMAGES || !blobs.every((blob) => blob instanceof Blob)) throw new Refusal('refused', `show at most ${SAMPLE_IMAGES} images, as Blobs`)
      if (!(await consented())) throw new Refusal('not_granted', 'the person chose not to let this page ask')
      // Only a question with no tools or images is the same question asked again.
      const key = given.cache === true && tools.length === 0 && blobs.length === 0 ? JSON.stringify([given.input, model]) : null
      const kept = key === null ? undefined : answered.get(key)
      if (kept && Date.now() - kept.at < SAMPLE_CACHE_MS) {
        const named = typeof given.ask === 'string' && /^[A-Za-z0-9-]{1,64}$/.test(given.ask) ? given.ask : 'q'
        context.emit('sample', 'text', { ask: named, text: kept.answer.text })
        return kept.answer
      }
      const images = await Promise.all((blobs as Blob[]).map(async (blob) => ({ media_type: blob.type, data: await base64Of(blob) })))
      // The page's name for the question stays between it and this view; the
      // daemon and the server hear one minted here, which no other page can pick.
      const named = typeof given.ask === 'string' && /^[A-Za-z0-9-]{1,64}$/.test(given.ask) ? given.ask : 'q'
      const ask = `${artifact.id}-${crypto.randomUUID()}`
      asks.set(named, ask)
      const unsubscribe = bus.subscribe(`sample:${ask}`)
      const unlisten = bus.listen((event) => {
        const said = event as { event?: string; ask?: string; text?: string }
        if (said.event === 'sample' && said.ask === ask) context.emit('sample', 'text', { ask: named, text: said.text ?? '' })
        const called = event as { event?: string; ask?: string; call?: number; name?: string; input?: unknown }
        if (called.event === 'sample_tool' && called.ask === ask) context.emit('sample', 'tool', { ask: named, call: called.call, name: called.name, input: called.input })
      })
      // The hosted app has no bus: it reads what the device's agent has said so far.
      let heard = ''
      // The calls of the page's tools already told to the page.
      const told = new Set<number>()
      const polling = hosted()
        ? window.setInterval(() => {
            void api
              .sampleSoFar(thread, artifact.id, ask)
              .then(({ text, calls }) => {
                if (text && text !== heard) context.emit('sample', 'text', { ask: named, text: (heard = text) })
                for (const call of calls ?? []) {
                  if (told.has(call.call)) continue
                  told.add(call.call)
                  context.emit('sample', 'tool', { ask: named, call: call.call, name: call.name, input: call.input })
                }
              })
              .catch(() => undefined)
          }, 400)
        : null
      try {
        const answer = await api.sample(thread, artifact.id, ask, given.input, images, tools, model)
        const shown = { text: answer.text, truncated: answer.truncated, ...(answer.model_applied ? { modelApplied: answer.model_applied } : {}) }
        if (key !== null) answered.set(key, { at: Date.now(), answer: shown })
        return shown
      } catch (failure) {
        throw refusalOf(failure)
      } finally {
        if (polling !== null) window.clearInterval(polling)
        if (asks.get(named) === ask) asks.delete(named)
        unlisten()
        unsubscribe()
      }
    },
  }
}
