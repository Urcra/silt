// A thread's live session, shared by its page and its Live page: the agent's
// state as it changes, and who else is here with their drafts.

import { useEffect, useRef, useState } from 'react'
import { api, type AgentStatus, type Presence, type Preset } from './api'
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

/** The thread's live state: fetched once, then followed on the event bus (ADR 0021). */
export function useLive(thread: string, total = 0): Live & { setPreset: (preset: Preset) => void } {
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
  const [round, setRound] = useState(0)
  useEffect(() => {
    let current = true
    api.live(thread).then(
      (state) =>
        current &&
        setLive((previous) => ({
          ...previous,
          changes: previous.deltas !== null && previous.deltas !== state.deltas ? previous.changes + 1 : previous.changes,
          deltas: state.deltas,
          agent: state.agent,
          preset: state.preset,
          said: NOTHING_SAID,
          began: entries.current,
        })),
      () => undefined,
    )
    return () => {
      current = false
    }
  }, [thread, round])
  useEffect(() => {
    if (live.said === NOTHING_SAID) return
    const forget = setTimeout(() => setLive((previous) => (previous.said === live.said ? { ...previous, said: NOTHING_SAID } : previous)), SAID_MS)
    return () => clearTimeout(forget)
  }, [live.said])
  useTopic(`thread:${thread}`)
  useBusEvents((event) => {
    if (event.event === 'lagged') {
      setRound((value) => value + 1)
      return
    }
    if (!('thread' in event) || event.thread !== thread) return
    if (event.event === 'thread') {
      setLive((previous) =>
        previous.deltas === event.deltas ? previous : { ...previous, deltas: event.deltas, changes: previous.changes + 1 },
      )
    } else if (event.event === 'agent') {
      const working = event.status?.state === 'working' || event.status?.state === 'awaiting_permission'
      setLive((previous) => {
        const before = previous.agent
        const now = event.status
        const text = now?.streaming ? '' : before?.streaming || previous.said.text
        const thought = now?.thinking ? '' : before?.thinking || previous.said.thought
        const kept = text === previous.said.text && thought === previous.said.thought
        const began = !before?.streaming && !before?.thinking && (now?.streaming || now?.thinking) ? entries.current : previous.began
        // Words kept across a change to the others keep where they began, or a later turn's thought would lose their entry.
        const textAfter = text === previous.said.text ? previous.said.textAfter : began
        const thoughtAfter = thought === previous.said.thought ? previous.said.thoughtAfter : began
        const said = kept ? previous.said : text || thought ? { text, thought, textAfter, thoughtAfter } : NOTHING_SAID
        return { ...previous, agent: now, tool: working ? previous.tool : NO_TOOL, said, began }
      })
    } else if (event.event === 'agent_text' && event.kind === 'tool') {
      setLive((previous) => {
        const same = !event.restart && previous.tool.id === event.tool_use_id
        return { ...previous, tool: { id: event.tool_use_id, text: same ? previous.tool.text + event.text : event.text } }
      })
    }
  })
  return { ...live, setPreset: (preset) => setLive((previous) => ({ ...previous, preset })) }
}

/** Tells the others what this viewer is doing; returns what they are doing. */
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
  const setPeople = (present: Presence[]) => {
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
  useEffect(() => {
    if (!enabled) return
    void api.presence(thread, { file, line, draft }).then((reply) => setPeople(reply.people), () => undefined)
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
