import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useContext, useEffect, useRef, useState } from 'react'
import { AGENTS, api, short, type CommentView, type HistoryEntry, type ThreadSummary, type TurnInfo } from '../api'
import { useTopic } from '../bus'
import { keys, message } from '../query'
import { MainScroll } from '../scroll'
import { hosted } from '../served'
import { useActions } from '../shortcuts'
import { useThreadState } from '../threadState'
import { Button, Input, toast } from '../ui'
import { CommentThread } from './CommentThread'
import { Diff } from './Diff'
import { DiffPanel } from './DiffPanel'
import { HostedLivePanel, LivePanel } from './Live'
import { ShareDialog } from './ShareDialog'

const KIND_STYLE: Record<string, string> = {
  user: 'font-medium text-fg-1',
  assistant: 'whitespace-pre-wrap text-fg-1',
  thinking: 'text-xs italic text-fg-3',
  tool_call: 'font-mono text-xs text-fg-2',
  tool_result: 'truncate font-mono text-xs text-fg-3',
  merge: 'text-info',
  pull: 'text-info',
}

function Copy({ command }: { command: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <button
      className="rounded border border-line px-2 py-1 font-mono text-xs"
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
  const diff = useQuery({
    queryKey: keys.diff(thread, { turn: turn.number }),
    queryFn: () => api.diff(thread, { turn: turn.number }),
    enabled: open,
  })
  return (
    <div className="my-1">
      <button className="text-xs text-accent-fg" onClick={() => setOpen(!open)}>
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
        className="flex-1 rounded border border-line px-2 py-1 text-sm"
      />
      <button className="rounded bg-accent px-2 py-1 text-xs text-on-accent">Comment</button>
    </form>
  )
}

function Entry({
  entry,
  turn,
  thread,
  comments,
  onComment,
  onChanged,
}: {
  entry: HistoryEntry
  turn: TurnInfo | undefined
  thread: string
  comments: CommentView[]
  onComment: (text: string) => Promise<void>
  onChanged: () => Promise<unknown>
}) {
  const [commenting, setCommenting] = useState(false)
  if (entry.kind === 'compaction' && entry.text.startsWith('(context compacted')) {
    return (
      <div data-testid="compaction" className="my-4 rounded-md bg-accent-tint px-3 py-2 text-xs text-fg-1">
        Context compacted here. Everything above is still part of the history, for you and for the agent.
      </div>
    )
  }
  if (entry.kind === 'compaction') {
    return <p className="ml-10 whitespace-pre-wrap text-xs text-fg-2">Compaction summary: {entry.text}</p>
  }
  return (
    <div data-testid="entry" className={`group ${entry.kind === 'user' ? 'mt-4' : ''}`}>
      <div className="flex gap-2">
        <span className="w-8 shrink-0 text-right font-mono text-xs text-fg-3">{entry.index}</span>
        <div className="min-w-0 flex-1">
          <p className={KIND_STYLE[entry.kind] ?? 'text-sm'}>
            {entry.kind === 'user' ? '› ' : ''}
            {entry.kind === 'tool_call' ? '⚙ ' : ''}
            {entry.text}
          </p>
          {comments.map((comment) => (
            <div key={comment.id} className="ml-2 my-0.5">
              <CommentThread thread={thread} comment={comment} onChanged={onChanged} />
            </div>
          ))}
          {turn && <TurnDiff thread={thread} turn={turn} />}
          {commenting && <CommentBox onSave={(text) => onComment(text).then(() => setCommenting(false))} />}
        </div>
        <button
          aria-label={`comment on entry ${entry.index}`}
          className="invisible text-xs text-fg-3 group-hover:visible"
          onClick={() => setCommenting(true)}
        >
          comment
        </button>
      </div>
    </div>
  )
}

/** Offers pinning, snoozing and settling the thread in the palette. */
function ThreadStateActions({ summary }: { summary: ThreadSummary }) {
  const items = useThreadState(summary)
  useActions(items.map((item) => ({ id: `state:${item.label}`, label: `${item.label} this thread`, run: item.onSelect })))
  return null
}

/** Fork the thread here, or offer a fork's work back to its source as a merge request. */
function ForkActions({ thread, forked }: { thread: string; forked: boolean }) {
  const navigate = useNavigate()
  const [offering, setOffering] = useState(false)
  const [title, setTitle] = useState('')
  const [error, setError] = useState<string>()
  const failed = (failure: unknown) => setError(message(failure))
  const fork = () => {
    setError(undefined)
    api.fork(thread).then((made) => void navigate({ to: '/t/$id', params: { id: made.thread } }), failed)
  }
  useActions([
    { id: 'fork', label: 'Fork this thread', run: fork },
    ...(forked ? [{ id: 'offer', label: 'Offer as merge request', run: () => setOffering(true) }] : []),
  ])
  return (
    <>
      <Button icon="branch" onClick={fork} title="A copy of the thread, in a checkout of its own, to try something else in">
        Fork
      </Button>
      {forked && !offering && (
        <Button icon="merge" onClick={() => setOffering(true)}>
          Offer as merge request
        </Button>
      )}
      {offering && (
        <form
          className="flex items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            setError(undefined)
            api.offerMergeRequest(thread, title).then((mr) => {
              toast(`Offered "${mr.title}"`, 'ok')
              void navigate({ to: '/mr/$id', params: { id: mr.id } })
            }, failed)
          }}
        >
          <Input aria-label="merge request title" autoFocus placeholder="What it does" value={title} onChange={(event) => setTitle(event.target.value)} />
          <Button type="submit" variant="primary" disabled={!title.trim()}>
            Offer
          </Button>
        </form>
      )}
      {error && <p className="w-full text-sm text-danger">{error}</p>}
    </>
  )
}

