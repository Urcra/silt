// The capabilities this view serves an artifact's page, by name (ADR 0072). A page that asks for one not here gets
// null.

import { api, type ArtifactView, type PageViewer } from '../api'
import { bus } from '../bus'
import { hosted } from '../served'
import { type CapabilityHandler, Refusal } from './broker'
import { dbHandler } from './db'
import { dictationHandler, dictationServed, mcpHandler, ttsHandler, ttsServed } from './voiceAndTools'

/** What a capability asks of the page holding the frame: the person's say before something acts for them. */
export type Holder = {
  /** Asks the person in the holding page itself, never in a browser dialog, and answers whether they agreed. */
  confirm: (ask: { title: string; detail: string; action: string }) => Promise<boolean>
  /** Opens the artifact's comment composer, quoting `text`. */
  openComposer: (text: string) => void
}

export function handlersFor(thread: string, artifact: ArtifactView, holder: Holder): Record<string, CapabilityHandler> {
  return {
    db: dbHandler(thread, artifact),
    artifact: artifactHandler(thread, artifact),
    user: userHandler(thread),
    downloads: downloadsHandler(holder),
    comments: commentsHandler(holder),
    sample: sampleHandler(thread, artifact, holder),
    assets: assetsHandler(thread, artifact),
    files: filesHandler(thread, artifact, holder),
    ...(dictationServed() ? { dictation: dictationHandler(thread, artifact, holder) } : {}),
    ...(ttsServed() ? { tts: ttsHandler() } : {}),
    mcp: mcpHandler(thread, artifact, holder),
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
    methods: ['id', 'me', 'isOwner', 'canEdit', 'can', 'profiles'],
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
function commentsHandler(holder: Holder): CapabilityHandler {
  return {
    methods: ['openComposer'],
    perMinute: 20,
    call: async (_method, args) => {
      const text = (args as { text?: unknown } | null)?.text
      holder.openComposer(typeof text === 'string' ? text.slice(0, 500) : '')
      return null
    },
  }
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
