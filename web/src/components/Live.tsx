import { EditorView, basicSetup } from 'codemirror'
import { useEffect, useRef, useState } from 'react'
import { api, type LiveState, type Presence } from '../api'
import { LiveReplica } from '../replica'
import { AgentPanel } from './Agent'

const HEARTBEAT_MS = 2000
const EDIT_DEBOUNCE_MS = 150

/** The thread's live state as the daemon streams it, and how often it changed. */
function useLive(thread: string): { changes: number; state: LiveState | null } {
  const [live, setLive] = useState<{ changes: number; state: LiveState | null }>({ changes: 0, state: null })
  useEffect(() => {
    const source = new EventSource(api.liveUrl(thread))
    source.addEventListener('state', (event) =>
      setLive((previous) => {
        const state = JSON.parse((event as MessageEvent<string>).data) as LiveState
        const grew = previous.state === null || previous.state.deltas !== state.deltas
        return { changes: previous.changes + (grew ? 1 : 0), state }
      }),
    )
    return () => source.close()
  }, [thread])
  return live
}

/** Tells the others what this viewer is doing; returns what they are doing. */
function usePresence(thread: string, file: string | null, draft: string): Presence[] {
  const [people, setPeople] = useState<Presence[]>([])
  const state = useRef({ file, draft })
  useEffect(() => {
    state.current = { file, draft }
  }, [file, draft])
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
    void api.presence(thread, { file, draft }).then((reply) => setPeople(reply.people), () => undefined)
  }, [thread, file, draft])
  return people
}

/** Applies `next` to the editor as the smallest change, so cursors keep their place. */
function replaceText(view: EditorView, next: string) {
  const current = view.state.doc.toString()
  if (current === next) return
  let start = 0
  while (start < current.length && start < next.length && current[start] === next[start]) start++
  let end = 0
  while (
    end < current.length - start &&
    end < next.length - start &&
    current[current.length - 1 - end] === next[next.length - 1 - end]
  )
    end++
  view.dispatch({ changes: { from: start, to: current.length - end, insert: next.slice(start, next.length - end) } })
}

function FileEditor({ replica, path, changes }: { replica: LiveReplica; path: string; changes: number }) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null)
  const applying = useRef(false)
  const [error, setError] = useState<string>()

  const flush = async () => {
    if (pending.current) clearTimeout(pending.current)
    pending.current = null
    if (view.current) await replica.edit(path, view.current.state.doc.toString()).catch((e: Error) => setError(e.message))
  }

  useEffect(() => {
    let current = true
    void replica.text(path).then((text) => {
      if (!current || !host.current) return
      view.current = new EditorView({
        doc: text ?? '',
        parent: host.current,
        extensions: [
          basicSetup,
          EditorView.editable.of(replica.writable),
          EditorView.updateListener.of((update) => {
            if (!update.docChanged || applying.current) return
            if (pending.current) clearTimeout(pending.current)
            pending.current = setTimeout(() => void flush(), EDIT_DEBOUNCE_MS)
          }),
        ],
      })
    })
    return () => {
      current = false
      view.current?.destroy()
      view.current = null
    }
    // The editor is made once per file; later text arrives through `changes`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [replica, path])

  useEffect(() => {
    if (changes === 0) return
    void (async () => {
      await flush()
      await replica.sync()
      const text = await replica.text(path)
      if (view.current && text !== undefined) {
        applying.current = true
        replaceText(view.current, text)
        applying.current = false
      }
    })()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [changes])

  return (
    <div>
      {!replica.writable && <p className="text-xs text-neutral-500">Read only: ask the owner to share the thread to edit it.</p>}
      {error && <p className="text-xs text-rose-700">{error}</p>}
      <div ref={host} data-testid="editor" className="rounded border border-neutral-300 text-sm" />
    </div>
  )
}

/** Who else is here, their drafts and the thread's files, live (ADR 0014). */
export function LivePanel({
  thread,
  onChange,
  lastPrompt,
}: {
  thread: string
  onChange?: () => void
  /** The latest prompt in the conversation, to clear a draft once it was sent. */
  lastPrompt?: string
}) {
  const [replica, setReplica] = useState<LiveReplica>()
  const [error, setError] = useState<string>()
  const [file, setFile] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const { changes, state } = useLive(thread)
  const people = usePresence(thread, file, draft)
  const changed = useRef(onChange)
  useEffect(() => {
    changed.current = onChange
  })
  useEffect(() => {
    if (changes > 0) changed.current?.()
  }, [changes])
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
            {person.draft && (
              <>
                {' '}
                and drafting: <span className="italic">{person.draft}</span>
              </>
            )}
          </li>
        ))}
        {people.length === 0 && <li className="text-neutral-500">Nobody else is here.</li>}
      </ul>
      <label className="mt-2 block text-sm">
        Your draft of the next prompt
        <textarea
          aria-label="draft"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          className="mt-1 block w-full rounded border border-neutral-300 p-2"
          rows={2}
        />
      </label>
      <AgentPanel thread={thread} status={state?.agent ?? null} draft={draft} onSent={() => setDraft('')} people={people} />
      {error && <p className="text-rose-700">{error}</p>}
      {replica && (
        <div className="mt-3 grid grid-cols-[12rem_1fr] gap-3">
          <ul aria-label="files" className="text-sm">
            {replica.paths().map((path) => (
              <li key={path}>
                <button className={path === file ? 'font-semibold' : 'text-sky-700'} onClick={() => setFile(path)}>
                  {path}
                </button>
              </li>
            ))}
          </ul>
          {file && <FileEditor key={file} replica={replica} path={file} changes={changes} />}
        </div>
      )}
    </section>
  )
}