export function ThreadView({ id }: { id: string }) {
  const queries = useQueryClient()
  const history = useQuery({ queryKey: keys.history(id), queryFn: () => api.history(id) })
  const turns = useQuery({ queryKey: keys.turns(id), queryFn: () => api.turns(id) })
  const comments = useQuery({ queryKey: keys.comments(id), queryFn: () => api.comments(id) })
  const reloadComments = () => queries.invalidateQueries({ queryKey: keys.comments(id) })
  const [agent, setAgent] = useState('')
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const summary = threads.data?.find((thread) => thread.id === history.data?.thread)
  const fullId = history.data?.thread ?? id
  // Looking at the thread marks it seen, again whenever something new happens in it.
  const unseen = summary !== undefined && (summary.mine.seen_ms === null || summary.last_ms > summary.mine.seen_ms)
  useEffect(() => {
    if (unseen) void api.setThreadState(fullId, { pinned: null, snooze: null, settled: null, seen: true }).catch(() => undefined)
  }, [fullId, unseen])
  useTopic('threads')
  useTopic(history.data ? `thread:${history.data.thread}` : null)
  const entries = history.data?.entries ?? []
  const list = useRef<HTMLDivElement>(null)
  const scroll = useContext(MainScroll)
  const rows = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scroll.current,
    estimateSize: () => 28,
    overscan: 20,
    scrollMargin: list.current?.offsetTop ?? 0,
  })

  const turnAt = new Map((turns.data ?? []).filter((turn) => turn.entry !== null).map((turn) => [turn.entry, turn]))
  const entryComments = (index: number) =>
    (comments.data ?? []).filter((comment) => comment.location.at === 'entry' && comment.location.index === index)
  const codeComments = (comments.data ?? []).filter((comment) => comment.location.at !== 'entry')

  return (
    <div className="grid grid-cols-1 gap-6 md:grid-cols-[1fr_18rem]">
      <div>
        <h2 className="text-lg font-semibold">{summary?.title ?? 'Thread'}</h2>
        <p className="font-mono text-xs text-fg-3">{fullId}</p>
        {summary?.forked_from && (
          <p className="text-sm">
            Fork of{' '}
            <Link className="font-mono text-accent-fg" to="/t/$id" params={{ id: summary.forked_from }}>
              {short(summary.forked_from)}
            </Link>
          </p>
        )}
        {summary?.agent && <p className="text-sm text-fg-2">Recorded with {summary.agent}</p>}
        <div className="my-3 flex flex-wrap items-center gap-2">
          <label className="text-sm" htmlFor="continue-agent">
            Continue with
          </label>
          <select
            id="continue-agent"
            aria-label="agent"
            value={agent}
            onChange={(event) => setAgent(event.target.value)}
            className="rounded border border-line px-2 py-1 text-sm"
          >
            <option value="">{summary?.agent ?? 'its agent'}</option>
            {AGENTS.filter((name) => name !== summary?.agent).map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
          <Copy command={`silt resume ${short(fullId)}${agent ? ` --agent ${agent}` : ''}`} />
          <Copy command={`silt fork ${short(fullId)}${agent ? ` --agent ${agent}` : ''}`} />
          <ForkActions thread={fullId} forked={Boolean(summary?.forked_from)} />
          {summary && <ThreadStateActions summary={summary} />}
          {summary && <ShareDialog thread={summary} />}
        </div>
        <DiffPanel thread={id} comments={comments.data ?? []} onCommented={reloadComments} />
        {history.data?.thread && !hosted() && (
          <LivePanel
            thread={history.data.thread}
            lastPrompt={history.data.entries.findLast((entry) => entry.kind === 'user')?.text}
          />
        )}
        {history.data?.thread && hosted() && <HostedLivePanel thread={history.data.thread} />}
        {history.error && <p className="text-danger">{message(history.error)}</p>}
        <div ref={list} className="relative mt-4" style={{ height: rows.getTotalSize() }} data-testid="history">
          {rows.getVirtualItems().map((row) => {
            const entry = entries[row.index]
            return (
              <div
                key={entry.index}
                data-index={row.index}
                ref={rows.measureElement}
                className="absolute left-0 w-full pb-1"
                style={{ transform: `translateY(${row.start - rows.options.scrollMargin}px)` }}
              >
                {entry.origin !== 'own' && entries[row.index - 1]?.origin !== entry.origin && (
                  <h4 className="mt-4 text-xs font-semibold uppercase text-fg-3">
                    {entry.origin === 'inherited' ? 'from the source, before this fork' : 'upstream progress'} (thread{' '}
                    {short(entry.thread)})
                  </h4>
                )}
                <Entry
                  entry={entry}
                  thread={id}
                  turn={turnAt.get(entry.index)}
                  comments={entryComments(entry.index)}
                  onComment={async (text) => {
                    await api.addComment(id, { on: 'entry', index: entry.index }, text)
                    await reloadComments()
                  }}
                  onChanged={reloadComments}
                />
              </div>
            )
          })}
        </div>
      </div>
      <aside className="text-sm">
        <h3 className="font-semibold">Comments on code</h3>
        {comments.error && <p className="text-xs text-danger">{message(comments.error)}</p>}
        <ul data-testid="code-comments" className="mt-2 space-y-2">
          {codeComments.length === 0 && <li className="text-xs text-fg-3">None yet.</li>}
          {codeComments.map((comment) => (
            <li key={comment.id} className={comment.resolved ? 'opacity-50' : ''}>
              <p className="font-mono text-xs">
                {comment.location.at === 'lines'
                  ? `${comment.location.path}:${comment.location.start}${comment.location.end !== comment.location.start ? `-${comment.location.end}` : ''}`
                  : 'unknown place'}
                {comment.location.at === 'lines' && comment.location.outdated && ' (outdated)'}
              </p>
              <CommentThread thread={id} comment={comment} onChanged={reloadComments} />
              {!comment.resolved && (
                <button
                  className="text-xs text-accent-fg"
                  onClick={() => void api.resolveComment(id, comment.id).then(reloadComments)}
                >
                  resolve
                </button>
              )}
            </li>
          ))}
        </ul>
        <p className="mt-3 text-xs text-fg-3">Comment on code from the line numbers in Show changes.</p>
      </aside>
    </div>
  )
}
