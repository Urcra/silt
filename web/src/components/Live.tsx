import { type Extension, Compartment, RangeSetBuilder, StateEffect, StateField } from '@codemirror/state'
import { Decoration, type DecorationSet, EditorView } from '@codemirror/view'
import { useQuery } from '@tanstack/react-query'
import { Link, useParams } from '@tanstack/react-router'
import { basicSetup } from 'codemirror'
import { useCallback, useEffect, useRef, useState } from 'react'
import { api, short, type Presence } from '../api'
import { STATE_PHRASE, running } from '../agentState'
import { languageOf } from '../highlight'
import { keys, message } from '../query'
import { LiveReplica } from '../replica'
import { useClearSentDraft, useLive, usePresence } from '../session'
import { Avatar, Button, Dot, Icon, Skeleton, StatusBar, Textarea, cx } from '../ui'
import { AgentPicker } from './Agent'
import { Markdown } from './Markdown'
import { Mock } from './Mock'
import { Page } from './Page'

const EDIT_DEBOUNCE_MS = 150

/** Puts the cursor on `line` and scrolls it into the middle. */
function showLine(view: EditorView, line: number) {
  const at = view.state.doc.line(Math.min(Math.max(line, 1), view.state.doc.lines)).from
  view.dispatch({ selection: { anchor: at }, effects: EditorView.scrollIntoView(at, { y: 'center' }) })
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

/** Nous in the editor: its surfaces, gutters and selection, light and dark. */
const NOUS = EditorView.theme({
  '&': { backgroundColor: 'var(--bg-elevated)', color: 'var(--fg-1)', fontSize: '12px', height: '100%' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '1.6' },
  '.cm-gutters': { backgroundColor: 'var(--bg-elevated)', color: 'var(--fg-2)', border: 'none' },
  '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'var(--accent-tint)' },
  '.cm-cursor': { borderLeftColor: 'var(--fg-1)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': { backgroundColor: 'var(--accent-tint)' },
  '&.cm-focused': { outline: 'none' },
  '.cm-remote-line': { backgroundColor: 'var(--info-tint)' },
  '.cm-remote-line::after': {
    content: 'attr(data-who)',
    marginLeft: '8px',
    padding: '0 4px',
    borderRadius: '2px',
    backgroundColor: 'var(--slate-600)',
    color: 'var(--paper-50)',
    fontFamily: 'var(--font-sans)',
    fontSize: '10px',
  },
})

/** The lines others here are on, as `[line, name]`. */
const setRemote = StateEffect.define<[number, string][]>()

/** Each other person's line, tinted and labelled with their name. */
const remoteLines = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(lines, transaction) {
    let next = lines.map(transaction.changes)
    for (const effect of transaction.effects) {
      if (!effect.is(setRemote)) continue
      const builder = new RangeSetBuilder<Decoration>()
      const doc = transaction.state.doc
      const sorted = [...effect.value].filter(([line]) => line >= 1 && line <= doc.lines).sort(([a], [b]) => a - b)
      for (const [line, name] of sorted) {
        // The name is drawn by CSS from the attribute, so it isn't part of the line's text.
        builder.add(doc.line(line).from, doc.line(line).from, Decoration.line({ class: 'cm-remote-line', attributes: { 'data-who': name } }))
      }
      next = builder.finish()
    }
    return next
  },
  provide: (field) => EditorView.decorations.from(field),
})

/** The editor's language, loaded the first time a file of it opens. */
async function languageFor(path: string): Promise<Extension> {
  const { syntaxHighlighting, StreamLanguage } = await import('@codemirror/language')
  const { classHighlighter } = await import('@lezer/highlight')
  const style = syntaxHighlighting(classHighlighter)
  switch (languageOf(path)) {
    case 'rust':
      return [(await import('@codemirror/lang-rust')).rust(), style]
    case 'ts':
    case 'tsx':
    case 'js':
    case 'jsx': {
      const language = languageOf(path)
      return [(await import('@codemirror/lang-javascript')).javascript({ typescript: language === 'ts' || language === 'tsx', jsx: language === 'tsx' || language === 'jsx' }), style]
    }
    case 'python':
      return [(await import('@codemirror/lang-python')).python(), style]
    case 'json':
      return [(await import('@codemirror/lang-json')).json(), style]
    case 'css':
      return [(await import('@codemirror/lang-css')).css(), style]
    case 'html':
      return [(await import('@codemirror/lang-html')).html(), style]
    case 'markdown':
      return [(await import('@codemirror/lang-markdown')).markdown(), style]
    case 'toml':
      return [StreamLanguage.define((await import('@codemirror/legacy-modes/mode/toml')).toml), style]
    case 'yaml':
      return [StreamLanguage.define((await import('@codemirror/legacy-modes/mode/yaml')).yaml), style]
    case 'shell':
      return [StreamLanguage.define((await import('@codemirror/legacy-modes/mode/shell')).shell), style]
    default:
      return []
  }
}

