// Dictation in the composer (ADR 0059): the microphone's audio goes to the
// daemon, whose words stay in a preview until the take ends.

import { useQuery } from '@tanstack/react-query'
import { useCallback, useEffect, useRef, useState } from 'react'
import { type VoiceDownload, type VoiceEvent, api, dictationSocket } from './api'
import { keys } from './query'

const SPEECH_RATE = 16_000
/** A tenth of a second, so words show as they're said. */
const FRAME_SAMPLES = SPEECH_RATE / 10
const LONGEST_TAKE_MS = 5 * 60 * 1000
const FINISH_WAIT_MS = 10_000
const DOWNLOAD_POLL_MS = 500
const AUTO_SEND_KEY = 'silt-voice-auto-send'

/** Brings audio down to the speech rate, averaging what each output sample covers. */
export class Downsampler {
  private readonly ratio: number
  private position = 0
  private sum = 0
  private count = 0

  constructor(from: number, to = SPEECH_RATE) {
    if (from < to) throw new Error(`can't raise audio from ${from} Hz to ${to} Hz`)
    this.ratio = from / to
  }

  push(input: Float32Array): Float32Array {
    const output: number[] = []
    for (const sample of input) {
      this.sum += sample
      this.count += 1
      this.position += 1
      if (this.position >= this.ratio) {
        this.position -= this.ratio
        output.push(this.sum / this.count)
        this.sum = 0
        this.count = 0
      }
    }
    return Float32Array.from(output)
  }
}

/** Cuts samples of any length into frames of `size`. */
export class Framer {
  private held: number[] = []
  private readonly size: number

  constructor(size: number) {
    this.size = size
  }

  push(samples: Float32Array): Float32Array[] {
    this.held.push(...samples)
    const frames: Float32Array[] = []
    while (this.held.length >= this.size) frames.push(Float32Array.from(this.held.splice(0, this.size)))
    return frames
  }

  /** What's left over, if anything. */
  rest(): Float32Array | undefined {
    if (this.held.length === 0) return undefined
    const rest = Float32Array.from(this.held)
    this.held = []
    return rest
  }
}

/** `value` with `words` in place of what's between `start` and `end`, set off by spaces where they'd run together. */
export function insertWords(value: string, start: number, end: number, words: string): { value: string; caret: number } {
  const before = value.slice(0, start)
  const after = value.slice(end)
  const lead = before !== '' && !/\s$/.test(before) ? ' ' : ''
  const trail = after !== '' && !/^[\s.,;:!?)]/.test(after) ? ' ' : ''
  const inserted = `${lead}${words}${trail}`
  return { value: `${before}${inserted}${after}`, caret: before.length + inserted.length }
}

/** Why the microphone wouldn't open, said to the person. */
export function microphoneProblem(failure: unknown): string {
  const name = failure instanceof DOMException || failure instanceof Error ? failure.name : ''
  if (name === 'NotAllowedError') return 'Microphone access was refused.'
  if (name === 'NotFoundError') return 'No microphone was found.'
  if (name === 'InvalidStateError') return "The audio system wouldn't start. On Linux, install GStreamer's good plugins (gst-plugins-good)."
  return 'The microphone could not be opened.'
}

const WORKLET = `class Silt extends AudioWorkletProcessor {
  process(inputs) {
    const channels = inputs[0]
    if (channels.length > 0) {
      const mono = new Float32Array(channels[0].length)
      for (const channel of channels) channel.forEach((sample, index) => { mono[index] += sample / channels.length })
      this.port.postMessage(mono, [mono.buffer])
    }
    return true
  }
}
registerProcessor('silt-microphone', Silt)`

