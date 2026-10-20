import { useState } from 'react'
import { api, short, type CommentView, type HistoryEntry, type TurnInfo } from '../api'
import { href } from '../router'
import { useLoad } from '../useLoad'
import { Diff } from './Diff'

const KIND_STYLE: Record<string, string> = {
  user: 'font-medium text-neutral-900',
  assistant: 'whitespace-pre-wrap text-neutral-800',
  thinking: 'text-xs italic text-neutral-500',
  tool_call: 'text-xs text-neutral-600',
  tool_result: 'truncate text-xs text-neutral-500',
  merge: 'text-violet-700',
  pull: 'text-violet-700',
}

function Copy({ command }: { command: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <button
      className="rounded border border-neutral-300 px-2 py-1 font-mono text-xs"
      onClick={() => {
        void navigator.clipboard?.writeText(command)
        setCopied(true)
      }}
      title="Copy, then run it in a clone of the repository"
    >
      {copied ? 'copied' : command}
    </button>
  )
}

function TurnDiff({ thread, turn }: { thread: string; turn: TurnInfo }) {
  const [open, setOpen] = useState(false)
  const diff = useLoad(() => (open ? api.diff(thread, turn.number) : Promise.resolve('')), `${thread}:${turn.number}:${open}`)
  return (
    <div className="my-1">
      <button className="text-xs text-sky-700" onClick={() => setOpen(!open)}>
        {open ? 'hide' : 'show'} what turn {turn.number} changed
      </button>
      {open && diff.data !== undefined && <Diff text={diff.data} />}
    </div>
  )
}

function CommentBox({ onSave }: { onSave: (text: string) => Promise<void> }) {
  const [text, setText] = useState('')
  return (
    <form
      className="my-1 flex gap-2"
      onSubmit={(event) => {
        event.preventDefault()
        void onSave(text).then(() => setText(''))
      }}
    >
      <input
        aria-label="comment"
        autoFocus
        value={text}
        onChange={(event) => setText(event.target.value)}
        className="flex-1 rounded border border-neutral-300 px-2 py-1 text-sm"
      />
      <button className="rounded bg-neutral-900 px-2 py-1 text-xs text-white">Comment</button>
    </form>
  )
}

function Entry({
  entry,
  turn,
  thread,
  comments,
  onComment,
}: {
  entry: HistoryEntry
  turn: TurnInfo | undefined
  thread: string
  comments: CommentView[]
  onComment: (text: string) => Promise<void>
}) {
  const [commenting, setCommenting] = useState(false)
  if (entry.kind === 'compaction' && entry.text.startsWith('(context compacted')) {
    return (
      <div data-testid="compaction" className="my-4 border-y border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-800">
        Context compacted here. Everything above is still part of the history, for you and for the agent.
      </div>
    )
  }
  if (entry.kind === 'compaction') {
    return <p className="ml-10 whitespace-pre-wrap text-xs text-amber-800">Compaction summary: {entry.text}</p>
  }
  return (
    <div data-testid="entry" className={`group ${entry.kind === 'user' ? 'mt-4' : ''}`}>
      <div className="flex gap-2">
        <span className="w-8 shrink-0 text-right font-mono text-xs text-neutral-400">{entry.index}</span>
        <div className="min-w-0 flex-1">
          <p className={KIND_STYLE[entry.kind] ?? 'text-sm'}>
            {entry.kind === 'user' ? '› ' : ''}
            {entry.kind === 'tool_call' ? '⚙ ' : ''}
            {entry.text}
          </p>
          {comments.map((comment) => (
            <p key={comment.id} className="ml-2 border-l-2 border-sky-300 pl-2 text-xs text-sky-900">
              {comment.by}: {comment.text} {comment.resolved && '(resolved)'}
            </p>
          ))}
          {turn && <TurnDiff thread={thread} turn={turn} />}
          {commenting && <CommentBox onSave={(text) => onComment(text).then(() => setCommenting(false))} />}
        </div>
        <button
          aria-label={`comment on entry ${entry.index}`}
          className="invisible text-xs text-neutral-400 group-hover:visible"
          onClick={() => setCommenting(true)}
        >
          comment
        </button>
      </div>
    </div>
  )
}

