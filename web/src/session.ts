// A thread's live session, shared by its page and its Live page: the agent's
// state as it changes, and who else is here with their drafts.

import { useEffect, useRef, useState } from 'react'
import { api, type AgentStatus, type Presence, type Preset } from './api'
import { useBusEvents, useTopic } from './bus'

const HEARTBEAT_MS = 2000

export interface Live {
  /** How often the family grew since this view opened. */
  changes: number
  deltas: number | null
  agent: AgentStatus | null
  preset: Preset
  /** What the running tool printed so far. */
  tool: { id: string | null; text: string }
}

const NO_TOOL = { id: null, text: '' }

/** The thread's live state: fetched once, then followed on the event bus (ADR 0021). */
export function useLive(thread: string): Live & { setPreset: (preset: Preset) => void } {
  const [live, setLive] = useState<Live>({
    changes: 0,
    deltas: null,
    agent: null,
    preset: 'ask_before_commands',
    tool: NO_TOOL,
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
        })),
      () => undefined,
    )
    return () => {
      current = false
    }
  }, [thread, round])
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
      setLive((previous) => ({ ...previous, agent: event.status, tool: working ? previous.tool : NO_TOOL }))
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