/** Opens the microphone, passing `onSamples` speech-rate audio; the result closes it. */
async function openMicrophone(onSamples: (samples: Float32Array) => void): Promise<() => void> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
  const stop = () => stream.getTracks().forEach((track) => track.stop())
  let context: AudioContext | undefined
  try {
    context = new AudioContext()
    await context.resume()
    const url = URL.createObjectURL(new Blob([WORKLET], { type: 'text/javascript' }))
    await context.audioWorklet.addModule(url)
    URL.revokeObjectURL(url)
    const node = new AudioWorkletNode(context, 'silt-microphone')
    const downsampler = new Downsampler(context.sampleRate)
    node.port.onmessage = (message: MessageEvent<Float32Array>) => onSamples(downsampler.push(message.data))
    context.createMediaStreamSource(stream).connect(node)
  } catch (failure) {
    stop()
    void context?.close()
    throw failure
  }
  const opened = context
  return () => {
    stop()
    void opened.close()
  }
}

export type DictationPhase = 'idle' | 'offering' | 'downloading' | 'starting' | 'recording' | 'finishing'

interface Take {
  socket: WebSocket | undefined
  closeMicrophone: (() => void) | undefined
  framer: Framer
  phrases: string[]
  /** The loudest sample so far: exactly none means a muted microphone. */
  peak: number
  send: boolean
  timers: ReturnType<typeof setTimeout>[]
}

export function microphoneAvailable(): boolean {
  return typeof navigator !== 'undefined' && navigator.mediaDevices?.getUserMedia !== undefined
}

/**
 * One composer's dictation. `onText` gets the words once a take ends, and
 * whether to send them. Only one take runs per composer, and it ends with it.
 */
