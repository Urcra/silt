import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useRouterState } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import { type ReactNode, useCallback, useContext, useEffect, useRef, useState } from 'react'
import { AGENTS, api, short, type CommentView, type HistoryEntry, type ThreadSummary, type TurnInfo } from '../api'
import { CloudSession } from './Cloud'
import { useTopic } from '../bus'
import { parseFiles } from '../diffParse'
import { appHref, filePath } from '../permalink'
import { projectName } from '../projects'
import { keys, message } from '../query'
import { useNow } from '../now'
import { MainScroll } from '../scroll'
import { hosted } from '../served'
import { useClearSentDraft, useLive, usePresence } from '../session'
import { useActions } from '../shortcuts'
import { threadTone, threadWhere } from '../threadLook'
import { useThreadState } from '../threadState'
import { ago } from '../time'
import { type Row, duration, piecesOf, rowsOf, toolOf, toolSummary } from '../transcript'
import {
  Avatar,
  Button,
  Counts,
  Dot,
  Icon,
  Input,
  KeyValue,
  Popover,
  Skeleton,
  Tabs,
  Tag,
  buttonClass,
  cx,
  toast,
} from '../ui'
import { AgentDock, AgentTail, PermissionCard, PlanCard } from './Agent'
import { CommentThread } from './CommentThread'
import { CopyLink } from './CopyLink'
import { DiffPanel } from './DiffPanel'
import { DiffView } from './DiffView'
import { Markdown } from './Markdown'
import { Mock } from './Mock'
import { Page } from './Page'
import { ShareDialog } from './ShareDialog'

/** The commands that pick the thread up in a terminal, each copied with a click. */
function CopyCommands({ commands }: { commands: string[] }) {
  return (
    <Popover
      label="commands"
      triggerClassName={buttonClass('secondary')}
      trigger={
        <>
          <Icon name="terminal" />
          Copy command
          <Icon name="chevronDown" className="h-3 w-3" />
        </>
      }
      panelClassName="flex w-80 flex-col gap-1 p-2"
    >
      {(close) => (
        <>
          <p className="px-1 pb-1 text-2xs text-fg-2">Copy, then run it in a clone of the repository.</p>
          {commands.map((command) => (
            <button
              key={command}
              type="button"
              className={cx(buttonClass('ghost'), 'justify-start font-mono font-normal text-fg-1')}
              onClick={() => {
                void navigator.clipboard?.writeText(command).catch(() => undefined)
                toast(`Copied: ${command}`, 'ok')
                close()
              }}
            >
              {command}
            </button>
          ))}
        </>
      )}
    </Popover>
  )
}

/** What a permalink's `#turn-N` or `#comment-<id>` points at on the thread page (ADR 0031). */
type Anchor = { entry?: number; comment?: string }

/** What `hash` names, once what it names has loaded. */
function anchorOf(hash: string, turns: TurnInfo[] | undefined, comments: CommentView[] | undefined): Anchor | undefined {
  const turn = /^turn-(\d+)$/.exec(hash)
  if (turn) {
    const found = turns?.find((candidate) => candidate.number === Number(turn[1]))
    return found && { entry: found.entry ?? undefined }
  }
  const commentId = /^comment-(.+)$/.exec(hash)?.[1]
  const comment = commentId === undefined ? undefined : comments?.find((candidate) => candidate.id === commentId)
  if (!comment) return undefined
  return { comment: comment.id, entry: comment.location.at === 'entry' ? comment.location.index : undefined }
}

/** Follows the page's hash once what it names has loaded, scrolling there with `scrollTo` (a row of `rows`).
 * `complete` says every page of the history has: until then, an entry not among `rows` may still come. */
