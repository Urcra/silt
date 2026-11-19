// A thread's live session, shared by its page and its Live page: the agent's
// state as it changes, and who else is here with their drafts.

import { useEffect, useRef, useState } from 'react'
import { api, type AgentStatus, type BusEvent, type Presence, type Preset } from './api'
import { useBusEvents, useTopic } from './bus'
import { NOTHING_SAID, type Said } from './unrecorded'

const HEARTBEAT_MS = 2000

export interface Live {
  /** How often the family grew since this view opened. */
  changes: number
  deltas: number | null
  agent: AgentStatus | null
  preset: Preset
  /** What the running tool printed so far. */
  tool: { id: string | null; text: string }
  /** What the agent last streamed, kept once its status drops it, until the history has it. */
  said: Said
  /** How many entries the history had when the agent last began to say something. */
  began: number
}

const NO_TOOL = { id: null, text: '' }

/** How long what the agent said is kept for a history that never brings it, as one that words it otherwise. */
const SAID_MS = 5000

/** Runs `run` before the next frame is drawn, or soon in a page that draws none, as one hidden. */
function nextFrame(run: () => void): number {
  if (typeof requestAnimationFrame === 'function' && document.visibilityState === 'visible') return requestAnimationFrame(run)
  return -Number(setTimeout(run, 50))
}

function cancelFrame(frame: number) {
  if (frame < 0) clearTimeout(-frame)
  else cancelAnimationFrame(frame)
}

/** `live` once `event` is heard; `entries` is how many the history has now. */
export function heard(live: Live, event: BusEvent, entries: number): Live {
  if (event.event === 'thread') {
    return live.deltas === event.deltas ? live : { ...live, deltas: event.deltas, changes: live.changes + 1 }
  }
  if (event.event === 'agent') {
    const working = event.status?.state === 'working' || event.status?.state === 'awaiting_permission'
    const before = live.agent
    const now = event.status
    const text = now?.streaming ? '' : before?.streaming || live.said.text
    const thought = now?.thinking ? '' : before?.thinking || live.said.thought
    const kept = text === live.said.text && thought === live.said.thought
    const began = !before?.streaming && !before?.thinking && (now?.streaming || now?.thinking) ? entries : live.began
    // Words kept across a change to the others keep where they began, or a later turn's thought would lose their entry.
    const textAfter = text === live.said.text ? live.said.textAfter : began
    const thoughtAfter = thought === live.said.thought ? live.said.thoughtAfter : began
    const said = kept ? live.said : text || thought ? { text, thought, textAfter, thoughtAfter } : NOTHING_SAID
    return { ...live, agent: now, tool: working ? live.tool : NO_TOOL, said, began }
  }
  if (event.event !== 'agent_text') return live
  if (event.kind === 'tool') {
    const same = !event.restart && live.tool.id === event.tool_use_id
    return { ...live, tool: { id: event.tool_use_id, text: same ? live.tool.text + event.text : event.text } }
  }
  // The daemon sends the agent's whole status only now and then while it streams; the words add up here between.
  const agent = live.agent
  if (!agent) return live
  const began = !agent.streaming && !agent.thinking ? entries : live.began
  return event.kind === 'message'
    ? { ...live, agent: { ...agent, streaming: agent.streaming + event.text }, began }
    : { ...live, agent: { ...agent, thinking: agent.thinking + event.text }, began }
}

