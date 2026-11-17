// The page's voice and connectors (ADRs 0079, 0080): dictation through the daemon's speech helper, speech through the
// browser's own voices, and MCP tools through the viewer's own daemon. Each asks the person in the holding page.

import { api, type ArtifactView } from '../api'
import { hosted } from '../served'
import { pageTake, type PageTake } from '../voice'
import { type CapabilityHandler, Refusal } from './broker'
import type { Holder } from './capabilities'
import { consentFor } from './consent'

/** Whether a take runs in any page, which speech waits for. */
let listening = false

/** `dictation`: the viewer's speech as text, a take at a time, started from the page's click. */
export function dictationHandler(thread: string, artifact: ArtifactView, holder: Holder): CapabilityHandler {
  let take: PageTake | null = null
  const consented = consentFor(thread, artifact, holder, 'dictation', () => ({
    title: `Let ${artifact.title} hear you?`,
    detail: 'Your words are turned into text on this computer, and the page reads the text.',
    action: 'Allow',
  }))
  return {
    methods: ['start', 'stop'],
    perMinute: 30,
    dispose: () => take?.stop(),
    call: async (method, _args, context) => {
      const state = (state: string, reason?: string) => context.emit('dictation', 'state', { state, reason })
      if (method === 'stop') {
        await take?.stop()
        return null
      }
      if (take) throw new Refusal('refused', 'a take is already running')
      if (!(await consented())) throw new Refusal('not_granted', 'the viewer chose not to let this page hear them')
      listening = true
      try {
        take = await pageTake({
          partial: (text) => context.emit('dictation', 'partial', { text }),
          final: (text) => context.emit('dictation', 'final', { text }),
          ended: (reason) => {
            take = null
            listening = false
            holder.voice?.('idle', null)
            state('idle', reason)
          },
        })
      } catch (failure) {
        listening = false
        throw new Refusal('unavailable', failure instanceof Error ? failure.message : String(failure))
      }
      holder.voice?.('listening', () => void take?.stop())
      state('listening')
      return null
    },
  }
}

/** Whether dictation can run in this view: the daemon's speech helper, never the hosted app. */
export function dictationServed(): boolean {
  return !hosted() && typeof navigator !== 'undefined' && Boolean(navigator.mediaDevices?.getUserMedia)
}

type Line = { text: string; voice?: string; rate?: number; language?: string; id?: string }

/** How often a reading tells the page how far into its line it is. */
const POSITION_MS = 250

/** `tts`: text spoken to the viewer in the browser's own voices, a reading at a time. */
export function ttsHandler(holder: Holder): CapabilityHandler {
  let rate = 1
  let reading: { cancel: () => void } | null = null
  let paused = false
  // How far into the line being spoken, told to the page as it goes; the browser's voices say only when a line starts.
  let ticking: number | null = null
  const stopTicking = () => {
    if (ticking !== null) window.clearInterval(ticking)
    ticking = null
  }
  const voiceFor = (named: string | undefined, language: string | undefined) => {
    const voices = speechSynthesis.getVoices()
    // Silt's voice names map onto whatever voices this browser has, alike each time.
    const order = ['buttery', 'airy', 'glassy', 'rounded']
    // Or one of this browser's own, by the name `voices()` gave.
    const own = voices.find((voice) => voice.name === named)
    if (own) return own
    const pool = language ? voices.filter((voice) => voice.lang.toLowerCase().startsWith(language.toLowerCase())) : voices
    const from = pool.length > 0 ? pool : voices
    if (from.length === 0) return undefined
    return from[Math.max(order.indexOf(named ?? 'buttery'), 0) % from.length]
  }
  return {
    methods: ['speak', 'add', 'stop', 'pause', 'resume', 'setRate', 'voices'],
    perMinute: 60,
    dispose: () => {
      stopTicking()
      reading?.cancel()
    },
    call: async (method, args, context) => {
      const state = (state: string, reason?: string) => context.emit('tts', 'state', { state, reason })
      const given = (args ?? {}) as { lines?: Line[]; rate?: number }
      if (method === 'stop') {
        reading?.cancel()
        return null
      }
      if (method === 'voices') return speechSynthesis.getVoices().map((voice) => ({ name: voice.name, language: voice.lang }))
      if (method === 'pause' || method === 'resume') {
        if (method === 'pause') speechSynthesis.pause()
        else speechSynthesis.resume()
        paused = method === 'pause'
        state(method === 'pause' ? 'paused' : 'speaking')
        return null
      }
      if (method === 'setRate') {
        rate = Math.min(Math.max(Number(given.rate) || 1, 0.7), 1.5)
        return null
      }
      if (listening) throw new Refusal('busy', 'the viewer is dictating')
      const lines = (Array.isArray(given.lines) ? given.lines : []).filter((line) => typeof line?.text === 'string' && line.text.trim())
      if (lines.length === 0 || lines.some((line) => line.text.length > 2000)) throw new Refusal('refused', 'speak takes lines of 1 to 2,000 characters')
      if (method === 'speak') reading?.cancel()
      return new Promise((done, fail) => {
        let stopped = false
        reading = {
          cancel: () => {
            stopped = true
            stopTicking()
            speechSynthesis.cancel()
            holder.voice?.('idle', null)
            state('idle', 'viewer_stopped')
            fail(new Refusal('viewer_stopped', 'the viewer stopped it'))
          },
        }
        lines.forEach((line, index) => {
          const utterance = new SpeechSynthesisUtterance(line.text)
          utterance.rate = Math.min(Math.max(line.rate ?? rate, 0.7), 1.5)
          const voice = voiceFor(line.voice, line.language)
          if (voice) utterance.voice = voice
          if (line.language) utterance.lang = line.language
          utterance.onstart = () => {
            holder.voice?.('speaking', () => reading?.cancel())
            state('speaking')
            context.emit('tts', 'line', { line: index, id: line.id ?? null })
            stopTicking()
            paused = false
            let ms = 0
            ticking = window.setInterval(() => {
              if (paused) return
              ms += POSITION_MS
              context.emit('tts', 'position', { ms, line: index })
            }, POSITION_MS)
          }
          utterance.onend = () => {
            stopTicking()
            if (index === lines.length - 1 && !stopped) {
              holder.voice?.('idle', null)
              state('idle', 'completed')
              done(null)
            }
          }
          speechSynthesis.speak(utterance)
        })
      })
    },
  }
}

