// Dictation in the composer (ADR 0059): the microphone's audio goes to the
// daemon, whose words stay in a preview until the take ends.

import { useQuery } from '@tanstack/react-query'
import { type KeyboardEvent as ReactKeyboardEvent, useCallback, useEffect, useRef, useState } from 'react'
import { type VoiceDownload, type VoiceEvent, api, dictationSocket } from './api'
import { keys } from './query'

const SPEECH_RATE = 16_000
/** A tenth of a second, so words show as they're said. */
const FRAME_SAMPLES = SPEECH_RATE / 10
const LONGEST_TAKE_MS = 5 * 60 * 1000
const FINISH_WAIT_MS = 10_000
const DOWNLOAD_POLL_MS = 500
/** How many of the latest frames' levels the recording bar draws. */
const LEVELS_KEPT = 160
/** With not one sample above silence this long, the microphone is likely muted. */
const QUIET_AFTER_MS = 1500
/** Held at least this long, the dictation key talks while held; shorter, it's a tap that toggles. */
export const HOLD_MS = 350
const HINT_KEY = 'silt-voice-hint-sessions'
const HINT_SESSIONS = 3

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

/** How loud a frame is, from 0 to 1, on a scale that makes speech fill most of it. */
export function level(frame: Float32Array): number {
  if (frame.length === 0) return 0
  let sum = 0
  for (const sample of frame) sum += sample * sample
  const rms = Math.sqrt(sum / frame.length)
  return Math.min(1, Math.sqrt(rms * 8))
}

/** Whether a press of the dictation key that lasted from `down` to `up` held it, or tapped it. */
export function pressed(down: number, up: number): 'hold' | 'tap' {
  return up - down >= HOLD_MS ? 'hold' : 'tap'
}

/** The dictation key, as the composer names it. */
export const DICTATION_KEYS = ['Ctrl', 'M']

/** The dictation key: Ctrl+M, Control even on a Mac, where ⌘M minimises the window. */
export function isDictationKey(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>): boolean {
  return event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 'm'
}

let hintCounted = false
/** Whether the composer still says how to send and talk: in the first few sessions only, counted once a page load. */
export function showKeyHint(): boolean {
  try {
    const seen = Number(localStorage.getItem(HINT_KEY) ?? '0')
    if (!hintCounted) {
      hintCounted = true
      localStorage.setItem(HINT_KEY, String(seen + 1))
      return seen + 1 <= HINT_SESSIONS
    }
    return seen <= HINT_SESSIONS
  } catch {
    return false
  }
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
  /** Let go of before it was recording: it ends as soon as it is. */
  endWhenOpen: boolean
  timers: ReturnType<typeof setTimeout>[]
}

/** Something that stopped a take or came of it; `note` is no fault, such as a take with no words. */
export interface DictationProblem {
  text: string
  tone: 'note' | 'danger'
}

export function microphoneAvailable(): boolean {
  return typeof navigator !== 'undefined' && navigator.mediaDevices?.getUserMedia !== undefined
}

/**
 * One composer's dictation. `onText` gets the words once a take ends, and
 * whether to send them. Only one take runs per composer, and it ends with it.
 */