function FileEditor({
  replica,
  path,
  changes,
  others,
  onCursor,
  reveal,
  onType,
}: {
  replica: LiveReplica
  path: string
  changes: number
  /** Everyone else on this file, by line. */
  others: [number, string][]
  /** Where this viewer's cursor moved, from 1. */
  onCursor: (line: number, column: number) => void
  /** A line to show, from someone being followed. */
  reveal: number | null
  /** This viewer typed, which ends following someone. */
  onType: () => void
}) {
  const cursorMoved = useRef(onCursor)
  const typed = useRef(onType)
  const revealed = useRef(reveal)
  const remote = useRef(others)
  useEffect(() => {
    cursorMoved.current = onCursor
    typed.current = onType
    revealed.current = reveal
    remote.current = others
  })
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
    const language = new Compartment()
    void replica.show(path).then((text) => {
      if (!current || !host.current) return
      view.current = new EditorView({
        doc: text ?? '',
        parent: host.current,
        extensions: [
          basicSetup,
          NOUS,
          remoteLines,
          language.of([]),
          EditorView.editable.of(replica.writable),
          EditorView.domEventHandlers({ keydown: () => typed.current() }),
          EditorView.updateListener.of((update) => {
            if (update.selectionSet || update.docChanged) {
              const head = update.state.selection.main.head
              const line = update.state.doc.lineAt(head)
              cursorMoved.current(line.number, head - line.from + 1)
            }
            if (!update.docChanged || applying.current) return
            if (pending.current) clearTimeout(pending.current)
            pending.current = setTimeout(() => void flush(), EDIT_DEBOUNCE_MS)
          }),
        ],
      })
      view.current.dispatch({ effects: setRemote.of(remote.current) })
      if (revealed.current !== null) showLine(view.current, revealed.current)
      void languageFor(path).then((extension) => {
        if (current) view.current?.dispatch({ effects: language.reconfigure(extension) })
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
      await replica.prepare(path)
      if (!view.current) return
      // From here on nothing awaits, so no keystroke can land between taking
      // the editor's text and replacing it with the merged one.
      if (pending.current) clearTimeout(pending.current)
      pending.current = null
      const sent = replica.edit(path, view.current.state.doc.toString())
      const text = replica.showNow(path)
      if (text !== undefined) {
        applying.current = true
        replaceText(view.current, text)
        applying.current = false
      }
      await sent.catch((e: Error) => setError(e.message))
    })()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [changes])

  const marks = others.map(([line, name]) => `${line}:${name}`).join('|')
  useEffect(() => {
    view.current?.dispatch({ effects: setRemote.of(remote.current) })
  }, [marks])

  useEffect(() => {
    if (view.current && reveal !== null) showLine(view.current, reveal)
  }, [reveal, path])

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {!replica.writable && <p className="border-b border-line px-4 py-1.5 text-xs text-fg-2">Read only: ask the owner to share the thread to edit it.</p>}
      {error && <p className="border-b border-line px-4 py-1.5 text-xs text-danger">{error}</p>}
      <div ref={host} data-testid="editor" className="min-h-[320px] flex-1 overflow-hidden" />
    </div>
  )
}

/** The files among the replica's paths, leaving out the directories that hold them. */
function filesOf(paths: string[]): string[] {
  return paths.filter((path) => !paths.some((other) => other.startsWith(`${path}/`)))
}

/** Where someone is, as `file:line`. */
function whereIs(person: Presence): string | null {
  if (!person.file) return null
  const name = person.file.split('/').pop() ?? person.file
  return person.line !== null ? `${name}:${person.line}` : name
}

/**
 * A thread's live session (ADR 0014, the Live session board): its files,
 * edited together with everyone here, who is where, the agent, and the
 * next prompt drafted together.
 */
export function LivePage() {
  const { id } = useParams({ from: '/t/$id/live' })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const summary = threads.data?.find((thread) => thread.id.endsWith(id))
  if (threads.error) return <p className="p-6 text-danger">{message(threads.error)}</p>
  if (!summary) return <Skeleton className="m-6 h-40" />
  return <LiveSession thread={summary.id} title={summary.title ?? 'Untitled thread'} />
}