export function ttsServed(): boolean {
  return typeof speechSynthesis !== 'undefined'
}

/** What a page's read-only tool calls answered last, by artifact and call, for as long as the app is open. */
const toolReads = new Map<string, { data: unknown; storedAt: number }>()
/** The most reads kept, the oldest going first. */
const TOOL_READS = 200

/** `mcp`: the viewer's page connectors' tools, through their own daemon. Reads run within the grant; every action is
 * confirmed by the viewer, each time. */
export function mcpHandler(thread: string, artifact: ArtifactView, holder: Holder): CapabilityHandler {
  const described = new Map<string, Promise<{ read_only: boolean; description: string | null; input_schema: unknown }>>()
  const describe = (server: string, tool: string) => {
    const key = `${server}/${tool}`
    if (!described.has(key))
      described.set(
        key,
        api.pageMcp(thread, artifact.id, { op: 'describe', server, tool }).then((answer) => answer as { read_only: boolean; description: string | null; input_schema: unknown }),
      )
    return described.get(key)!
  }
  const consented = consentFor(thread, artifact, holder, 'mcp', () => {
    const servers = ((artifact.capabilities?.mcp as { servers?: { server: string; tools: string[] }[] } | undefined)?.servers ?? [])
      .map((entry) => `${entry.server}: ${entry.tools.join(', ')}`)
      .join('; ')
    return { title: `Let ${artifact.title} use your connectors?`, detail: `${servers}. It reads what these return, as you.`, action: 'Allow' }
  })
  return {
    methods: ['callTool', 'describeTool', 'listTools', 'cached'],
    perMinute: 30,
    call: async (method, args) => {
      const given = (args ?? {}) as { server?: unknown; tool?: unknown; input?: unknown; read?: unknown }
      const server = String(given.server ?? '')
      const tool = String(given.tool ?? '')
      if (method === 'listTools') {
        const servers = (artifact.capabilities?.mcp as { servers?: { server: string; tools: string[] }[] } | undefined)?.servers ?? []
        return servers.find((entry) => entry.server === server)?.tools ?? []
      }
      if (!(await consented())) throw new Refusal('not_granted', 'the viewer chose not to let this page use their connectors')
      const readKey = `${artifact.id}\u0000${server}\u0000${tool}\u0000${JSON.stringify(given.input ?? {})}`
      if (method === 'cached') return toolReads.get(readKey) ?? null
      let about: Awaited<ReturnType<typeof describe>>
      try {
        about = await describe(server, tool)
      } catch (failure) {
        described.delete(`${server}/${tool}`)
        throw new Refusal(/not_granted/.test(String(failure)) ? 'not_granted' : 'unavailable', failure instanceof Error ? failure.message : String(failure))
      }
      if (method === 'describeTool') return about
      if (given.read === true && !about.read_only) throw new Refusal('refused', `${tool} isn't marked as only reading, so it can't be watched`)
      if (!about.read_only) {
        const input = JSON.stringify(given.input ?? {}).slice(0, 400)
        const agreed = await holder.confirm({ title: `Run ${server}'s ${tool}?`, detail: input, action: 'Run' })
        if (!agreed) throw new Refusal('not_granted', 'the viewer chose not to run it')
      }
      try {
        const result = await api.pageMcp(thread, artifact.id, { op: 'call', server, tool, input: given.input ?? {} })
        if (about.read_only) {
          toolReads.delete(readKey)
          toolReads.set(readKey, { data: result, storedAt: Date.now() })
          if (toolReads.size > TOOL_READS) toolReads.delete(toolReads.keys().next().value as string)
        }
        return result
      } catch (failure) {
        throw new Refusal('refused', failure instanceof Error ? failure.message : String(failure))
      }
    },
  }
}