export function useDictation({ enabled, composer, onText }: { enabled: boolean; composer: string; onText: (text: string, send: boolean) => void }) {
  const status = useQuery({ queryKey: keys.voice, queryFn: api.voiceStatus, enabled, staleTime: 30_000 })
  const available = enabled && status.data?.helper === true && microphoneAvailable()
  const [phase, setPhase] = useState<DictationPhase>('idle')
  const [phrases, setPhrases] = useState<string[]>([])
  const [partial, setPartial] = useState('')
  const [problem, setProblem] = useState<string>()
  const [download, setDownload] = useState<VoiceDownload>()
  const [autoSend, setAutoSendState] = useState(() => localStorage.getItem(AUTO_SEND_KEY) === '1')
  const take = useRef<Take | null>(null)
  const latestOnText = useRef(onText)
  useEffect(() => {
    latestOnText.current = onText
  }, [onText])
  const autoSendNow = useRef(autoSend)

  const setAutoSend = useCallback((on: boolean) => {
    autoSendNow.current = on
    setAutoSendState(on)
    localStorage.setItem(AUTO_SEND_KEY, on ? '1' : '0')
  }, [])

  const clear = useCallback(() => {
    setPhrases([])
    setPartial('')
    setPhase('idle')
  }, [])

  /** Lets go of everything the take holds. */
  const release = useCallback((current: Take) => {
    current.timers.forEach(clearTimeout)
    current.closeMicrophone?.()
    current.closeMicrophone = undefined
    current.socket?.close()
    if (take.current === current) take.current = null
  }, [])

  const discard = useCallback(() => {
    if (take.current) release(take.current)
    clear()
  }, [clear, release])

  const fail = useCallback(
    (current: Take, message: string) => {
      if (take.current !== current) return
      release(current)
      clear()
      setProblem(message)
    },
    [clear, release],
  )

  const finish = useCallback(
    (current: Take, send: boolean) => {
      if (take.current !== current || !current.closeMicrophone) return
      current.send = send
      current.closeMicrophone()
      current.closeMicrophone = undefined
      const rest = current.framer.rest()
      if (rest && current.socket?.readyState === WebSocket.OPEN) current.socket.send(rest.buffer as ArrayBuffer)
      current.socket?.send('finish')
      setPhase('finishing')
      current.timers.push(setTimeout(() => fail(current, 'Dictation took too long to finish.'), FINISH_WAIT_MS))
    },
    [fail],
  )

  const onEvent = useCallback(
    (current: Take, event: VoiceEvent) => {
      if (take.current !== current) return
      if (event.kind === 'partial') setPartial(event.text)
      else if (event.kind === 'segment') {
        current.phrases.push(event.text)
        setPhrases([...current.phrases])
        setPartial('')
      } else if (event.kind === 'paused') {
        if (autoSendNow.current) finish(current, true)
      } else if (event.kind === 'failed') fail(current, event.message)
      else {
        const text = current.phrases.join(' ')
        const { peak, send } = current
        release(current)
        clear()
        if (text) latestOnText.current(text, send)
        else setProblem(peak === 0 ? 'Silt heard only silence; your microphone may be muted.' : "Silt didn't catch any words.")
      }
    },
    [clear, fail, finish, release],
  )

  const begin = useCallback(async () => {
    const current: Take = { socket: undefined, closeMicrophone: undefined, framer: new Framer(FRAME_SAMPLES), phrases: [], peak: 0, send: false, timers: [] }
    take.current = current
    setPhase('starting')
    let closeMicrophone: () => void
    try {
      closeMicrophone = await openMicrophone((samples) => {
        for (const sample of samples) current.peak = Math.max(current.peak, Math.abs(sample))
        for (const frame of current.framer.push(samples)) {
          if (current.socket?.readyState === WebSocket.OPEN) current.socket.send(frame.buffer as ArrayBuffer)
        }
      })
    } catch (failure) {
      fail(current, microphoneProblem(failure))
      return
    }
    if (take.current !== current) {
      closeMicrophone()
      return
    }
    current.closeMicrophone = closeMicrophone
    const socket = dictationSocket()
    current.socket = socket
    socket.onmessage = (message: MessageEvent<string>) => onEvent(current, JSON.parse(message.data) as VoiceEvent)
    socket.onclose = () => fail(current, 'Dictation stopped unexpectedly.')
    socket.onopen = () => {
      if (take.current === current) setPhase('recording')
    }
    current.timers.push(setTimeout(() => finish(current, autoSendNow.current), LONGEST_TAKE_MS))
  }, [fail, finish, onEvent])

  const press = useCallback(async () => {
    setProblem(undefined)
    if (phase === 'recording' && take.current) {
      finish(take.current, autoSendNow.current)
      return
    }
    if (phase === 'offering') {
      setPhase('idle')
      return
    }
    if (phase !== 'idle') return
    try {
      const now = await api.voiceStatus()
      if (now.model) await begin()
      else setPhase('offering')
    } catch {
      setProblem('Silt could not reach the speech helper.')
    }
  }, [begin, finish, phase])

  const startDownload = useCallback(async () => {
    setPhase('downloading')
    setDownload(undefined)
    await api.voiceModel().catch(() => undefined)
  }, [])

  useEffect(() => {
    if (phase !== 'downloading') return undefined
    const poll = setInterval(() => {
      api
        .voiceStatus()
        .then((now) => {
          setDownload(now.download ?? undefined)
          if (now.model) setPhase('idle')
          else if (now.failed) {
            setProblem(`The speech model couldn't be downloaded: ${now.failed}`)
            setPhase('idle')
          }
        })
        .catch(() => undefined)
    }, DOWNLOAD_POLL_MS)
    return () => clearInterval(poll)
  }, [phase])

  // A take belongs to the composer it began in.
  useEffect(() => discard, [composer, discard])

  // Escape throws the take away, and nothing else that listens for it should act.
  useEffect(() => {
    if (phase !== 'starting' && phase !== 'recording') return undefined
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      event.stopPropagation()
      discard()
    }
    window.addEventListener('keydown', key, true)
    return () => window.removeEventListener('keydown', key, true)
  }, [phase, discard])

  return { available, phase, phrases, partial, problem, download, autoSend, setAutoSend, press, startDownload, dismissOffer: clear }
}

export type Dictation = ReturnType<typeof useDictation>