/** The thread's live state: fetched once, then followed on the event bus (ADR 0021). */
export function useLive(thread: string, total = 0): Live & { heard: boolean; setPreset: (preset: Preset) => void } {
  const entries = useRef(total)
  useEffect(() => {
    entries.current = total
  }, [total])
  const [live, setLive] = useState<Live>({
    changes: 0,
    deltas: null,
    agent: null,
    preset: 'ask_before_commands',
    tool: NO_TOOL,
    said: NOTHING_SAID,
    began: 0,
  })
  // `agent` is null both before the page hears and when no agent runs; only the second may start one.
  const [heardFor, setHeardFor] = useState<string>()
  const [round, setRound] = useState(0)
  useEffect(() => {
    let current = true
    api.live(thread).then(
      (state) => {
        if (!current) return
        setHeardFor(thread)
        setLive((previous) => ({
          ...previous,
          changes: previous.deltas !== null && previous.deltas !== state.deltas ? previous.changes + 1 : previous.changes,
          deltas: state.deltas,
          agent: state.agent,
          preset: state.preset,
          said: NOTHING_SAID,
          began: entries.current,
        }))
      },
      () => current && setHeardFor(thread),
    )
    return () => {
      current = false
    }
  }, [thread, round])
  useEffect(() => {
    if (live.said === NOTHING_SAID) return
    const forget = setTimeout(
      () => setLive((previous) => (previous.said === live.said ? { ...previous, said: NOTHING_SAID } : previous)),
      SAID_MS,
    )
    return () => clearTimeout(forget)
  }, [live.said])
  useTopic(`thread:${thread}`)
  // Heard events are applied once a frame: an agent streaming a long reply sends hundreds a second, and drawing the
  // page for each was most of what the browser did.
  const heldEvents = useRef<BusEvent[]>([])
  const flushing = useRef<number | null>(null)
  useEffect(
    () => () => {
      if (flushing.current !== null) cancelFrame(flushing.current)
      flushing.current = null
      heldEvents.current = []
    },
    [thread],
  )
  useBusEvents((event) => {
    if (event.event === 'lagged') {
      setRound((value) => value + 1)
      return
    }
    if (!('thread' in event) || event.thread !== thread) return
    if (event.event !== 'thread' && event.event !== 'agent' && event.event !== 'agent_text') return
    heldEvents.current.push(event)
    flushing.current ??= nextFrame(() => {
      flushing.current = null
      const events = heldEvents.current
      heldEvents.current = []
      if (events.some((held) => held.event === 'agent')) setHeardFor(thread)
      setLive((previous) => events.reduce((state, held) => heard(state, held, entries.current), previous))
    })
  })
  return { ...live, heard: heardFor === thread, setPreset: (preset) => setLive((previous) => ({ ...previous, preset })) }
}

/** How long typing pauses before the draft is sent to the others; the heartbeat sends it too. */
const DRAFT_SEND_MS = 250

function samePeople(a: Presence[], b: Presence[]): boolean {
  return a.length === b.length && JSON.stringify(a) === JSON.stringify(b)
}

/** Tells the others what this viewer is doing; returns what they are doing, the same array until that changes. */
export function usePresence(
  thread: string,
  file: string | null,
  line: number | null,
  draft: string,
  onPeople?: (people: Presence[]) => void,
  /** Off where nobody can be present, as on a page the server hosts. */
  enabled = true,
): Presence[] {
  const [people, setPresent] = useState<Presence[]>([])
  const heard = useRef(onPeople)
  useEffect(() => {
    heard.current = onPeople
  })
  const shown = useRef(people)
  const setPeople = (present: Presence[]) => {
    if (samePeople(shown.current, present)) return
    shown.current = present
    setPresent(present)
    heard.current?.(present)
  }
  const state = useRef({ file, line, draft })
  useEffect(() => {
    state.current = { file, line, draft }
  }, [file, line, draft])
  useEffect(() => {
    if (!enabled) return
    let alive = true
    const beat = () =>
      api.presence(thread, state.current).then(
        (reply) => alive && setPeople(reply.people),
        () => undefined,
      )
    void beat()
    const timer = setInterval(() => void beat(), HEARTBEAT_MS)
    return () => {
      alive = false
      clearInterval(timer)
      void api.presence(thread, { ...state.current, leaving: true }).catch(() => undefined)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [thread, enabled])
  // Each key typed was a request, and a draw of the page for its answer; where the cursor is goes at once.
  const moved = useRef({ file, line })
  useEffect(() => {
    if (!enabled) return
    const typed = moved.current.file === file && moved.current.line === line
    moved.current = { file, line }
    const send = setTimeout(
      () =>
        void api.presence(thread, { file, line, draft }).then(
          (reply) => setPeople(reply.people),
          () => undefined,
        ),
      typed ? DRAFT_SEND_MS : 0,
    )
    return () => clearTimeout(send)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [thread, file, line, draft, enabled])
  return people
}

/** Clears a draft once the conversation shows it was sent, by anyone. */
export function useClearSentDraft(lastPrompt: string | undefined, draft: string, clear: () => void) {
  const sent = useRef(lastPrompt)
  useEffect(() => {
    if (lastPrompt !== sent.current && draft.trim() && lastPrompt?.includes(draft.trim())) clear()
    sent.current = lastPrompt
  }, [lastPrompt, draft, clear])
}