function useAnchor(
  rows: Row[] | undefined,
  complete: boolean,
  turns: TurnInfo[] | undefined,
  comments: CommentView[] | undefined,
  scrollTo: (row: number) => void,
): Anchor {
  const hash = useRouterState({ select: (state) => state.location.hash }).replace(/^#/, '')
  const anchor = anchorOf(hash, turns, comments)
  const followed = useRef('')
  useEffect(() => {
    if (!anchor || !rows || followed.current === hash) return
    const holds = (row: Row, index: number) =>
      row.kind === 'turn' && (row.prompt?.index === index || row.entries.some((entry) => entry.index === index))
    const at = anchor.entry === undefined ? -1 : rows.findIndex((row) => holds(row, anchor.entry!))
    if (at < 0 && anchor.entry !== undefined && !complete) return
    followed.current = hash
    if (at >= 0) scrollTo(at)
    else if (anchor.comment) document.getElementById(`comment-${anchor.comment}`)?.scrollIntoView({ block: 'center' })
  }, [hash, anchor, rows, complete, scrollTo])
  return anchor ?? {}
}

function CommentBox({ onSave, onCancel }: { onSave: (text: string) => Promise<void>; onCancel: () => void }) {
  const [text, setText] = useState('')
  return (
    <form
      className="flex gap-2"
      onSubmit={(event) => {
        event.preventDefault()
        void onSave(text).then(() => setText(''))
      }}
    >
      <Input aria-label="comment" autoFocus value={text} onChange={(event) => setText(event.target.value)} className="flex-1" />
      <Button type="submit" variant="primary" disabled={!text.trim()}>
        Comment
      </Button>
      <Button variant="ghost" onClick={onCancel}>
        Cancel
      </Button>
    </form>
  )
}

interface EntryProps {
  thread: string
  comments: CommentView[]
  anchor: Anchor
  onComment: (index: number, text: string) => Promise<void>
  onChanged: () => Promise<unknown>
}

/** One entry of the history, with its comments and a way to comment on it. */
function Entry({ entry, children, thread, comments, anchor, onComment, onChanged }: EntryProps & { entry: HistoryEntry; children: ReactNode }) {
  const [commenting, setCommenting] = useState(false)
  const here = comments.filter((comment) => comment.location.at === 'entry' && comment.location.index === entry.index)
  const highlighted = anchor.entry === entry.index
  return (
    <div
      data-testid="entry"
      data-rewound={entry.rewound || undefined}
      data-highlighted={highlighted || undefined}
      className={cx('group/entry relative', entry.rewound && 'opacity-50', highlighted && 'rounded-sm ring-2 ring-accent ring-offset-4 ring-offset-bg')}
      title={entry.rewound ? 'Rewound by a revert: kept in the history, no longer part of the conversation' : undefined}
    >
      {entry.rewound && entry.kind === 'user' && <p className="text-xs text-fg-2 italic">rewound by a revert</p>}
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">{children}</div>
        <button
          type="button"
          aria-label={`comment on entry ${entry.index}`}
          className="invisible shrink-0 rounded-sm p-0.5 text-fg-2 group-hover/entry:visible hover:bg-surface-2 hover:text-fg-1 focus:visible"
          onClick={() => setCommenting(true)}
        >
          <Icon name="comment" className="h-3.5 w-3.5" />
        </button>
      </div>
      {here.length > 0 && (
        <div className="mt-1.5 flex flex-col gap-2 rounded-md bg-accent-tint px-3 py-2.5">
          {here.map((comment) => (
            <div key={comment.id} id={`comment-${comment.id}`}>
              <CommentThread thread={thread} comment={comment} onChanged={onChanged} highlighted={anchor.comment === comment.id} />
            </div>
          ))}
        </div>
      )}
      {commenting && (
        <div className="mt-1.5">
          <CommentBox onCancel={() => setCommenting(false)} onSave={(text) => onComment(entry.index, text).then(() => setCommenting(false))} />
        </div>
      )}
    </div>
  )
}

/** What one entry says: a prompt, the agent's words as Markdown, or an event. */
function EntryText({ entry }: { entry: HistoryEntry }) {
  if (entry.kind === 'user') return <p className="text-sm font-medium whitespace-pre-wrap">› {entry.text}</p>
  if (entry.kind === 'assistant') return <Markdown text={entry.text} />
  if (entry.kind === 'merge' || entry.kind === 'pull')
    return (
      <p className="flex items-center gap-1.5 text-xs text-info">
        <Icon name="merge" className="h-3.5 w-3.5" />
        {entry.text}
      </p>
    )
  if (entry.kind === 'thinking') return <p className="text-xs text-fg-2 italic whitespace-pre-wrap">{entry.text}</p>
  if (entry.kind === 'tool_call') {
    const call = toolOf(entry.text)
    return (
      <p className="font-mono text-xs break-all text-fg-1" title={entry.text}>
        <span className="font-medium">{call.name}</span>
        {call.file ? <span className="text-fg-2"> {call.file}</span> : <span className="text-fg-2"> {entry.text.slice(call.name.length + 1, 160)}</span>}
      </p>
    )
  }
  return <p className="line-clamp-4 font-mono text-xs break-all whitespace-pre-wrap text-fg-2">{entry.text}</p>
}

/** A run of the agent's tool work, folded under what it amounted to. */
function ToolFold({ entries, ...props }: EntryProps & { entries: HistoryEntry[] }) {
  return (
    <details className="group/tools rounded-sm text-xs text-fg-2">
      <summary className="flex cursor-pointer list-none items-center gap-1.5 py-0.5 hover:text-fg-1 max-md:min-h-11">
        <Icon name="chevronRight" className="h-3 w-3 transition-transform group-open/tools:rotate-90" />
        <Icon name="terminal" className="h-3.5 w-3.5" />
        {toolSummary(entries)}
      </summary>
      <div className="mt-1 ml-[18px] flex flex-col gap-1.5 border-l border-line pl-3">
        {entries.map((entry) => (
          <Entry key={entry.index} entry={entry} {...props}>
            <EntryText entry={entry} />
          </Entry>
        ))}
      </div>
    </details>
  )
}

/** Under a turn: what it did, in numbers, and what can be done from it (ADR 0029, 0031). */
function TurnBar({ thread, turn, row, rewound }: { thread: string; turn: TurnInfo; row: Extract<Row, { kind: 'turn' }>; rewound: boolean }) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  const [reverting, setReverting] = useState(false)
  const diff = useQuery({
    queryKey: keys.diff(thread, { turn: turn.number }),
    queryFn: () => api.diff(thread, { turn: turn.number }),
    staleTime: Infinity,
  })
  const files = diff.data === undefined ? null : parseFiles(diff.data)
  const tools = row.entries.filter((entry) => entry.kind === 'tool_call').length
  const last = row.entries[row.entries.length - 1]
  const took = row.prompt && last ? duration(row.prompt.time_ms, last.time_ms) : null
  const facts = [
    `turn ${turn.number}`,
    tools > 0 && `${tools} tool ${tools === 1 ? 'call' : 'calls'}`,
    files && files.length > 0 && `${files.length} ${files.length === 1 ? 'file' : 'files'}`,
  ].filter(Boolean)
  const forkHere = () =>
    api.fork(thread, turn.number).then(
      (made) => void navigate({ to: '/t/$id', params: { id: made.thread } }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const revertHere = () =>
    api.revert(thread, turn.number).then(
      (reverted) => {
        setReverting(false)
        toast(`Reverted to turn ${reverted.turn}: ${reverted.files.length} files written back`)
        void queryClient.invalidateQueries()
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const ghost = cx(buttonClass('ghost', 'sm'), 'font-normal')
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-fg-2">
        <span className="font-mono text-2xs">
          {facts.join(' · ')}
          {files && files.length > 0 && (
            <>
              {' · '}
              <Counts added={files.reduce((sum, file) => sum + file.added, 0)} removed={files.reduce((sum, file) => sum + file.removed, 0)} />
            </>
          )}
          {took && <> · {took}</>}
        </span>
        {files && files.length > 0 && (
          <button
            type="button"
            aria-label={`${open ? 'hide' : 'show'} what turn ${turn.number} changed`}
            className="text-accent-fg hover:text-fg-1 max-md:min-h-11"
            onClick={() => setOpen(!open)}
          >
            {open ? 'Hide what this turn changed' : 'What this turn changed'}
          </button>
        )}
        <span className="ml-auto flex flex-wrap gap-0.5">
          {!hosted() &&
            !rewound &&
            (reverting ? (
              <span className="flex items-center gap-2">
                Files go back to how turn {turn.number} left them; later turns stay, marked rewound.
                <Button variant="danger" size="sm" onClick={() => void revertHere()}>
                  Revert
                </Button>
                <Button variant="ghost" size="sm" onClick={() => setReverting(false)}>
                  Cancel
                </Button>
              </span>
            ) : (
              <button type="button" aria-label={`revert to turn ${turn.number}`} className={ghost} onClick={() => setReverting(true)} title="Take this thread's code and conversation back to this turn">
                <Icon name="revert" className="h-3.5 w-3.5" />
                Revert to here
              </button>
            ))}
          {!hosted() && (
            <button type="button" aria-label={`fork from turn ${turn.number}`} className={ghost} onClick={() => void forkHere()} title="A fork with the code and conversation as this turn left them">
              <Icon name="fork" className="h-3.5 w-3.5" />
              Fork from here
            </button>
          )}
          <CopyLink path={`/t/${thread}#turn-${turn.number}`} thread={thread} label="Copy link" name={`link to turn ${turn.number}`} className={ghost} />
        </span>
      </div>
      {open && diff.data !== undefined && <DiffView text={diff.data} viewFiles={thread} />}
    </div>
  )
}

function TranscriptRow({ row, ...props }: EntryProps & { row: Row }) {
  const [summary, setSummary] = useState(false)
  if (row.kind === 'origin') {
    return (
      <div className="flex items-center gap-2.5 py-2 text-xs text-fg-2">
        <span className="h-px flex-1 bg-line" />
        <span className="font-mono text-2xs tracking-[0.04em] uppercase">
          {row.origin === 'inherited' ? 'From ' : 'Upstream progress from '}
          <span data-volatile>{short(row.thread)}</span>
          {row.origin === 'inherited' && ', before this fork'}
        </span>
        <span className="h-px flex-1 bg-line" />
      </div>
    )
  }
  if (row.kind === 'compaction') {
    return (
      <div className="py-2">
        <div role="separator" data-testid={row.marker ? 'compaction' : undefined} className="flex flex-wrap items-center gap-2.5 rounded-md border border-dashed border-line-strong bg-surface-1 px-3 py-2">
          <span className="text-xs">
            {row.marker ? 'Context compacted here. Everything above stays in the history, for you and for the agent, and in search.' : 'The agent summarized its context here.'}
          </span>
          {row.summary && (
            <Button variant="ghost" size="sm" className="ml-auto" aria-expanded={summary} onClick={() => setSummary(!summary)}>
              {summary ? 'Hide summary' : 'Show summary'}
            </Button>
          )}
          {summary && row.summary && <p className="w-full text-xs whitespace-pre-wrap text-fg-2">Compaction summary: {row.summary.text}</p>}
        </div>
      </div>
    )
  }
  const pieces = piecesOf(row.entries)
  const rewound = row.prompt?.rewound ?? row.entries.some((entry) => entry.rewound)
  return (
    <div id={row.turn ? `turn-${row.turn.number}` : undefined} className="grid grid-cols-[32px_minmax(0,1fr)] gap-3 py-2.5 md:grid-cols-[40px_minmax(0,1fr)]">
      <span className="pt-0.5 text-right font-mono text-2xs text-fg-2">{row.prompt ? `#${row.prompt.index}` : ''}</span>
      <div className="flex min-w-0 flex-col gap-2">
        {row.prompt && (
          <Entry entry={row.prompt} {...props}>
            <EntryText entry={row.prompt} />
          </Entry>
        )}
        {pieces.map((piece) =>
          piece.kind === 'tools' ? (
            <ToolFold key={piece.entries[0].index} entries={piece.entries} {...props} />
          ) : (
            <Entry key={piece.entry.index} entry={piece.entry} {...props}>
              <EntryText entry={piece.entry} />
            </Entry>
          ),
        )}
        {row.turn && <TurnBar thread={props.thread} turn={row.turn} row={row} rewound={rewound} />}
      </div>
    </div>
  )
}

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
      <Button icon="fork" onClick={fork} title="A copy of the thread, in a checkout of its own, to try something else in">
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

/** The thread's title, where it is, what it is, and what can be done with it (the Thread board's header). */
function ThreadHeader({
  thread,
  summary,
  here,
  changes,
  showingChanges,
  onChanges,
  onDetails,
}: {
  thread: string
  summary: ThreadSummary | undefined
  here: number
  changes: { added: number; removed: number } | null
  showingChanges: boolean
  onChanges: () => void
  /** Opens the inspector on a phone, where it isn't beside the conversation. */
  onDetails: () => void
}) {
  const [agent, setAgent] = useState('')
  const where = summary ? threadWhere(summary) : null
  const meta = summary
    ? [
        projectName(summary.repo || 'no remote'),
        summary.owner && `started by ${summary.owner}`,
        `${summary.messages} ${summary.messages === 1 ? 'message' : 'messages'}`,
        `${summary.edits} ${summary.edits === 1 ? 'edit' : 'edits'}`,
      ].filter(Boolean)
    : []
  return (
    <header className="flex flex-col gap-2.5 border-b border-line px-4 pt-5 pb-4 md:px-8">
      <div className="flex flex-wrap items-center gap-2.5">
        <h1 className="text-xl font-medium">{summary?.title ?? 'Thread'}</h1>
        {where && (
          <span className="inline-flex items-center gap-1.5 rounded-sm border border-line px-2 py-0.5 text-xs">
            <Dot tone={summary?.agent_state === 'working' ? 'ok' : where.tone} />
            {where.label}
          </span>
        )}
        {summary?.local_only && (
          <span title="Its project keeps new threads off every server; choose a server on the Projects page to move it">
            <Tag>This machine only</Tag>
          </span>
        )}
      </div>
      <p className="font-mono text-2xs text-fg-2">
        <span data-volatile>{short(thread)}</span>
        {meta.map((part) => ` · ${part}`)}
        {summary?.agent && (
          <>
            {' · '}
            <span>Recorded with {summary.agent}</span>
          </>
        )}
      </p>
      {summary?.forked_from && (
        <p className="text-xs text-fg-2">
          Fork of{' '}
          <Link className="font-mono text-accent-fg" to="/t/$id" params={{ id: summary.forked_from }} data-volatile>
            {short(summary.forked_from)}
          </Link>
        </p>
      )}
      {summary && <CloudSession thread={summary} />}
      <div className="mt-1 flex flex-wrap items-center gap-2">
        {/* A terminal's commands mean nothing on a phone. */}
        <span className="flex items-center gap-2 max-md:hidden">
          <label className="text-xs text-fg-2" htmlFor="continue-agent">
            Continue with
          </label>
          <select
            id="continue-agent"
            aria-label="agent"
            value={agent}
            onChange={(event) => setAgent(event.target.value)}
            className="rounded-sm border border-line bg-elevated px-2 py-1 font-mono text-xs text-fg-1"
          >
            <option value="">{summary?.agent ?? 'its agent'}</option>
            {AGENTS.filter((name) => name !== summary?.agent).map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
          <CopyCommands
            commands={[
              `silt resume ${short(thread)}${agent ? ` --agent ${agent}` : ''}`,
              `silt fork ${short(thread)}${agent ? ` --agent ${agent}` : ''}`,
            ]}
          />
        </span>
        <ForkActions thread={thread} forked={Boolean(summary?.forked_from)} />
        <span className="flex flex-wrap items-center gap-2 lg:ml-auto">
          {!hosted() && (
            <Link to="/t/$id/live" params={{ id: thread }} className={cx(buttonClass('ghost'), 'max-md:hidden')}>
              <Dot tone={here > 0 ? 'ok' : 'idle'} />
              Join live{here > 0 && ` · ${here} here`}
            </Link>
          )}
          {summary && <ShareDialog thread={summary} />}
          <Button
            aria-label="Changes"
            aria-pressed={showingChanges}
            onClick={onChanges}
            title="What the thread changed, beside the conversation (D)"
            className="aria-pressed:border-line-strong aria-pressed:bg-surface-3"
          >
            Changes
            {changes && <Counts added={changes.added} removed={changes.removed} />}
          </Button>
          <Button className="lg:hidden" icon="info" onClick={onDetails}>
            Details
          </Button>
        </span>
      </div>
      {summary && <ThreadStateActions summary={summary} />}
    </header>
  )
}

/** The thread's lineage: what it was forked from, and its forks. */
function Lineage({ summary, threads }: { summary: ThreadSummary; threads: ThreadSummary[] }) {
  const mrs = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests })
  const above: ThreadSummary[] = []
  for (let source = summary.forked_from; source; ) {
    const found = threads.find((thread) => thread.id === source)
    if (!found || above.includes(found)) break
    above.unshift(found)
    source = found.forked_from
  }
  const forks = threads.filter((thread) => thread.forked_from === summary.id)
  const openFrom = (fork: string) => (mrs.data ?? []).find((mr) => mr.fork === fork && mr.status === 'open')
  const row = (thread: ThreadSummary, depth: number, note: ReactNode, current = false) => (
    <li
      key={thread.id}
      style={{ paddingLeft: depth * 16 }}
      className={cx('flex min-w-0 items-center gap-2 rounded-sm py-1 text-xs', current && 'bg-accent-tint pr-1.5')}
    >
      <Dot tone={current ? 'accent' : threadTone(thread)} className={current ? 'ml-1.5' : undefined} />
      {current ? (
        <span className="font-mono" data-volatile>
          {short(thread.id)}
        </span>
      ) : (
        <Link to="/t/$id" params={{ id: thread.id }} className="inline-flex items-center font-mono text-accent-fg max-md:min-h-11" data-volatile>
          {short(thread.id)}
        </Link>
      )}
      <span className="min-w-0 truncate text-fg-2">{note}</span>
    </li>
  )
  return (
    <ul className="flex flex-col gap-0.5">
      {above.map((thread, depth) => row(thread, depth, thread.title ?? 'Untitled thread'))}
      {row(summary, above.length, 'this thread', true)}
      {forks.map((fork) => {
        const mr = openFrom(fork.id)
        return row(
          fork,
          above.length + 1,
          mr ? (
            <Link to="/mr/$id" params={{ id: mr.id }} className="text-accent-fg">
              {fork.owner || 'fork'} · MR open
            </Link>
          ) : (
            [fork.owner, fork.title ?? 'Untitled thread'].filter(Boolean).join(' · ')
          ),
        )
      })}
    </ul>
  )
}

function InspectorSection({ title, label, aside, children }: { title: string; label?: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section aria-label={label ?? title.toLowerCase()} className="flex flex-col gap-2">
      <h2 className="flex items-center font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">
        {title}
        {aside && <span className="ml-auto font-sans text-xs tracking-normal normal-case">{aside}</span>}
      </h2>
      {children}
    </section>
  )
}

/** The commits the thread made, each with the turn it came in (ADR 0030). */
function ThreadCommits({ thread }: { thread: string }) {
  const commits = useQuery({ queryKey: keys.commits(thread), queryFn: () => api.commits(thread), retry: false })
  if (!commits.data?.length) return null
  return (
    <InspectorSection title="Commits from this thread" label="commits">
      <ul className="flex flex-col gap-1.5 text-xs">
        {commits.data.map((commit) => (
          <li key={commit.commit} className="flex flex-col gap-0.5">
            <span className="min-w-0 truncate">
              <code className="mr-1.5 font-mono text-fg-2" data-volatile>
                {commit.commit.slice(0, 7)}
              </code>
              {commit.subject ?? ''}
            </span>
            {commit.turn !== null && (
              <Link className="text-2xs text-accent-fg" to="/t/$id" params={{ id: thread }} hash={`turn-${commit.turn}`}>
                made in turn {commit.turn}
              </Link>
            )}
          </li>
        ))}
      </ul>
    </InspectorSection>
  )
}

/** The Thread board's inspector: handoff, lineage, files, commits and comments on code. */
function Details({
  thread,
  summary,
  threads,
  total,
  diff,
  codeComments,
  anchor,
  onChanged,
  onCommentLines,
}: {
  thread: string
  summary: ThreadSummary | undefined
  threads: ThreadSummary[]
  total: number
  diff: string | undefined
  codeComments: CommentView[]
  anchor: Anchor
  onChanged: () => Promise<unknown>
  onCommentLines: () => void
}) {
  const now = useNow()
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const device = status.data?.servers?.[0]?.name ?? 'this machine'
  const files = diff === undefined ? null : parseFiles(diff)
  const liveOn =
    summary?.location === 'moved' ? 'another device' : summary?.location === 'remote' ? 'the server only' : hosted() ? 'its machine' : device
  return (
    <div className="flex flex-col gap-6 px-5 py-5">
      <InspectorSection title="Handoff">
        <KeyValue
          rows={[
            ['Live on', liveOn],
            [
              'Before',
              <Mock key="before" id="thread.handoff-before">
                laptop · 2h ago
              </Mock>,
            ],
            [
              'Synced',
              <Mock key="synced" id="thread.synced">
                all {total} entries
              </Mock>,
            ],
            ...(summary ? ([['Last moved', <span key="moved" data-volatile>{ago(summary.last_ms, now)}</span>]] as [ReactNode, ReactNode][]) : []),
          ]}
        />
        <p className="text-xs leading-normal text-fg-2">Picking it up on another machine moves the agent session there. This one stops writing.</p>
      </InspectorSection>
      {summary && (
        <InspectorSection
          title="Lineage"
          aside={
            <Link to="/graph" className="text-accent-fg hover:text-fg-1">
              View graph
            </Link>
          }
        >
          <Lineage summary={summary} threads={threads} />
        </InspectorSection>
      )}
      <InspectorSection title="Files changed">
        {files === null && <Skeleton className="h-10" />}
        {files?.length === 0 && <p className="text-xs text-fg-2">No changes yet.</p>}
        <ul className="flex flex-col gap-0.5 font-mono text-2xs">
          {files?.map((file) => (
            <li key={file.path} className="flex items-center gap-2 py-0.5">
              <a
                href={appHref(filePath(thread, file.path))}
                className="min-w-0 flex-1 truncate text-fg-1 hover:text-accent-fg max-md:inline-flex max-md:min-h-11 max-md:items-center"
                title={file.path}
              >
                {file.path}
              </a>
              <Counts added={file.added} removed={file.removed} />
            </li>
          ))}
        </ul>
      </InspectorSection>
      <ThreadCommits thread={thread} />
      <InspectorSection title="Comments on code">
        <ul data-testid="code-comments" className="flex flex-col gap-2">
          {codeComments.length === 0 && <li className="text-xs text-fg-2">None yet.</li>}
          {codeComments.map((comment) => (
            <li
              key={comment.id}
              id={`comment-${comment.id}`}
              className={cx('flex flex-col gap-1 rounded-md border border-line bg-elevated px-2.5 py-2', comment.resolved && 'opacity-60')}
            >
              <p className="font-mono text-2xs text-fg-2">
                {comment.location.at === 'lines' ? (
                  <a
                    className="text-accent-fg"
                    href={appHref(filePath(thread, comment.location.path, undefined, comment.location))}
                    title="The file as the thread has it now"
                  >
                    {comment.location.path}:{comment.location.start}
                    {comment.location.end !== comment.location.start ? `–${comment.location.end}` : ''}
                  </a>
                ) : (
                  'unknown place'
                )}
                {comment.location.at === 'lines' && comment.location.outdated && ' (outdated)'}
              </p>
              <CommentThread thread={thread} comment={comment} onChanged={onChanged} highlighted={anchor.comment === comment.id} />
              {!comment.resolved && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="self-start"
                  aria-label="resolve"
                  onClick={() => void api.resolveComment(thread, comment.id).then(onChanged)}
                >
                  Resolve
                </Button>
              )}
            </li>
          ))}
        </ul>
        <Button className="self-start" icon="comment" onClick={onCommentLines}>
          Comment on lines
        </Button>
      </InspectorSection>
    </div>
  )
}

/** People here, as avatars, for the header's Join live. */
function Here({ people }: { people: { user: string; device: string }[] }) {
  if (people.length === 0) return null
  return (
    <span className="flex -space-x-1.5" aria-label={`${people.length} here`}>
      {people.slice(0, 4).map((person) => (
        <span key={person.device} className="rounded-full ring-2 ring-bg">
          <Avatar name={person.user} size={20} />
        </span>
      ))}
    </span>
  )
}

type InspectorTab = 'details' | 'changes'

function ThreadBody({
  thread,
  id,
  entries,
  total,
}: {
  thread: string
  /** The id in the address, maybe short. */
  id: string
  entries: HistoryEntry[]
  total: number
}) {
  const queries = useQueryClient()
  const turns = useQuery({ queryKey: keys.turns(id), queryFn: () => api.turns(id) })
  const comments = useQuery({ queryKey: keys.comments(id), queryFn: () => api.comments(id) })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const diff = useQuery({ queryKey: keys.diff(thread, { base: '' }), queryFn: () => api.diff(thread, { base: '' }) })
  const reloadComments = () => queries.invalidateQueries({ queryKey: keys.comments(id) })
  const summary = threads.data?.find((candidate) => candidate.id === thread)
  const [tab, setTab] = useState<InspectorTab>('details')
  const [sheet, setSheet] = useState(false)
  const [draft, setDraft] = useState('')
  const navigate = useNavigate()
  const hash = useRouterState({ select: (state) => state.location.hash })
  // A short id in the address becomes the full one, so the sidebar marks the thread.
  useEffect(() => {
    if (id !== thread) void navigate({ to: '/t/$id', params: { id: thread }, hash: hash.replace(/^#/, '') || undefined, replace: true })
  }, [id, thread, hash, navigate])
  const { agent, tool, preset, setPreset } = useLive(thread)
  const people = usePresence(thread, null, null, draft, undefined, !hosted())
  const lastPrompt = entries.findLast((entry) => entry.kind === 'user')?.text
  const clearDraft = useCallback(() => setDraft(''), [])
  useClearSentDraft(lastPrompt, draft, clearDraft)
  useActions([{ id: 'changes', label: tab === 'changes' ? 'Hide changes' : 'Show changes', keys: 'd', run: () => setTab(tab === 'changes' ? 'details' : 'changes') }])

  // Looking at the thread marks it seen, again whenever something new happens in it.
  const unseen = summary !== undefined && (summary.mine.seen_ms === null || summary.last_ms > summary.mine.seen_ms)
  useEffect(() => {
    if (unseen) void api.setThreadState(thread, { pinned: null, snooze: null, settled: null, seen: true }).catch(() => undefined)
  }, [thread, unseen])

  const rows = rowsOf(entries, turns.data ?? [])
  const list = useRef<HTMLDivElement>(null)
  const scroll = useContext(MainScroll)
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroll.current,
    estimateSize: () => 96,
    overscan: 6,
    scrollMargin: list.current?.offsetTop ?? 0,
  })
  const scrollTo = useCallback((row: number) => virtual.scrollToIndex(row, { align: 'center' }), [virtual])
  const anchor = useAnchor(rows, entries.length >= total, turns.data, comments.data, scrollTo)
  const codeComments = (comments.data ?? []).filter((comment) => comment.location.at !== 'entry')
  const files = diff.data === undefined ? null : parseFiles(diff.data)
  const totals = files && { added: files.reduce((sum, file) => sum + file.added, 0), removed: files.reduce((sum, file) => sum + file.removed, 0) }
  const turn = (turns.data ?? []).length || null
  const entryProps: EntryProps = {
    thread,
    comments: comments.data ?? [],
    anchor,
    onChanged: reloadComments,
    onComment: async (index, text) => {
      await api.addComment(thread, { on: 'entry', index }, text)
      await reloadComments()
    },
  }

  return (
    <Page
      flush
      label="thread"
      inspectorLabel="thread details"
      inspectorWide={tab === 'changes'}
      sheet={{ open: sheet, onClose: () => setSheet(false) }}
      inspector={
        <div className="flex flex-col">
          <div className="sticky top-0 z-10 flex h-10 shrink-0 items-center gap-2 border-b border-line bg-bg px-4">
            <Tabs
              label="inspector"
              tabs={[
                { id: 'details', label: 'Details' },
                {
                  id: 'changes',
                  label: (
                    <>
                      Changes {totals && <Counts added={totals.added} removed={totals.removed} />}
                    </>
                  ),
                },
              ]}
              value={tab}
              onChange={setTab}
            />
          </div>
          {tab === 'details' ? (
            <Details
              thread={thread}
              summary={summary}
              threads={threads.data ?? []}
              total={total}
              diff={diff.data}
              codeComments={codeComments}
              anchor={anchor}
              onChanged={reloadComments}
              onCommentLines={() => setTab('changes')}
            />
          ) : (
            <div className="px-4 py-4">
              <DiffPanel thread={thread} comments={comments.data ?? []} onCommented={reloadComments} agent={agent?.agent} wide />
            </div>
          )}
        </div>
      }
    >
      <ThreadHeader
        thread={thread}
        summary={summary}
        here={people.length}
        changes={totals}
        showingChanges={tab === 'changes'}
        onChanges={() => {
          const opening = tab !== 'changes'
          setTab(opening ? 'changes' : 'details')
          setSheet(opening && window.matchMedia('(max-width: 1023px)').matches)
        }}
        onDetails={() => {
          setTab('details')
          setSheet(true)
        }}
      />
      <div className="flex flex-1 flex-col px-4 md:px-8">
        <div className="flex items-center gap-2 pt-2.5 text-xs text-fg-2">
          <Here people={people} />
          <span>
            {total} {total === 1 ? 'entry' : 'entries'}
            {turn !== null && ` · ${turn} ${turn === 1 ? 'turn' : 'turns'}`}
          </span>
        </div>
        <div ref={list} className="relative max-w-[880px]" style={{ height: virtual.getTotalSize() }} data-testid="history">
          {virtual.getVirtualItems().map((item) => {
            const row = rows[item.index]
            return (
              <div
                key={row.key}
                data-index={item.index}
                ref={virtual.measureElement}
                className="absolute left-0 w-full"
                style={{ transform: `translateY(${item.start - virtual.options.scrollMargin}px)` }}
              >
                <TranscriptRow row={row} {...entryProps} />
              </div>
            )
          })}
        </div>
        <div className="flex max-w-[880px] flex-col gap-2.5 pt-2 pb-4">
          {agent && <AgentTail status={agent} toolOutput={tool.text} />}
          {agent?.permissions.map((asked, index) => (
            <PermissionCard key={asked.id} thread={thread} asked={asked} agent={agent.agent} first={index === 0} />
          ))}
          {agent?.state === 'awaiting_input' && <PlanCard thread={thread} />}
        </div>
        <div className="sticky bottom-0 z-10 mt-auto max-w-[880px] bg-bg pt-1 pb-3 md:pb-4">
          <AgentDock
            thread={thread}
            status={agent}
            preset={preset}
            onPreset={setPreset}
            draft={draft}
            onDraft={setDraft}
            people={people}
            turn={turn}
          />
        </div>
      </div>
    </Page>
  )
}

/** Entries in a thread's first page of history, which shows on its own while
 * the rest loads. */
const FIRST_PAGE = 50
/** Entries in each page after the first. */
const PAGE = 500

/** A thread's whole history, a page at a time: the first is drawn as soon as
 * it comes, and the others follow it. */
function useHistory(id: string) {
  const pages = useInfiniteQuery({
    queryKey: [...keys.history(id), 'pages'],
    queryFn: ({ pageParam }) => api.history(id, pageParam, pageParam === 0 ? FIRST_PAGE : PAGE),
    initialPageParam: 0,
    getNextPageParam: (last, _all, offset) => {
      const next = offset + (offset === 0 ? FIRST_PAGE : PAGE)
      return next < last.total ? next : undefined
    },
  })
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = pages
  useEffect(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage()
  }, [hasNextPage, isFetchingNextPage, fetchNextPage])
  const loaded = pages.data?.pages
  const last = loaded?.[loaded.length - 1]
  return {
    error: pages.error,
    data: last && { thread: last.thread, total: last.total, entries: loaded.flatMap((page) => page.entries) },
  }
}

export function ThreadView({ id }: { id: string }) {
  const history = useHistory(id)
  useTopic('threads')
  useTopic(history.data ? `thread:${history.data.thread}` : null)
  if (history.error) return <p className="p-6 text-danger">{message(history.error)}</p>
  if (!history.data)
    return (
      <div className="flex flex-col gap-3 p-6">
        <Skeleton className="h-6 w-1/3" />
        <Skeleton className="h-4 w-1/2" />
        <Skeleton className="h-40" />
      </div>
    )
  return <ThreadBody thread={history.data.thread} id={id} entries={history.data.entries} total={history.data.total} />
}