function CodeComment({ onSave }: { onSave: (path: string, start: number, end: number, text: string) => Promise<void> }) {
  const [path, setPath] = useState('')
  const [lines, setLines] = useState('')
  const [text, setText] = useState('')
  const [error, setError] = useState<string>()
  return (
    <form
      className="mt-3 space-y-1 text-xs"
      onSubmit={(event) => {
        event.preventDefault()
        const [start, end = start] = lines.split('-').map((part) => Number(part.trim()))
        setError(undefined)
        onSave(path, start, end, text).then(
          () => setText(''),
          (failure: unknown) => setError(failure instanceof Error ? failure.message : String(failure)),
        )
      }}
    >
      <p className="font-semibold text-neutral-600">Comment on code</p>
      <input aria-label="file" placeholder="src/main.rs" value={path} onChange={(event) => setPath(event.target.value)} className="w-full rounded border border-neutral-300 px-2 py-1" />
      <input aria-label="lines" placeholder="10-14" value={lines} onChange={(event) => setLines(event.target.value)} className="w-full rounded border border-neutral-300 px-2 py-1" />
      <input aria-label="code comment" placeholder="comment" value={text} onChange={(event) => setText(event.target.value)} className="w-full rounded border border-neutral-300 px-2 py-1" />
      <button className="rounded bg-neutral-900 px-2 py-1 text-white">Add</button>
      {error && <p className="text-rose-700">{error}</p>}
    </form>
  )
}

export function ThreadView({ id }: { id: string }) {
  const history = useLoad(() => api.history(id), `history:${id}`)
  const turns = useLoad(() => api.turns(id), `turns:${id}`)
  const comments = useLoad(() => api.comments(id), `comments:${id}`)
  const [showDiff, setShowDiff] = useState(false)
  const diff = useLoad(() => (showDiff ? api.diff(id) : Promise.resolve('')), `diff:${id}:${showDiff}`)
  const threads = useLoad(api.threads, 'threads')
  const summary = threads.data?.find((thread) => thread.id === history.data?.thread)
  const fullId = history.data?.thread ?? id

  const turnAt = new Map((turns.data ?? []).filter((turn) => turn.entry !== null).map((turn) => [turn.entry, turn]))
  const entryComments = (index: number) =>
    (comments.data ?? []).filter((comment) => comment.location.at === 'entry' && comment.location.index === index)
  const codeComments = (comments.data ?? []).filter((comment) => comment.location.at !== 'entry')

  return (
    <div className="grid grid-cols-[1fr_18rem] gap-6">
      <div>
        <h2 className="text-lg font-semibold">{summary?.title ?? 'Thread'}</h2>
        <p className="font-mono text-xs text-neutral-500">{fullId}</p>
        {summary?.forked_from && (
          <p className="text-sm">
            Fork of{' '}
            <a className="font-mono text-sky-700" href={href({ page: 'thread', id: summary.forked_from })}>
              {short(summary.forked_from)}
            </a>
          </p>
        )}
        <div className="my-3 flex flex-wrap gap-2">
          <Copy command={`silt resume ${short(fullId)}`} />
          <Copy command={`silt fork ${short(fullId)}`} />
          <button className="text-sm text-sky-700" onClick={() => setShowDiff(!showDiff)}>
            {showDiff ? 'Hide' : 'Show'} all changes
          </button>
        </div>
        {showDiff && diff.data !== undefined && <Diff text={diff.data} />}
        {history.error && <p className="text-rose-700">{history.error}</p>}
        <div className="mt-4 space-y-1">
          {(history.data?.entries ?? []).map((entry, index, entries) => (
            <div key={entry.index}>
              {entry.origin !== 'own' && entries[index - 1]?.origin !== entry.origin && (
                <h4 className="mt-4 text-xs font-semibold uppercase text-neutral-500">
                  {entry.origin === 'inherited' ? 'from the source, before this fork' : 'upstream progress'} (thread {short(entry.thread)})
                </h4>
              )}
              <Entry
                entry={entry}
                thread={id}
                turn={turnAt.get(entry.index)}
                comments={entryComments(entry.index)}
                onComment={async (text) => {
                  await api.addComment(id, { on: 'entry', index: entry.index }, text)
                  comments.reload()
                }}
              />
            </div>
          ))}
        </div>
      </div>
      <aside className="text-sm">
        <h3 className="font-semibold">Comments on code</h3>
        {comments.error && <p className="text-xs text-rose-700">{comments.error}</p>}
        <ul data-testid="code-comments" className="mt-2 space-y-2">
          {codeComments.length === 0 && <li className="text-xs text-neutral-500">None yet.</li>}
          {codeComments.map((comment) => (
            <li key={comment.id} className={comment.resolved ? 'opacity-50' : ''}>
              <p className="font-mono text-xs">
                {comment.location.at === 'lines'
                  ? `${comment.location.path}:${comment.location.start}${comment.location.end !== comment.location.start ? `-${comment.location.end}` : ''}`
                  : 'unknown place'}
                {comment.location.at === 'lines' && comment.location.outdated && ' (outdated)'}
              </p>
              <p>
                {comment.by}: {comment.text}
              </p>
              {!comment.resolved && (
                <button
                  className="text-xs text-sky-700"
                  onClick={() => void api.resolveComment(id, comment.id).then(comments.reload)}
                >
                  resolve
                </button>
              )}
            </li>
          ))}
        </ul>
        <CodeComment
          onSave={async (path, start, end, text) => {
            await api.addComment(id, { on: 'lines', path, start, end }, text)
            comments.reload()
          }}
        />
      </aside>
    </div>
  )
}