function LiveSession({ thread, title }: { thread: string; title: string }) {
  const [replica, setReplica] = useState<LiveReplica>()
  const [error, setError] = useState<string>()
  const [file, setFile] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [cursor, setCursor] = useState<{ line: number; column: number } | null>(null)
  const [following, setFollowing] = useState<string | null>(null)
  const [agent, setAgent] = useState('')
  const [sendError, setSendError] = useState<string>()
  const live = useLive(thread)
  const signedIn = useQuery({ queryKey: keys.status, queryFn: api.status })
  const me = signedIn.data?.servers?.[0]?.user
  const device = signedIn.data?.servers?.[0]?.name ?? 'this machine'
  const history = useQuery({ queryKey: keys.history(thread), queryFn: () => api.history(thread) })
  const lastPrompt = history.data?.entries.findLast((entry) => entry.kind === 'user')?.text
  const clearDraft = useCallback(() => setDraft(''), [])
  useClearSentDraft(lastPrompt, draft, clearDraft)
  const people = usePresence(thread, file, cursor?.line ?? null, draft, (present) => {
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
    setCursor(null)
  }

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

  const status = live.agent
  const running_ = status !== null && running(status.state)
  const elsewhere = people.find((person) => person.agent)
  const drafting = people.filter((person) => person.draft.trim())
  const ready = status?.state === 'idle' || status?.state === 'awaiting_input'
  const send = () => {
    setSendError(undefined)
    api.prompt(thread, draft).then(() => setDraft(''), (failure: unknown) => setSendError(message(failure)))
  }

  const session = (
    <div className="flex flex-col gap-5 px-4 py-4">
      <section aria-label="here now" className="flex flex-col gap-2">
        <h2 className="font-mono text-2xs font-medium tracking-[0.06em] text-fg-2 uppercase">Here now</h2>
        <ul data-testid="presence" className="flex flex-col gap-2 text-xs">
          {people.map((person) => (
            <li key={person.device} className="flex items-center gap-2">
              <Avatar name={person.user} size={20} />
              <span className="min-w-0 flex-1 truncate" title={person.file ?? undefined}>
                <b className="font-medium">{person.user}</b>{' '}
                <span className="font-mono text-2xs text-fg-2">{whereIs(person) ?? 'here'}</span>
              </span>
              {person.file && (
                <Button
                  variant={following === person.device ? 'secondary' : 'ghost'}
                  size="sm"
                  aria-pressed={following === person.device}
                  aria-label={following === person.device ? `Stop following ${person.user}` : `Follow ${person.user}`}
                  onClick={() => follow(following === person.device ? null : person.device)}
                >
                  {following === person.device ? 'Following' : 'Follow'}
                </Button>
              )}
            </li>
          ))}
          {people.length === 0 && <li className="text-fg-2">Nobody else is here.</li>}
          <li className="flex items-center gap-2 text-fg-2">
            <Avatar name={me ?? 'you'} size={20} />
            <b className="font-medium text-fg-1">you</b>
            <span>on {device}</span>
          </li>
        </ul>
      </section>
      <section aria-label="agent" className="flex flex-col gap-2">
        <h2 className="font-mono text-2xs font-medium tracking-[0.06em] text-fg-2 uppercase">Agent</h2>
        {running_ ? (
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <Dot tone={status.state === 'working' ? 'ok' : status.state.startsWith('awaiting') ? 'warn' : 'idle'} />
            <span data-testid="agent-state">
              <b className="font-medium">{status.agent}</b> {STATE_PHRASE[status.state]}
            </span>
            {status.state === 'working' && (
              <Button variant="ghost" size="sm" className="ml-auto" onClick={() => void api.cancelTurn(thread)}>
                Cancel turn
              </Button>
            )}
          </div>
        ) : elsewhere?.agent ? (
          <p data-testid="agent-state" className="text-xs">
            {elsewhere.user}&apos;s {elsewhere.agent.name} {STATE_PHRASE[elsewhere.agent.state]}
          </p>
        ) : (
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <span className="text-fg-2">No agent here.</span>
            <AgentPicker value={agent} onChange={setAgent} />
            <Button variant="primary" size="sm" disabled={!agent} onClick={() => void api.startAgent(agent, { thread }).catch((failure: unknown) => setSendError(message(failure)))}>
              Start agent
            </Button>
          </div>
        )}
        {status?.streaming && <Markdown text={status.streaming} className="text-xs text-fg-2" />}
      </section>
      <section aria-label="next prompt" className="flex flex-col gap-2">
        <h2 className="font-mono text-2xs font-medium tracking-[0.06em] text-fg-2 uppercase">Next prompt, drafted together</h2>
        {drafting.map((person) => (
          <div key={person.device} className="rounded-sm border border-line bg-elevated px-2.5 py-2 text-xs">
            <span className="font-mono text-2xs text-fg-2">{person.user} is drafting</span>
            <p className="mt-0.5 whitespace-pre-wrap">{person.draft}</p>
          </div>
        ))}
        <label htmlFor="live-draft" className="text-xs text-fg-2">
          Your draft
        </label>
        <Textarea id="live-draft" aria-label="draft" rows={3} value={draft} onChange={(event) => setDraft(event.target.value)} />
        <div className="flex items-center gap-2">
          <Button variant="primary" disabled={!running_ || !ready || !draft.trim()} onClick={send}>
            {drafting.length > 0 ? 'Send all as the prompt' : 'Send as the prompt'}
          </Button>
          {!running_ && <span className="text-2xs text-fg-2">Start an agent to send it.</span>}
        </div>
        {sendError && <p className="text-xs text-danger">{sendError}</p>}
      </section>
    </div>
  )

  return (
    <Page
      flush
      label="live session"
      inspector={session}
      inspectorLabel="session"
      title={
        <span className="flex items-center gap-2">
          Live
          <Link to="/t/$id" params={{ id: thread }} className="text-sm font-normal text-accent-fg hover:text-fg-1">
            {title}
          </Link>
        </span>
      }
      meta={<span data-volatile>{short(thread)}</span>}
      actions={
        <span className="inline-flex items-center gap-2">
          <span className="flex -space-x-1.5">
            {people.slice(0, 4).map((person) => (
              <span key={person.device} className="rounded-full ring-2 ring-bg">
                <Avatar name={person.user} size={22} />
              </span>
            ))}
          </span>
          <span className="inline-flex items-center gap-1.5 rounded-sm border border-line-subtle bg-surface-1 px-2 py-0.5 font-mono text-2xs text-fg-2">
            <Dot tone="ok" />
            {people.length + 1} here
          </span>
        </span>
      }
    >
      {error && <p className="px-6 py-3 text-danger">{error}</p>}
      {!replica && !error && <Skeleton className="m-6 h-40" />}
      {replica && (
        <div className="flex flex-1 flex-col md:min-h-[calc(100dvh-130px)] md:flex-row">
          <nav aria-label="files" className="flex max-h-60 shrink-0 flex-col gap-0.5 overflow-y-auto border-b border-line p-2 md:max-h-none md:w-60 md:border-r md:border-b-0">
            <p className="px-2 pb-1.5 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">Files in the thread</p>
            {filesOf(replica.paths()).map((path) => {
              const on = people.filter((person) => person.file === path)
              return (
                <button
                  key={path}
                  type="button"
                  aria-current={path === file || undefined}
                  className={cx(
                    'flex items-center gap-2 rounded-sm px-2 py-1 text-left font-mono text-2xs hover:bg-surface-2 max-md:min-h-11',
                    path === file ? 'bg-surface-2 text-fg-1' : 'text-fg-2',
                  )}
                  title={path}
                  onClick={() => open(path)}
                >
                  <span className="min-w-0 flex-1 truncate">{path}</span>
                  {on.map((person) => (
                    <Avatar key={person.device} name={person.user} size={14} />
                  ))}
                </button>
              )
            })}
          </nav>
          <div className="flex min-w-0 flex-1 flex-col">
            <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line px-4">
              <span className="truncate font-mono text-xs">{file ?? 'Pick a file to edit it together'}</span>
              {status?.state === 'working' && file && (
                <Mock id="live.agent-editing">
                  <span className="rounded-sm bg-accent px-1.5 text-2xs text-on-accent">{status.agent} is editing</span>
                </Mock>
              )}
              {leader && (
                <span className="ml-auto inline-flex items-center gap-2 rounded-sm bg-info-tint px-2 py-0.5 text-xs">
                  <Avatar name={leader.user} size={16} />
                  Following {leader.user}
                  <Button variant="ghost" size="sm" onClick={() => setFollowing(null)}>
                    Stop
                  </Button>
                </span>
              )}
            </div>
            {file ? (
              <FileEditor
                key={file}
                replica={replica}
                path={file}
                changes={live.changes}
                others={people.filter((person) => person.file === file && person.line !== null).map((person) => [person.line!, person.user])}
                onCursor={(line, column) => setCursor({ line, column })}
                reveal={leader && leader.file === file ? leader.line : null}
                onType={() => setFollowing(null)}
              />
            ) : (
              <div className="flex flex-1 items-center justify-center p-8 text-sm text-fg-2">
                <Icon name="file" className="mr-2" />
                Edits apply live for everyone here; nothing to save.
              </div>
            )}
            <StatusBar label="editor status">
              <span className="inline-flex items-center gap-1">
                <Dot tone="ok" />
                <Mock id="live.sync-devices">
                  in sync with {people.length + 1} {people.length === 0 ? 'device' : 'devices'}
                </Mock>
              </span>
              <span className="max-md:hidden">edits apply live; nothing to save</span>
              {file && (
                <span className="ml-auto">
                  Ln {(leader?.file === file ? leader.line : null) ?? cursor?.line ?? 1}, Col {cursor?.column ?? 1}
                </span>
              )}
            </StatusBar>
          </div>
        </div>
      )}
    </Page>
  )
}