export function useDictation({
  enabled,
  composer,
  onBegin,
  onText,
}: {
  enabled: boolean
  composer: string
  /** As a take is asked for, before anything of it shows. */
  onBegin?: () => void
  onText: (text: string, send: boolean) => void
}) {
  const status = useQuery({ queryKey: keys.voice, queryFn: api.voiceStatus, enabled, staleTime: 30_000 })
  const available = enabled && status.data?.helper === true && microphoneAvailable()
  const [phase, setPhase] = useState<DictationPhase>('idle')
  const [phrases, setPhrases] = useState<string[]>([])
  const [partial, setPartial] = useState('')
  const [problem, setProblem] = useState<DictationProblem>()
  const [download, setDownload] = useState<VoiceDownload>()
  const [levels, setLevels] = useState<number[]>([])
  const [quiet, setQuiet] = useState(false)
  const [since, setSince] = useState<number>()
  /** Talking while the key is held, rather than until it's pressed again. */
  const [holding, setHolding] = useState(false)
  const take = useRef<Take | null>(null)
  const latestOnText = useRef(onText)
  const latestOnBegin = useRef(onBegin)
  useEffect(() => {
    latestOnText.current = onText
    latestOnBegin.current = onBegin
  }, [onText, onBegin])

  const clear = useCallback(() => {
    setPhrases([])
    setPartial('')
    setLevels([])
    setQuiet(false)
    setSince(undefined)
    setHolding(false)
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
      setProblem({ text: message, tone: 'danger' })
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
      } else if (event.kind === 'failed') fail(current, event.message)
      else if (event.kind === 'done') {
        const text = current.phrases.join(' ')
        const { peak, send } = current
        release(current)
        clear()
        if (text) latestOnText.current(text, send)
        else
          setProblem({
            text: peak === 0 ? 'Silt heard only silence; your microphone may be muted.' : "Silt didn't catch any words. Nothing was added.",
            tone: 'note',
          })
      }
    },
    [clear, fail, release],
  )

  const begin = useCallback(async () => {
    const current: Take = {
      socket: undefined,
      closeMicrophone: undefined,
      framer: new Framer(FRAME_SAMPLES),
      phrases: [],
      peak: 0,
      send: false,
      endWhenOpen: false,
      timers: [],
    }
    take.current = current
    setPhase('starting')
    let closeMicrophone: () => void
    try {
      closeMicrophone = await openMicrophone((samples) => {
        for (const sample of samples) current.peak = Math.max(current.peak, Math.abs(sample))
        for (const frame of current.framer.push(samples)) {
          if (take.current === current) setLevels((shown) => [...shown.slice(1 - LEVELS_KEPT), level(frame)])
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
      if (take.current !== current) return
      setPhase('recording')
      setSince(Date.now())
      current.timers.push(setTimeout(() => take.current === current && current.peak === 0 && setQuiet(true), QUIET_AFTER_MS))
      if (current.endWhenOpen) finish(current, current.send)
    }
    current.timers.push(setTimeout(() => finish(current, false), LONGEST_TAKE_MS))
  }, [fail, finish, onEvent])

  /** Starts a take, or offers the model first where it isn't here yet. */
  const start = useCallback(async () => {
    setProblem(undefined)
    if (phase === 'offering') {
      setPhase('idle')
      return
    }
    if (phase !== 'idle') return
    latestOnBegin.current?.()
    try {
      const now = await api.voiceStatus()
      if (now.model) await begin()
      else setPhase('offering')
    } catch {
      setProblem({ text: 'Silt could not reach the speech helper.', tone: 'danger' })
    }
  }, [begin, phase])

  /** Ends the take, the words going into the draft, and sent with it when `send`. */
  const end = useCallback(
    (send: boolean) => {
      const current = take.current
      if (!current) return
      if (current.closeMicrophone && current.socket?.readyState === WebSocket.OPEN) finish(current, send)
      else {
        current.endWhenOpen = true
        current.send = send
      }
    },
    [finish],
  )

  /** The mic button: starts a take, or ends one keeping its words. */
  const press = useCallback(async () => {
    if (phase === 'recording' || phase === 'starting') end(false)
    else await start()
  }, [end, phase, start])

  // The dictation key, Ctrl+M: held, it talks until it's let go; tapped, it starts a take that a second tap ends.
  const keyDown = useRef<number | null>(null)
  const onKey = useCallback(
    (event: KeyboardEvent | ReactKeyboardEvent) => {
      if (!available || !isDictationKey(event)) return false
      event.preventDefault()
      if (event.repeat) return true
      if (take.current) {
        end(false)
        return true
      }
      const down = Date.now()
      keyDown.current = down
      // Past the hold, the take lasts only while the key does.
      setTimeout(() => keyDown.current === down && setHolding(true), HOLD_MS)
      void start()
      return true
    },
    [available, end, start],
  )
  useEffect(() => {
    const up = (event: KeyboardEvent) => {
      if (keyDown.current === null || (event.key.toLowerCase() !== 'm' && event.key !== 'Control')) return
      const down = keyDown.current
      keyDown.current = null
      if (pressed(down, Date.now()) === 'hold') end(false)
    }
    window.addEventListener('keyup', up)
    return () => window.removeEventListener('keyup', up)
  }, [end])

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
            setProblem({ text: `The speech model couldn't be downloaded: ${now.failed}`, tone: 'danger' })
            setPhase('idle')
          }
        })
        .catch(() => undefined)
    }, DOWNLOAD_POLL_MS)
    return () => clearInterval(poll)
  }, [phase])

  // A take belongs to the composer it began in.
  useEffect(() => discard, [composer, discard])

  // Escape throws the take away and ⏎ sends it, and nothing else that listens for either should act.
  useEffect(() => {
    if (phase !== 'starting' && phase !== 'recording') return undefined
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') discard()
      else if (event.key === 'Enter' && !event.shiftKey && !event.isComposing && !(event.target instanceof HTMLButtonElement)) end(true)
      else return
      event.preventDefault()
      event.stopPropagation()
    }
    window.addEventListener('keydown', key, true)
    return () => window.removeEventListener('keydown', key, true)
  }, [phase, discard, end])

  return {
    available,
    phase,
    phrases,
    partial,
    problem,
    download,
    levels,
    quiet,
    since,
    holding,
    press,
    keep: () => end(false),
    send: () => end(true),
    discard,
    onKey,
    startDownload,
    dismissOffer: clear,
  }
}

/** Whether the row under the box has something to say of dictation: the download, or what came of a take. */
export function hasVoiceNote(dictation: Dictation): boolean {
  return dictation.phase === 'downloading' || dictation.problem !== undefined
}

/** Whether a take is under way, from asking for the microphone until its words are in. */
export function taking(phase: DictationPhase): boolean {
  return phase === 'starting' || phase === 'recording' || phase === 'finishing'
}

export type Dictation = ReturnType<typeof useDictation>
