import { Suspense, lazy, useEffect, useRef, useState } from 'react'
import { api, type AgentStatus, type Presence, type Preset } from '../api'
import { useBusEvents, useTopic } from '../bus'
import { LiveReplica } from '../replica'
import { AgentPanel } from './Agent'

const HEARTBEAT_MS = 2000

interface Live {
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
function useLive(thread: string): Live & { setPreset: (preset: Preset) => void } {
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
function usePresence(
  thread: string,
  file: string | null,
  line: number | null,
  draft: string,
  onPeople: (people: Presence[]) => void,
): Presence[] {
  const [people, setPresent] = useState<Presence[]>([])
  const heard = useRef(onPeople)
  useEffect(() => {
    heard.current = onPeople
  })
  const setPeople = (present: Presence[]) => {
    setPresent(present)
    heard.current(present)
  }
  const state = useRef({ file, line, draft })
  useEffect(() => {
    state.current = { file, line, draft }
  }, [file, line, draft])
  useEffect(() => {
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
  }, [thread])
  useEffect(() => {
    void api.presence(thread, { file, line, draft }).then((reply) => setPeople(reply.people), () => undefined)
  }, [thread, file, line, draft])
  return people
}

const FileEditor = lazy(() => import('./LiveEditor').then((module) => ({ default: module.FileEditor })))

/** The thread's agent as the machine running it reports it, on a page the server serves (ADR 0026). */
export function HostedLivePanel({ thread }: { thread: string }) {
  const [draft, setDraft] = useState('')
  const { agent, tool, preset, setPreset } = useLive(thread)
  return (
    <section className="mt-6" aria-label="live">
      <h3 className="font-semibold">Live</h3>
      <label className="mt-2 block text-sm">
        Your next prompt
        <textarea
          aria-label="draft"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          className="mt-1 block w-full rounded border border-line p-2"
          rows={2}
        />
      </label>
      <AgentPanel
        thread={thread}
        status={agent}
        toolOutput={tool.text}
        preset={preset}
        onPreset={setPreset}
        draft={draft}
        onSent={() => setDraft('')}
        people={[]}
      />
    </section>
  )
}

/** Who else is here, their drafts and the thread's files, live (ADR 0014). */
export function LivePanel({
  thread,
  lastPrompt,
}: {
  thread: string
  /** The latest prompt in the conversation, to clear a draft once it was sent. */
  lastPrompt?: string
}) {
  const [replica, setReplica] = useState<LiveReplica>()
  const [error, setError] = useState<string>()
  const [file, setFile] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [line, setLine] = useState<number | null>(null)
  const [following, setFollowing] = useState<string | null>(null)
  const { changes, agent, tool, preset, setPreset } = useLive(thread)
  const people = usePresence(thread, file, line, draft, (present) => {
    const followed = present.find((person) => person.device === following)?.file
    if (followed) setFile(followed)
  })
  const leader = people.find((person) => person.device === following)
  const follow = (device: string | null) => {
    const followed = people.find((person) => person.device === device)?.file
    if (followed) setFile(followed)
    setFollowing(device)
  }
  const open = (path: string) => {
    setFollowing(null)
    setFile(path)
    setLine(null)
  }
  const sent = useRef(lastPrompt)
  useEffect(() => {
    if (lastPrompt !== sent.current && draft.trim() && lastPrompt?.includes(draft.trim())) setDraft('')
    sent.current = lastPrompt
  }, [lastPrompt, draft])

  useEffect(() => {
    let current = true
    LiveReplica.open(thread).then(
      (opened) => current && setReplica(opened),
      (e: Error) => current && setError(e.message),
    )
    return () => {
      current = false
    }
  }, [thread])

  return (
    <section className="mt-6" aria-label="live">
      <h3 className="font-semibold">Live</h3>
      <ul data-testid="presence" className="text-sm">
        {people.map((person) => (
          <li key={person.device}>
            <span className="font-medium">{person.user}</span> is here
            {person.file && <> viewing {person.file}</>}
            {person.file && person.line !== null && <> at line {person.line}</>}
            {person.draft && (
              <>
                {' '}
                and drafting: <span className="italic">{person.draft}</span>
              </>
            )}{' '}
            {person.file && (
              <button
                className="text-xs text-accent-fg"
                onClick={() => follow(following === person.device ? null : person.device)}
              >
                {following === person.device ? `Stop following ${person.user}` : `Follow ${person.user}`}
              </button>
            )}
          </li>
        ))}
        {people.length === 0 && <li className="text-fg-3">Nobody else is here.</li>}
      </ul>
      <label className="mt-2 block text-sm">
        Your draft of the next prompt
        <textarea
          aria-label="draft"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          className="mt-1 block w-full rounded border border-line p-2"
          rows={2}
        />
      </label>
      <AgentPanel
        thread={thread}
        status={agent}
        toolOutput={tool.text}
        preset={preset}
        onPreset={setPreset}
        draft={draft}
        onSent={() => setDraft('')}
        people={people}
      />
      {error && <p className="text-danger">{error}</p>}
      {replica && (
        <div className="mt-3 grid grid-cols-1 gap-3 md:grid-cols-[12rem_1fr]">
          <ul aria-label="files" className="max-h-96 overflow-y-auto text-sm">
            {replica.paths().map((path) => (
              <li key={path}>
                <button
                  className={`block w-full truncate text-left ${path === file ? 'font-semibold' : 'text-accent-fg'}`}
                  title={path}
                  onClick={() => open(path)}
                >
                  {path}
                </button>
              </li>
            ))}
          </ul>
          {file && (
            <Suspense fallback={<p className="text-xs text-fg-3">Opening the editor…</p>}>
              <FileEditor
                key={file}
                replica={replica}
                path={file}
                changes={changes}
                onLine={setLine}
                reveal={leader && leader.file === file ? leader.line : null}
                onType={() => setFollowing(null)}
              />
            </Suspense>
          )}
        </div>
      )}
    </section>
  )
}
