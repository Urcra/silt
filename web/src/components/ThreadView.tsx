import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useRouterState } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import { type ReactNode, type RefObject, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react'
import { AGENTS, api, short, type CommentView, type HistoryEntry, type ThreadSummary, type TurnInfo } from '../api'
import { CloudSession } from './Cloud'
import { useTopic } from '../bus'
import { parseFiles } from '../diffParse'
import { appHref, filePath } from '../permalink'
import { forgeOf, projectName } from '../projects'
import { keys, message } from '../query'
import { useNow } from '../now'
import { useMedia } from '../media'
import { MainScroll } from '../scroll'
import { hosted } from '../served'
import { useClearSentDraft, useLive, usePresence } from '../session'
import { formatKeys, useActions } from '../shortcuts'
import { threadTone, threadWhere } from '../threadLook'
import { taskNotification } from '../taskNotification'
import { useThreadState } from '../threadState'
import { ago } from '../time'
import { type Piece, type Row, type ToolStep, duration, foldInherited, headline, kindOf, originOf, piecesOf, proposedPlan, rowsOf, runItems, shortCommand, thoughtTotal, tokens, toolOf, toolSummary } from '../transcript'
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
import { mocksOn } from '../mocks'
import { DrawerButton, Page } from './Page'
import { ShareDialog } from './ShareDialog'
import { CommandCard, ImageThumb, PlanSteps, ProposedPlan, QuestionCard, StepRow } from './ToolSteps'
import { LiveQuestions } from '../liveQuestions'
import { DETAILS, GROUPINGS, LiveTool, ThoughtsOpen, TranscriptDetail, TranscriptGrouping, leaveDraft, takeDraft, useDetailChoice, useGroupingChoice, type Detail, type Grouping } from '../detail'

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

/** Keeps the page at its bottom while the person is there, as a turn grows and asks things below it;
 * scrolling up leaves them where they are. Says whether it follows, for a jump to the end to set. */
function useFollow(scroll: RefObject<HTMLElement | null>, watched: RefObject<HTMLElement | null>[], thread: string) {
  // Off until the page lands at its end, once the whole history is in: following a history still
  // loading would chase each page's end, and a link to a place in it is where the page stays.
  const atBottom = useRef(false)
  const landed = useRef(false)
  useEffect(() => {
    atBottom.current = false
    landed.current = false
  }, [thread])
  useEffect(() => {
    const element = scroll.current
    if (!element || typeof ResizeObserver === 'undefined') return
    const onScroll = () => {
      if (landed.current) atBottom.current = element.scrollHeight - element.scrollTop - element.clientHeight < 120
    }
    element.addEventListener('scroll', onScroll, { passive: true })
    const grown = new ResizeObserver(() => {
      if (atBottom.current) element.scrollTop = element.scrollHeight
    })
    for (const ref of watched) if (ref.current) grown.observe(ref.current)
    return () => {
      element.removeEventListener('scroll', onScroll)
      grown.disconnect()
    }
    // The refs' elements are the page's own, there from its first render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scroll, thread])
  return useMemo(() => ({ atBottom, landed }), [])
}

/** Follows the page's hash once what it names has loaded, scrolling there with `scrollTo` (a row of `rows`).
 * `complete` says every page of the history has: until then, an entry not among `rows` may still come. */
function useAnchor(
  rows: Row[] | undefined,
  complete: boolean,
  turns: TurnInfo[] | undefined,
  comments: CommentView[] | undefined,
  scrollTo: (row: number) => void,
  scroller: RefObject<HTMLElement | null>,
): Anchor {
  const hash = useRouterState({ select: (state) => state.location.hash }).replace(/^#/, '')
  const anchor = anchorOf(hash, turns, comments)
  // Followed again in a new scroller: the page's column takes over from the main one once the panel beside it shows.
  const followed = useRef<{ hash: string; in: RefObject<HTMLElement | null> } | null>(null)
  useEffect(() => {
    if (!anchor || !rows || (followed.current?.hash === hash && followed.current.in === scroller)) return
    const holds = (row: Row, index: number) =>
      row.kind === 'turn' && (row.prompt?.index === index || row.entries.some((entry) => entry.index === index))
    const at = anchor.entry === undefined ? -1 : rows.findIndex((row) => holds(row, anchor.entry!))
    if (at < 0 && anchor.entry !== undefined && !complete) return
    followed.current = { hash, in: scroller }
    if (at >= 0) scrollTo(at)
    else if (anchor.comment) document.getElementById(`comment-${anchor.comment}`)?.scrollIntoView({ block: 'center' })
  }, [hash, anchor, rows, complete, scrollTo, scroller])
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

/** Copies what a message says, and for a prompt, forks from the turn before it with the prompt in the fork's draft to edit. */
function MessageActions({ entry, thread, turn }: { entry: HistoryEntry; thread: string; turn: number | undefined }) {
  const navigate = useNavigate()
  const [copied, setCopied] = useState(false)
  const action = 'invisible shrink-0 rounded-sm p-0.5 text-fg-2 group-hover/entry:visible hover:bg-surface-2 hover:text-fg-1 focus:visible max-md:visible'
  const copy = () =>
    void navigator.clipboard?.writeText(entry.text).then(
      () => {
        setCopied(true)
        setTimeout(() => setCopied(false), 1500)
      },
      () => undefined,
    )
  const edit = () =>
    api.fork(thread, turn! - 1).then(
      (made) => {
        leaveDraft(made.thread, entry.text)
        void navigate({ to: '/t/$id', params: { id: made.thread } })
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <>
      <button type="button" aria-label={copied ? 'copied' : `copy entry ${entry.index}`} title="Copy" className={action} onClick={copy}>
        <Icon name={copied ? 'check' : 'copy'} className="h-3.5 w-3.5" />
      </button>
      {entry.kind === 'user' && turn !== undefined && turn > 1 && !hosted() && !taskNotification(entry.text) && (
        <button type="button" aria-label={`edit from turn ${turn}`} title="Edit this prompt in a fork from the turn before it" className={action} onClick={() => void edit()}>
          <Icon name="edit" className="h-3.5 w-3.5" />
        </button>
      )}
    </>
  )
}

/** One entry of the history, with its comments and a way to comment on it. */
function Entry({ entry, children, thread, comments, anchor, onComment, onChanged, turn }: EntryProps & { entry: HistoryEntry; children: ReactNode; turn?: number }) {
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
        {/* Over the card's corner on a desktop, so every card in a turn is as wide as the others and nothing sits outside the column. */}
        <span
          className={cx(
            'flex shrink-0 items-center gap-0.5 md:invisible md:absolute md:-top-2.5 md:right-1 md:z-10 md:rounded-sm md:border md:border-line md:bg-elevated md:px-0.5 md:shadow-sm md:group-hover/entry:visible md:focus-within:visible',
            // A phone shows no comment button, so only messages keep the column.
            entry.kind !== 'user' && entry.kind !== 'assistant' && 'max-md:hidden',
          )}
        >
          {(entry.kind === 'user' || entry.kind === 'assistant') && <MessageActions entry={entry} thread={thread} turn={turn} />}
          <button
            type="button"
            aria-label={`comment on entry ${entry.index}`}
            className="invisible shrink-0 rounded-sm p-0.5 text-fg-2 group-hover/entry:visible hover:bg-surface-2 hover:text-fg-1 focus:visible max-md:hidden"
            onClick={() => setCommenting(true)}
          >
            <Icon name="comment" className="h-3.5 w-3.5" />
          </button>
        </span>
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
  const notification = entry.kind === 'user' ? taskNotification(entry.text) : null
  if (notification)
    return (
      <details data-testid="task-notification" className="group/task rounded-md border border-line bg-elevated text-xs">
        <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-1.5 text-fg-2 hover:text-fg-1 max-md:min-h-11">
          <Icon name="chevronRight" className="h-3 w-3 transition-transform group-open/task:rotate-90" />
          <span className="min-w-0 truncate">{notification.summary}</span>
          {notification.status && notification.status !== 'completed' && <span className="text-danger">· {notification.status}</span>}
        </summary>
        {notification.result && (
          <div className="border-t border-line-subtle px-3 py-2 text-sm">
            <Markdown text={notification.result} />
          </div>
        )}
      </details>
    )
  if (entry.kind === 'user')
    return <p className="ml-auto w-fit max-w-[85%] rounded-lg bg-surface-2 px-3.5 py-2.5 text-sm whitespace-pre-wrap">{entry.text}</p>
  // A turn the agent opened itself, as Claude does once a background task ends: nobody said anything.
  if (entry.kind === 'wake')
    return (
      <p data-testid="wake" className="flex w-fit items-center gap-2 rounded-md border border-line bg-elevated px-3 py-1.5 text-xs text-fg-2">
        <Icon name="agent" className="h-3.5 w-3.5 shrink-0" />
        {entry.text}
      </p>
    )
  if (entry.kind === 'assistant') return <Markdown text={entry.text} />
  if (entry.kind === 'merge' || entry.kind === 'pull') {
    // `merged "title" from fork <id>: <summary>`, whose summary is Markdown that reads as such folded under the line.
    const [, said, summary] = /^([^:]*?from fork \S+): ([\s\S]+)$/.exec(entry.text) ?? [null, entry.text, null]
    return (
      <details className="group/merge text-xs text-info">
        <summary className={cx('flex items-center gap-1.5', summary ? 'cursor-pointer' : 'list-none')}>
          <Icon name="merge" className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0">{said}</span>
          {summary && <Icon name="chevronRight" className="h-3 w-3 shrink-0 transition-transform group-open/merge:rotate-90" />}
        </summary>
        {summary && (
          <div className="mt-1.5 border-l-2 border-line pl-3 text-sm text-fg-1">
            <Markdown text={summary} />
          </div>
        )}
      </details>
    )
  }
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

/** A thought in its own look, a rule beside it: how long it took, its words a click away. */
function Thought({ entries, took, ...props }: EntryProps & { entries: HistoryEntry[]; took: string | null }) {
  const verbose = useContext(TranscriptDetail) === 'verbose'
  const thoughts = useContext(ThoughtsOpen)
  return (
    <details key={`${verbose}-${thoughts.times}`} open={verbose || thoughts.open} data-testid="thought" className="group/think border-l-2 border-line pl-3 text-xs text-fg-2">
      <summary className="flex cursor-pointer list-none items-center gap-1.5 hover:text-fg-1 max-md:min-h-11">
        <Icon name="chevronRight" className="h-3 w-3 transition-transform group-open/think:rotate-90" />
        {took ? `Thought for ${took}` : 'Thought'}
      </summary>
      <div className="mt-1.5 flex flex-col gap-1.5">
        {entries.map((entry) => (
          <Entry key={entry.index} entry={entry} {...props}>
            <EntryText entry={entry} />
          </Entry>
        ))}
      </div>
    </details>
  )
}

/** How many of a run's tool calls failed. */
function failedIn(entries: HistoryEntry[]): number {
  return entries.filter((entry) => entry.kind === 'tool_result' && entry.outcome?.failed).length
}

/** One of a run's steps, as its row. */
function StepItem({ step, running, ...props }: EntryProps & { step: ToolStep; running: boolean }) {
  const verbose = useContext(TranscriptDetail) === 'verbose'
  const anchor = step.call ?? step.result!
  return (
    <Entry entry={anchor} {...props}>
      {step.call && kindOf(step.call) === 'execute' ? <CommandCard step={step} running={running} /> : <StepRow step={step} open={verbose} running={running} />}
    </Entry>
  )
}

/**
 * A run of the agent's work between its words (ADR 0049): one closed card that says what it did and how long it
 * thought, its thoughts and steps inside in the order taken. `live`: the turn is still running, so the step
 * without its result is under way, and shows below the card's line with the thought that led to it.
 */
function RunFold({ entries, until, live = false, ...props }: EntryProps & { entries: HistoryEntry[]; until: number | null; live?: boolean }) {
  const verbose = useContext(TranscriptDetail) === 'verbose'
  const thoughts = useContext(ThoughtsOpen)
  const items = runItems(entries, until)
  const steps = items.flatMap((item) => (item.kind === 'step' ? [item.step] : []))
  const current = live ? steps.find((step) => step.call && !step.result) : undefined
  const thought = thoughtTotal(items)
  if (steps.length === 0) {
    return (
      <>
        {items.map((item) => item.kind === 'thought' && <Thought key={item.entries[0].index} entries={item.entries} took={item.took} {...props} />)}
      </>
    )
  }
  const tools = entries.filter((entry) => entry.kind !== 'thinking')
  const failed = failedIn(tools)
  const only = steps.length === 1 && !current ? steps[0] : null
  const led = current && [...items.slice(0, items.findIndex((item) => item.kind === 'step' && item.step === current))].reverse().find((item) => item.kind === 'thought')
  const open = verbose || (thoughts.open && items.some((item) => item.kind === 'thought'))
  if (current && steps.length === 1 && !items.some((item) => item.kind === 'thought')) {
    return (
      <div data-testid="run" className="rounded-md border border-line bg-elevated px-3 py-1.5 text-xs">
        <StepItem step={current} running {...props} />
      </div>
    )
  }
  return (
    <div data-testid="run" className="rounded-md border border-line bg-elevated text-xs">
      <details key={`${verbose}-${thoughts.times}`} open={open} data-testid="tools" className="group/tools">
        <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-1.5 text-fg-2 hover:text-fg-1 max-md:min-h-11">
          <Icon name="chevronRight" className="h-3 w-3 shrink-0 transition-transform group-open/tools:rotate-90" />
          {/* A run that is so far only thoughts and the step under way is named by its thinking; the step shows below. */}
          {current && steps.length === 1 ? (
            <span className="min-w-0 truncate">{thought ? `Thought for ${thought}` : 'Thought'}</span>
          ) : (
            <>
              <span className="min-w-0 truncate">{only ? stepLabel(only) : toolSummary(tools, live, 3)}</span>
              {thought && <span className="shrink-0 text-fg-3">· thought for {thought}</span>}
            </>
          )}
          {failed > 0 && <span className="shrink-0 text-danger">· {failed} failed</span>}
        </summary>
        <div className="flex flex-col gap-1.5 border-t border-line-subtle px-3 py-2">
          {items.map((item) =>
            item.kind === 'thought' ? (
              <Thought key={item.entries[0].index} entries={item.entries} took={item.took} {...props} />
            ) : item.step === current ? null : (
              <StepItem key={(item.step.call ?? item.step.result!).index} step={item.step} running={false} {...props} />
            ),
          )}
        </div>
      </details>
      {current && (
        <div className="flex flex-col gap-1.5 border-t border-line-subtle px-3 py-2">
          {led?.kind === 'thought' && <p className="line-clamp-2 text-xs text-fg-2 italic">{headline(led.entries.map((entry) => entry.text).join(' '))}</p>}
          <StepItem step={current} running {...props} />
        </div>
      )}
    </div>
  )
}

/**
 * A turn still running, grouped by turn (ADR 0049): everything since the agent last wrote as one card. It says what
 * that work did, the thought that led to the step under way, the last few steps and the one running; the rest
 * opens from it. Once the turn ends, the turn folds under "Worked for".
 */
function ActivityCard({ entries, wrote, ...props }: EntryProps & { entries: HistoryEntry[]; wrote: boolean }) {
  const verbose = useContext(TranscriptDetail) === 'verbose'
  const [all, setAll] = useState(false)
  const items = runItems(entries, null)
  const steps = items.flatMap((item) => (item.kind === 'step' ? [item.step] : []))
  const current = steps.find((step) => step.call && !step.result)
  const done = steps.filter((step) => step !== current)
  const thought = thoughtTotal(items)
  const before = current ? items.slice(0, items.findIndex((item) => item.kind === 'step' && item.step === current)) : items
  const led = [...before].reverse().find((item) => item.kind === 'thought')
  const tools = entries.filter((entry) => entry.kind !== 'thinking')
  const failed = failedIn(tools)
  const shown = verbose || all
  const recent = done.slice(-3)
  const thoughts = items.filter((item) => item.kind === 'thought').length
  // What the card leaves out: earlier steps, and thoughts other than the headline it shows.
  const hidden = done.length - recent.length + thoughts - (led ? 1 : 0)
  return (
    <section data-testid="activity" aria-label="the agent's work so far" className="flex flex-col gap-2 rounded-md border border-line bg-elevated px-3 py-2 text-xs">
      <p className="flex min-w-0 items-center gap-1.5 text-fg-2">
        <span className="shrink-0 font-medium text-fg-1">{wrote ? 'Since it last wrote' : 'So far'}</span>
        {done.length > 0 && <span className="min-w-0 truncate">· {toolSummary(tools.filter((entry) => !current || (entry !== current.call && entry !== current.result)), false, 3)}</span>}
        {thought && <span className="shrink-0 text-fg-3">· thought for {thought}</span>}
        {failed > 0 && <span className="shrink-0 text-danger">· {failed} failed</span>}
      </p>
      {led?.kind === 'thought' && !shown && <p className="line-clamp-2 text-fg-2 italic">{headline(led.entries.map((entry) => entry.text).join(' '))}</p>}
      <div className="flex flex-col gap-1">
        {shown
          ? items.map((item) =>
              item.kind === 'thought' ? (
                <Thought key={item.entries[0].index} entries={item.entries} took={item.took} {...props} />
              ) : (
                <StepItem key={(item.step.call ?? item.step.result!).index} step={item.step} running={item.step === current} {...props} />
              ),
            )
          : [
              ...recent.map((step) => (
                <div key={(step.call ?? step.result!).index} className="opacity-80">
                  <StepItem step={step} running={false} {...props} />
                </div>
              )),
              current && <StepItem key={current.call!.index} step={current} running {...props} />,
            ]}
      </div>
      {!verbose && hidden > 0 && (
        <button type="button" onClick={() => setAll(!all)} className="w-fit text-accent-fg hover:text-fg-1 max-md:min-h-11">
          {all ? 'Show only the latest' : `Show everything · ${steps.length} ${steps.length === 1 ? 'step' : 'steps'}${thoughts > 0 ? `, ${thoughts} ${thoughts === 1 ? 'thought' : 'thoughts'}` : ''}`}
        </button>
      )}
    </section>
  )
}

/** What one step did, as a run of one says it: a command's purpose or the command, else what the tool did. */
function stepLabel(step: ToolStep): string {
  const call = step.call
  if (!call) return "A tool's output"
  if (kindOf(call) === 'execute') return call.tool?.detail?.trim() || shortCommand(call.tool?.subject ?? toolOf(call.text).name)
  return toolSummary(entriesOfStep(step))
}

function entriesOfStep(step: ToolStep): HistoryEntry[] {
  return [step.call, step.result].filter((entry): entry is HistoryEntry => entry !== null)
}

/** The thread's files this machine leaves off its disk: names its file system can't hold, such as `aux.txt` on Windows, or that differ only in case (ADR 0045). */
function Unwritten({ thread }: { thread: string }) {
  const unwritten = useQuery({ queryKey: keys.unwritten(thread), queryFn: () => api.unwritten(thread), enabled: !hosted(), retry: false })
  const paths = unwritten.data ?? []
  if (paths.length === 0) return null
  return (
    <p role="note" className="mb-4 rounded-md border border-line-subtle px-3 py-2 text-xs text-fg-2">
      {paths.length === 1 ? 'One file' : `${paths.length} files`} of this thread {paths.length === 1 ? "isn't" : "aren't"} written on this machine, as its file system can't hold{' '}
      {paths.length === 1 ? 'its name' : 'their names'}: <span className="font-mono">{paths.join(', ')}</span>. The thread keeps {paths.length === 1 ? 'it' : 'them'} for machines that can.
    </p>
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
          {row.tokens > 0 && <> · {tokens(row.tokens)} tokens</>}
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

/** Where the history moves to another thread's part: the source before a fork, or upstream work pulled in. Named by its title, as people know it. */
function OriginDivider({ origin, thread, folded, onUnfold }: { origin: string; thread: string; folded?: number; onUnfold?: () => void }) {
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const title = threads.data?.find((candidate) => candidate.id === thread || candidate.id.endsWith(thread))?.title
  return (
    <div role="separator" aria-label={origin === 'own' ? 'this fork starts here' : `from ${title ?? short(thread)}`} className="flex items-center gap-2.5 py-2 text-xs text-fg-2">
      <span className="h-px flex-1 bg-line" />
      {origin === 'own' ? (
        <span className="text-2xs font-medium text-fg-1">This fork starts here</span>
      ) : (
        <>
          <span className="min-w-0 truncate text-2xs">
            {origin === 'inherited' ? 'From ' : 'Upstream progress from '}
            <Link to="/t/$id" params={{ id: thread }} className="text-accent-fg hover:underline" title={title ?? thread} data-volatile>
              {title ?? short(thread)}
            </Link>
            {origin === 'inherited' && ', before this fork'}
          </span>
          {/* Outside the cut title, so a long one never hides it. */}
          {folded !== undefined && folded > 0 && (
            <button type="button" className="shrink-0 text-2xs whitespace-nowrap text-accent-fg hover:underline" onClick={onUnfold}>
              Show {folded} {folded === 1 ? 'turn' : 'turns'}
            </button>
          )}
        </>
      )}
      <span className="h-px flex-1 bg-line" />
    </div>
  )
}

/** `running`: the turn is still under way, so it has no totals or actions yet. `onUnfold` shows what a fork inherited. */
function TranscriptRow({ row, running = false, onUnfold, ...props }: EntryProps & { row: Row; running?: boolean; onUnfold?: () => void }) {
  const [summary, setSummary] = useState(false)
  const grouping = useContext(TranscriptGrouping)
  const verbose = useContext(TranscriptDetail) === 'verbose'
  const thoughts = useContext(ThoughtsOpen)
  if (row.kind === 'origin') return <OriginDivider origin={row.origin} thread={row.thread} folded={row.folded} onUnfold={onUnfold} />
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
  // Grouped by turn, a finished turn's work folds under one line, leaving its last reply showing (ADR 0049).
  const last = pieces.findLastIndex((piece) => piece.kind === 'entry' && piece.entry.kind === 'assistant')
  const folded = grouping === 'turn' && !running && Boolean(row.turn) && last >= 0 && pieces.length > 1
  // What the person decided, a question they answered or a plan put to them, stays out of the fold.
  const decided = (piece: Piece) => piece.kind === 'step' && Boolean(piece.step.call) && (kindOf(piece.step.call!) === 'question' || Boolean(proposedPlan(piece.step.call!)))
  const work = folded ? pieces.filter((piece, index) => index !== last && !decided(piece)) : []
  const reply = folded ? [...pieces.filter(decided), pieces[last]] : []
  // Grouped by turn, a turn still running shows the work since the agent last wrote as one card; what it said
  // and what waits on the person stay outside it.
  const wrote = pieces.findLastIndex((piece) => piece.kind === 'entry' && piece.entry.kind === 'assistant')
  const since = grouping === 'turn' && running ? pieces.slice(wrote + 1) : []
  const carded = since.filter((piece) => piece.kind === 'tools' || (piece.kind === 'step' && !decided(piece)))
  const activity = carded.some((piece) => piece.kind === 'step' || (piece.kind === 'tools' && piece.entries.some((entry) => entry.kind !== 'thinking'))) ? carded.flatMap((piece) => (piece.kind === 'tools' ? piece.entries : piece.kind === 'step' ? entriesOfStep(piece.step) : [])).sort((a, b) => a.index - b.index) : []
  const final = row.entries[row.entries.length - 1]
  const worked = row.prompt && final ? duration(row.prompt.time_ms, final.time_ms) : null
  const drawn = (piece: Piece) =>
          piece.kind === 'tools' ? (
            <RunFold key={piece.entries[0].index} entries={piece.entries} until={piece.until} live={running} {...props} />
          ) : piece.kind === 'plan' ? (
            <Entry key={piece.entry.index} entry={piece.entry} {...props}>
              <PlanSteps entry={piece.entry} />
            </Entry>
          ) : piece.kind === 'step' ? (
            <Entry key={piece.step.call!.index} entry={piece.step.call!} {...props}>
              {proposedPlan(piece.step.call!) ? (
                <ProposedPlan step={piece.step} />
              ) : kindOf(piece.step.call!) === 'question' ? (
                <QuestionCard step={piece.step} />
              ) : (
                <CommandCard step={piece.step} card />
              )}
            </Entry>
          ) : (
            <Entry key={piece.entry.index} entry={piece.entry} {...props}>
              <EntryText entry={piece.entry} />
            </Entry>
          )
  return (
    <div id={row.turn ? `turn-${row.turn.number}` : undefined} className="py-3">
      <div className="flex min-w-0 flex-col gap-3.5">
        {row.prompt && (
          <Entry entry={row.prompt} turn={row.turn?.number} {...props}>
            <EntryText entry={row.prompt} />
            {row.images.length > 0 && (
              <div className="mt-1.5 flex flex-wrap justify-end gap-2">
                {row.images.map((image) => (
                  <ImageThumb key={image.index} entry={image} from="in the prompt" />
                ))}
              </div>
            )}
          </Entry>
        )}
        {folded ? (
          <>
            <details
              key={`${verbose}-${thoughts.times}`}
              open={verbose || (thoughts.open && row.entries.some((entry) => entry.kind === 'thinking'))}
              data-testid="worked"
              className="group/worked text-xs text-fg-2"
            >
              <summary className="cursor-pointer list-none hover:text-fg-1 max-md:min-h-11">
                <Icon name="chevronRight" className="mr-1.5 inline h-3 w-3 align-[-1px] transition-transform group-open/worked:rotate-90" />
                <span className="text-fg-1">{worked ? `Worked for ${worked}` : 'Worked'}</span> · {toolSummary(row.entries, false, 2)}
                {failedIn(row.entries) > 0 && <span className="text-danger"> · {failedIn(row.entries)} failed on the way</span>}
              </summary>
              <div className="mt-3 flex flex-col gap-3.5 border-l-2 border-line pl-3 text-sm text-fg-1">{work.map(drawn)}</div>
            </details>
            {reply.map(drawn)}
          </>
        ) : activity.length > 0 ? (
          <>
            {pieces.slice(0, wrote + 1).map(drawn)}
            {/* The card stands where its first piece was; what isn't in it keeps its place. */}
            {since.map((piece) =>
              piece === carded[0] ? <ActivityCard key="activity" entries={activity} wrote={wrote >= 0} {...props} /> : carded.includes(piece) ? null : drawn(piece),
            )}
          </>
        ) : (
          pieces.map(drawn)
        )}
        {row.turn && !running && <TurnBar thread={props.thread} turn={row.turn} row={row} rewound={rewound} />}
      </div>
    </div>
  )
}

function ThreadStateActions({ summary }: { summary: ThreadSummary }) {
  const items = useThreadState(summary)
  useActions(items.map((item) => ({ id: `state:${item.label}`, label: `${item.label} this thread`, run: item.onSelect })))
  return null
}

/** A fork's work offered back to its source as a merge request, by its title. */
function OfferForm({ thread, onError }: { thread: string; onError: (failure: unknown) => void }) {
  const navigate = useNavigate()
  const [title, setTitle] = useState('')
  return (
    <form
      className="flex items-center gap-2 [&>input]:min-w-0 [&>input]:flex-1"
      onSubmit={(event) => {
        event.preventDefault()
        api.offerMergeRequest(thread, title).then((mr) => {
          toast(`Offered "${mr.title}"`, 'ok')
          void navigate({ to: '/mr/$id', params: { id: mr.id } })
        }, onError)
      }}
    >
      <Input aria-label="merge request title" autoFocus placeholder="What it does" value={title} onChange={(event) => setTitle(event.target.value)} />
      <Button type="submit" variant="primary" disabled={!title.trim()}>
        Offer
      </Button>
    </form>
  )
}

/** What the thread changed, over the composer: how much, a way to review it, a pull request on the forge, and for a fork, offering it back. */
function ChangesBar({
  thread,
  files,
  totals,
  forked,
  forge,
  onReview,
}: {
  thread: string
  files: number
  totals: { added: number; removed: number }
  forked: boolean
  /** The forge the project's remote is on, by name, when it is a known one. */
  forge: string | undefined
  onReview: () => void
}) {
  const [offering, setOffering] = useState(false)
  const [pushing, setPushing] = useState(false)
  // Asked again with the changes, as a commit or a branch can lift what stood in the way.
  const blocker = useQuery({
    queryKey: [...keys.thread(thread), 'pull-request', files, totals.added, totals.removed],
    queryFn: () => api.pullRequestBlocker(thread),
    enabled: !hosted(),
  })
  // The tab opens on the click, so no popup blocker stops it, and goes to the forge once the push is done.
  const createPr = () => {
    const tab = window.open('', '_blank')
    setPushing(true)
    api.openPullRequest(thread).then(
      (url) => {
        setPushing(false)
        if (tab) tab.location.href = url
        else window.location.assign(url)
      },
      (failure: unknown) => {
        setPushing(false)
        tab?.close()
        toast(message(failure), 'danger')
      },
    )
  }
  return (
    <div role="group" aria-label="what the thread changed" className="mb-2 flex flex-wrap items-center gap-2 rounded-md border border-line bg-elevated px-3 py-1.5 text-xs">
      <Icon name="file" className="h-3.5 w-3.5 text-fg-2" />
      <span>
        {files} {files === 1 ? 'file' : 'files'} changed
      </span>
      <Counts added={totals.added} removed={totals.removed} />
      <span className="ml-auto flex items-center gap-1.5">
        <Button size="sm" variant="ghost" onClick={onReview}>
          Review
        </Button>
        {forked &&
          (offering ? (
            <OfferForm thread={thread} onError={(failure) => toast(message(failure), 'danger')} />
          ) : (
            <Button size="sm" variant={hosted() ? 'primary' : 'secondary'} icon="merge" onClick={() => setOffering(true)}>
              Offer as merge request
            </Button>
          ))}
        {!hosted() && !offering && (
          <Button
            size="sm"
            variant={blocker.data ? 'secondary' : 'primary'}
            icon="external"
            disabled={pushing || Boolean(blocker.data)}
            onClick={createPr}
            title={blocker.data ? `No pull request yet: ${blocker.data}` : `Pushes the thread's branch to origin and opens ${forge ?? "the forge"}'s page for its pull request`}
          >
            {pushing ? 'Pushing…' : forge ? `Create PR on ${forge}` : 'Create PR on the forge'}
          </Button>
        )}
      </span>
    </div>
  )
}

function ForkActions({ thread, forked }: { thread: string; forked: boolean }) {
  const navigate = useNavigate()
  // Each time the palette asks, the offer opens again.
  const [offered, setOffered] = useState(0)
  const failed = (failure: unknown) => toast(message(failure), 'danger')
  const fork = () => api.fork(thread).then((made) => void navigate({ to: '/t/$id', params: { id: made.thread } }), failed)
  useActions([
    { id: 'fork', label: 'Fork this thread', run: () => void fork() },
    ...(forked ? [{ id: 'offer', label: 'Offer as merge request', run: () => setOffered((count) => count + 1) }] : []),
  ])
  return (
    <>
      <Button icon="fork" onClick={() => void fork()} aria-label="Fork" title="A copy of the thread, in a checkout of its own, to try something else in">
        <span className="@max-xl/head:sr-only">Fork</span>
      </Button>
      {forked && (
        <Popover
          key={offered}
          openOnMount={offered > 0}
          label="Offer as merge request"
          triggerLabel="Offer as merge request"
          align="right"
          triggerClassName={buttonClass()}
          panelClassName="w-80 p-2"
          trigger={
            <span className="inline-flex items-center gap-1.5" title="Offer this fork's work back to its source">
              <Icon name="merge" className="h-3.5 w-3.5" />
              <span className="@max-3xl/head:sr-only">Offer as merge request</span>
            </span>
          }
        >
          <OfferForm thread={thread} onError={failed} />
        </Popover>
      )}
    </>
  )
}

/** The header's More (ADR 0044): the commands that pick the thread up in a terminal, with which agent, Join live, the graph and the details. */
function ThreadMore({
  thread,
  summary,
  here,
  onDetails,
  detail,
  onDetail,
  grouping,
  onGrouping,
}: {
  thread: string
  summary: ThreadSummary | undefined
  here: number
  onDetails: () => void
  detail: Detail
  onDetail: (detail: Detail) => void
  grouping: Grouping
  onGrouping: (grouping: Grouping) => void
}) {
  const [agent, setAgent] = useState('')
  const commands = [`silt resume ${short(thread)}${agent ? ` --agent ${agent}` : ''}`, `silt fork ${short(thread)}${agent ? ` --agent ${agent}` : ''}`]
  return (
    <Popover
      label="more for this thread"
      triggerLabel="More for this thread"
      align="right"
      triggerClassName="inline-flex h-7 w-7 items-center justify-center rounded-sm text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11 max-md:min-w-11"
      panelClassName="flex w-80 flex-col gap-1 p-2"
      trigger={<Icon name="more" />}
    >
      {(close) => (
        <>
          <p className="px-1 pb-0.5 text-2xs text-fg-2">Group the agent&apos;s work</p>
          <span role="radiogroup" aria-label="transcript grouping" className="mx-1 flex rounded-sm border border-line p-0.5">
            {GROUPINGS.map((choice) => (
              <button
                key={choice.id}
                type="button"
                role="radio"
                aria-checked={grouping === choice.id}
                title={choice.says}
                onClick={() => onGrouping(choice.id)}
                className={cx('flex-1 rounded-[3px] px-2 py-1 text-xs max-md:min-h-11', grouping === choice.id ? 'bg-surface-2 text-fg-1' : 'text-fg-2 hover:text-fg-1')}
              >
                {choice.label}
              </button>
            ))}
          </span>
          <p className="px-1 pt-1 pb-0.5 text-2xs text-fg-2">How much each step shows</p>
          <span role="radiogroup" aria-label="transcript detail" className="mx-1 flex rounded-sm border border-line p-0.5">
            {DETAILS.map((choice) => (
              <button
                key={choice.id}
                type="button"
                role="radio"
                aria-checked={detail === choice.id}
                title={choice.says}
                onClick={() => onDetail(choice.id)}
                className={cx('flex-1 rounded-[3px] px-2 py-1 text-xs max-md:min-h-11', detail === choice.id ? 'bg-surface-2 text-fg-1' : 'text-fg-2 hover:text-fg-1')}
              >
                {choice.label}
              </button>
            ))}
          </span>
          <div className="my-1 border-t border-line-subtle" />
          <p className="px-1 pb-0.5 text-2xs text-fg-2">Continue in a terminal, in a clone of the repository</p>
          <label className="flex items-center gap-2 px-1 text-xs text-fg-2">
            with
            <select
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
          </label>
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
              <Icon name="copy" />
              {command}
            </button>
          ))}
          <div className="my-1 border-t border-line-subtle" />
          {!hosted() && (
            <Link to="/t/$id/live" params={{ id: thread }} className={cx(buttonClass('ghost'), 'justify-start font-normal text-fg-1')}>
              <Dot tone={here > 0 ? 'ok' : 'idle'} />
              Join live{here > 0 && ` · ${here} here`}
            </Link>
          )}
          <Link to="/graph" className={cx(buttonClass('ghost'), 'justify-start font-normal text-fg-1')}>
            <Icon name="graph" />
            History graph
          </Link>
          <button
            type="button"
            className={cx(buttonClass('ghost'), 'justify-start font-normal text-fg-1')}
            onClick={() => {
              close()
              onDetails()
            }}
          >
            <Icon name="info" />
            Details
          </button>
        </>
      )}
    </Popover>
  )
}

/** The thread's header (ADR 0044): its title and where it is, then Fork, Share, Changes, and the rest behind More. */
function ThreadHeader({
  thread,
  summary,
  people,
  changes,
  panelOpen,
  waiting,
  onChanges,
  onDetails,
  detail,
  onDetail,
  grouping,
  onGrouping,
}: {
  thread: string
  summary: ThreadSummary | undefined
  detail: Detail
  onDetail: (detail: Detail) => void
  grouping: Grouping
  onGrouping: (grouping: Grouping) => void
  people: { user: string; device: string }[]
  changes: { added: number; removed: number } | null
  panelOpen: boolean
  /** The agent waits on the person: a plan to approve or a permission. */
  waiting: boolean
  onChanges: () => void
  /** Opens the details, beside the conversation or as a sheet on a phone. */
  onDetails: () => void
}) {
  const where = summary ? threadWhere(summary) : null
  const commits = useQuery({ queryKey: keys.commits(thread), queryFn: () => api.commits(thread), retry: false })
  const branch = commits.data?.findLast((commit) => commit.branch)?.branch
  // Where it is and what records it; who started it and the rest are in Details.
  const meta = summary ? [projectName(summary.repo || 'no remote'), where?.label.toLowerCase(), summary.agent].filter(Boolean) : []
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const source = summary?.forked_from ? threads.data?.find((candidate) => candidate.id === summary.forked_from) : undefined
  return (
    <header className="@container/head sticky top-0 z-10 flex h-12 shrink-0 items-center gap-2.5 border-b border-line-subtle bg-bg px-4 md:pr-3 md:pl-5">
      <DrawerButton />
      {/* Two lines, as ADR 0044 draws it: the title keeps the room, what it is goes under it. */}
      <div className="flex min-w-0 flex-1 flex-col justify-center">
        <div className="flex min-w-0 items-center gap-2">
          <h1 className="min-w-0 truncate text-[15px] leading-tight font-medium" title={summary?.title ?? undefined}>
            {summary?.title ?? 'Thread'}
          </h1>
          {waiting && (
            <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-warn-tint px-2 py-0.5 text-2xs whitespace-nowrap max-md:hidden">
              <Dot tone="warn" />
              Waiting on you
            </span>
          )}
          {summary?.local_only && (
            <span className="shrink-0 max-md:hidden" title="Its project keeps new threads off every server; choose a server on the Projects page to move it">
              <Tag>This machine only</Tag>
            </span>
          )}
        </div>
        <div className="flex min-w-0 items-center gap-2 overflow-hidden font-mono text-2xs leading-tight whitespace-nowrap text-fg-2">
          <span className="min-w-0 truncate" data-volatile>
            {short(thread)}
            {meta.map((part) => ` · ${part}`)}
          </span>
          {branch && (
            <span className="inline-flex min-w-0 shrink items-center gap-1" title="The branch its latest commit went on">
              <Icon name="branch" className="h-3 w-3 shrink-0" />
              <span className="truncate">{branch}</span>
            </span>
          )}
          {summary?.forked_from && !summary.title?.startsWith('Fork of ') && (
            <span className="inline-flex min-w-0 shrink items-center gap-1 font-sans">
              fork of
              <Link className="min-w-0 truncate text-accent-fg" to="/t/$id" params={{ id: summary.forked_from }} title={summary.forked_from} data-volatile>
                {source?.title ?? short(summary.forked_from)}
              </Link>
            </span>
          )}
        </div>
      </div>
      <Here people={people} />
      <span className="flex shrink-0 items-center gap-1">
        <ForkActions thread={thread} forked={Boolean(summary?.forked_from)} />
        {summary && <ShareDialog thread={summary} />}
        <Button
          aria-label="Changes"
          aria-pressed={panelOpen}
          onClick={onChanges}
          title="What the thread changed, beside the conversation (D)"
          // Amber is for what waits on someone; an open panel is only on.
          className="aria-pressed:border-line-strong aria-pressed:bg-surface-2 aria-pressed:text-fg-1"
        >
          <Icon name="file" className="h-3.5 w-3.5 @xl/head:hidden" />
          <span className="@max-xl/head:sr-only">Changes</span>
          {changes && !panelOpen && (changes.added > 0 || changes.removed > 0) && <Counts added={changes.added} removed={changes.removed} />}
        </Button>
        <ThreadMore thread={thread} summary={summary} here={people.length} onDetails={onDetails} detail={detail} onDetail={onDetail} grouping={grouping} onGrouping={onGrouping} />
      </span>
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
  total,
  diff,
  codeComments,
  anchor,
  onChanged,
  onCommentLines,
}: {
  thread: string
  summary: ThreadSummary | undefined
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
            // Rows with nothing to say yet are left out rather than shown blank.
            ...(mocksOn
              ? ([
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
                ] as [ReactNode, ReactNode][])
              : []),
            ...(summary ? ([['Last moved', <span key="moved" data-volatile>{ago(summary.last_ms, now)}</span>]] as [ReactNode, ReactNode][]) : []),
          ]}
        />
        <p className="text-xs leading-normal text-fg-2">Picking it up on another machine moves the agent session there. This one stops writing.</p>
      </InspectorSection>
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
              <Counts added={file.added} removed={file.removed} binary={file.binary} />
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

type InspectorTab = 'changes' | 'details' | 'lineage'

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
  const [tab, setTab] = useState<InspectorTab>('changes')
  // Beside the conversation on a desktop, open on Changes (ADR 0044) until closed.
  const [panel, setPanel] = useState(true)
  // The panel sits beside the conversation only on a desktop; a phone opens it as a sheet.
  const desktop = useMedia('(min-width: 1024px)')
  const [sheet, setSheet] = useState(false)
  const [draft, setDraft] = useState('')
  const [detail, setDetail] = useDetailChoice()
  const [grouping, setGrouping] = useGroupingChoice()
  const [thoughts, setThoughts] = useState({ open: false, times: 0 })
  const toggleThoughts = () => {
    const open = !thoughts.open
    setThoughts({ open, times: thoughts.times + 1 })
    toast(open ? `Every thought open · ${formatKeys('shift+t')} closes them` : 'Every thought closed')
  }
  // A prompt to edit, left by Edit on the thread this one was forked from.
  const [leftFor, setLeftFor] = useState<string>()
  if (leftFor !== thread) {
    setLeftFor(thread)
    const left = takeDraft(thread)
    if (left) setDraft(left)
  }
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
  useActions([
    { id: 'changes', label: panel ? 'Hide the changes panel' : 'Show changes', keys: 'd', run: () => setPanel(!panel) },
    ...DETAILS.filter((choice) => choice.id !== detail).map((choice) => ({ id: `detail:${choice.id}`, label: `Transcript: ${choice.label}`, run: () => setDetail(choice.id) })),
    ...GROUPINGS.filter((choice) => choice.id !== grouping).map((choice) => ({ id: `grouping:${choice.id}`, label: `Group the work ${choice.label.toLowerCase()}`, run: () => setGrouping(choice.id) })),
    { id: 'thoughts', label: thoughts.open ? 'Close every thought' : 'Open every thought', keys: 'shift+t', run: toggleThoughts },
  ])

  // Looking at the thread marks it seen, again whenever something new happens in it.
  const unseen = summary !== undefined && (summary.mine.seen_ms === null || summary.last_ms > summary.mine.seen_ms)
  useEffect(() => {
    if (unseen) void api.setThreadState(thread, { pinned: null, snooze: null, settled: null, seen: true }).catch(() => undefined)
  }, [thread, unseen])

  // A plan call's own output is the plan again, then the agent's note that it was approved; the plan card shows it once.
  const planCalls = useRef(new Set<string>())
  for (const asked of agent?.permissions ?? []) if (asked.plan && asked.tool_call) planCalls.current.add(asked.tool_call)
  const [unfolded, setUnfolded] = useState<string | null>(null)
  const history = rowsOf(entries, turns.data ?? [])
  // A link into the history may name an inherited turn, so it shows them all.
  const rows = unfolded === thread || hash ? history : foldInherited(history)
  const list = useRef<HTMLDivElement>(null)
  // Beside the open panel the history scrolls in the page's own column (see Page), not the main one.
  const outer = useContext(MainScroll)
  const column = useRef<HTMLDivElement>(null)
  const scroll = (panel || sheet) && desktop ? column : outer
  const virtual = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroll.current,
    estimateSize: () => 96,
    overscan: 6,
    scrollMargin: list.current?.offsetTop ?? 0,
  })
  const scrollTo = useCallback((row: number) => virtual.scrollToIndex(row, { align: 'center' }), [virtual])
  // After a prompt is sent, the page follows to its turn, which arrives a moment later.
  const following = useRef(false)
  const below = useRef<HTMLDivElement>(null)
  const follow = useFollow(scroll, [list, below], thread)
  const toEnd = useCallback(() => {
    follow.landed.current = true
    follow.atBottom.current = true
    if (rows.length > 0) virtual.scrollToIndex(rows.length - 1, { align: 'end' })
    // What waits below the history, a permission or a plan, comes into view too.
    requestAnimationFrame(() => {
      if (scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight
    })
  }, [virtual, rows.length, scroll, follow])
  useEffect(() => {
    if (!following.current) return
    following.current = false
    toEnd()
  }, [rows.length, toEnd])
  const anchor = useAnchor(rows, entries.length >= total, turns.data, comments.data, scrollTo, scroll)
  // A thread opens where it is now, its latest turn, unless a link names a place in it.
  const landed = useRef<string | null>(null)
  const linked = anchor.entry !== undefined || anchor.comment !== undefined
  useEffect(() => {
    if (landed.current === thread || entries.length < total || rows.length === 0) return
    landed.current = thread
    // Linked, the page stays where the link put it until the person scrolls to its end.
    if (linked) follow.landed.current = true
    else toEnd()
  }, [thread, entries.length, total, rows.length, linked, toEnd, follow])
  const codeComments = (comments.data ?? []).filter((comment) => comment.location.at !== 'entry')
  const files = diff.data === undefined ? null : parseFiles(diff.data)
  const totals = files && { added: files.reduce((sum, file) => sum + file.added, 0), removed: files.reduce((sum, file) => sum + file.removed, 0) }
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

  const showChanges = () => {
    const phone = window.matchMedia('(max-width: 1023px)').matches
    if (phone) {
      setTab('changes')
      setSheet(true)
      return
    }
    if (!panel || tab !== 'changes') {
      setTab('changes')
      setPanel(true)
    } else setPanel(false)
  }
  const busy = agent?.state === 'working' || agent?.state === 'awaiting_permission' || agent?.state === 'starting'
  // A plan waiting for approval is still that turn's to finish.
  const turnRunning = agent?.state === 'working' || agent?.state === 'awaiting_permission' || agent?.state === 'awaiting_input'

  return (
    <TranscriptDetail.Provider value={detail}>
    <TranscriptGrouping.Provider value={grouping}>
    <ThoughtsOpen.Provider value={thoughts}>
    <LiveTool.Provider value={tool.id !== null && planCalls.current.has(tool.id) ? { id: null, text: '' } : tool}>
    <LiveQuestions.Provider
      value={
        agent
          ? { thread, agent: agent.agent, questions: agent.questions ?? [], awaiting: agent.permissions.flatMap((asked) => (asked.tool_call ? [asked.tool_call] : [])) }
          : null
      }
    >
    <Page
      flush
      label="thread"
      inspectorLabel="thread details"
      inspectorWidth={480}
      columnRef={column}
      sheet={{ open: sheet, onClose: () => setSheet(false) }}
      inspector={
        (panel || sheet) && (
        <div className="flex flex-col">
          <div className="sticky top-0 z-10 flex h-12 shrink-0 items-center gap-2 border-b border-line-subtle bg-bg px-4">
            <Tabs
              label="inspector"
              tabs={[
                {
                  id: 'changes',
                  label: (
                    <>
                      Changes {totals && (totals.added > 0 || totals.removed > 0) && <Counts added={totals.added} removed={totals.removed} />}
                    </>
                  ),
                },
                { id: 'details', label: 'Details' },
                { id: 'lineage', label: 'Lineage' },
              ]}
              value={tab}
              onChange={setTab}
            />
          </div>
          {tab === 'lineage' ? (
            <div className="flex flex-col gap-6 px-5 py-5">
              {summary && (
                <InspectorSection
                  title="Lineage"
                  aside={
                    <Link to="/graph" className="text-accent-fg hover:text-fg-1">
                      View graph
                    </Link>
                  }
                >
                  <Lineage summary={summary} threads={threads.data ?? []} />
                </InspectorSection>
              )}
            </div>
          ) : tab === 'details' ? (
            <Details
              thread={thread}
              summary={summary}
              total={total}
              diff={diff.data}
              codeComments={codeComments}
              anchor={anchor}
              onChanged={reloadComments}
              onCommentLines={() => setTab('changes')}
            />
          ) : (
            <div className="px-4 py-4">
              <Unwritten thread={thread} />
              <DiffPanel thread={thread} comments={comments.data ?? []} onCommented={reloadComments} agent={agent?.agent} wide />
            </div>
          )}
        </div>
        )
      }
    >
      <ThreadHeader
        thread={thread}
        summary={summary}
        people={people}
        changes={totals}
        panelOpen={panel && desktop}
        waiting={agent?.state === 'awaiting_input' || agent?.state === 'awaiting_permission'}
        detail={detail}
        onDetail={setDetail}
        grouping={grouping}
        onGrouping={setGrouping}
        onChanges={showChanges}
        onDetails={() => {
          setTab('details')
          setPanel(true)
          setSheet(window.matchMedia('(max-width: 1023px)').matches)
        }}
      />
      <div className="mx-auto flex w-full max-w-[720px] flex-1 flex-col px-4 md:px-6">
        {summary?.cloud && (
          <div className="pt-3">
            <CloudSession thread={summary} />
          </div>
        )}
        <div ref={list} className="relative" style={{ height: virtual.getTotalSize() }} data-testid="history">
          {virtual.getVirtualItems().map((item) => {
            const row = rows[item.index]
            return (
              <div
                key={row.key}
                data-index={item.index}
                ref={virtual.measureElement}
                // What a fork inherited, once shown, reads as the source's rather than its own.
                className={cx('absolute left-0 w-full', row.kind !== 'origin' && originOf(row) === 'inherited' && 'opacity-60')}
                style={{ transform: `translateY(${item.start - virtual.options.scrollMargin}px)` }}
              >
                <TranscriptRow
                  row={row}
                  running={turnRunning && item.index === rows.length - 1}
                  onUnfold={() => setUnfolded(thread)}
                  {...entryProps}
                />
              </div>
            )
          })}
        </div>
        <div ref={below} className="flex flex-col gap-3 pt-2 pb-4">
          {agent?.permissions.map((asked, index) => (
            <PermissionCard key={asked.id} thread={thread} asked={asked} agent={agent.agent} first={index === 0} />
          ))}
          {agent?.state === 'awaiting_input' && <PlanCard thread={thread} preset={preset} onPreset={setPreset} />}
          {agent && <AgentTail status={agent} toolOutput={tool.id !== null && planCalls.current.has(tool.id) ? '' : tool.text} onCancel={() => void api.cancelTurn(thread).catch((failure: unknown) => toast(message(failure), 'danger'))} />}
        </div>
        <div className="sticky bottom-0 z-10 mt-auto bg-bg pt-1 pb-3 before:pointer-events-none before:absolute before:inset-x-0 before:-top-6 before:h-6 before:bg-gradient-to-t before:from-bg before:to-transparent md:pb-5">
          {files && files.length > 0 && totals && !busy && (
            <ChangesBar thread={thread} files={files.length} totals={totals} forked={Boolean(summary?.forked_from)} forge={forgeOf(summary?.repo ?? '')} onReview={showChanges} />
          )}
          <AgentDock
            thread={thread}
            status={agent}
            preset={preset}
            onPreset={setPreset}
            draft={draft}
            onDraft={setDraft}
            onSent={() => {
              following.current = true
              toEnd()
            }}
            people={people}
          />
        </div>
      </div>
    </Page>
    </LiveQuestions.Provider>
    </LiveTool.Provider>
    </ThoughtsOpen.Provider>
    </TranscriptGrouping.Provider>
    </TranscriptDetail.Provider>
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
  const { hasNextPage, isFetching, fetchNextPage } = pages
  const loaded = pages.data?.pages
  // Whenever a fetch settles with more to come, whichever fetch it was: a refetch,
  // such as the one subscribing to the thread starts, cancels the next page's
  // fetch and brings back only the pages it had, which left a thread at its first page.
  useEffect(() => {
    if (hasNextPage && !isFetching) void fetchNextPage()
  }, [hasNextPage, isFetching, fetchNextPage, loaded?.length])
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
