import { useQuery, useQueryClient } from '@tanstack/react-query'
import { LanNotifyBanner } from './LanPhones'
import { Link, useNavigate } from '@tanstack/react-router'
import { type ReactNode, useCallback, useEffect, useState } from 'react'
import { APPROVED } from '../agentState'
import { useAgentStatus } from '../agentStatus'
import { api, short, type AgentStatus, type MrSummary, type ThreadSummary } from '../api'
import { useTopic } from '../bus'
import { useNow } from '../now'
import { projectName } from '../projects'
import { keys, message } from '../query'
import { sections } from '../sections'
import { hosted } from '../served'
import { usePhone } from '../shell'
import { useShortcut } from '../shortcuts'
import { WAIT_KINDS, type WaitKind, waitKind } from '../threadLook'
import { ago } from '../time'
import { Button, Dot, type DotTone, EmptyState, Icon, Kbd, SectionLabel, Skeleton, buttonClass, cx, toast } from '../ui'
import { useInWorkspace } from '../workspaceFilter'
import { PermissionCard, SignInCard } from './Agent'
import { Markdown } from './Markdown'
import { Page } from './Page'
import { PendingApprovals } from './Sandbox'
import { useApprovalsWaiting } from '../sandbox'
import { AnswerForm } from './ToolSteps'

/** A card on the phone's first screen: what it is, what it asks, and how to answer (the Phone board). */
function Card({ tone, kind, label, children }: { tone: DotTone; kind: string; label: string; children: ReactNode }) {
  return (
    <li role="group" aria-label={label} className="flex flex-col gap-2.5 rounded-md border border-line bg-elevated p-3.5">
      <span className="inline-flex items-center gap-1.5 font-mono text-2xs font-medium tracking-[0.04em] text-fg-2 uppercase">
        <Dot tone={tone} />
        {kind}
      </span>
      {children}
    </li>
  )
}

/** A merge request waiting on the person: its guide, how much is seen, and approving it. */
function ReviewCard({ thread, mr }: { thread: ThreadSummary; mr: MrSummary }) {
  const queries = useQueryClient()
  const details = useQuery({ queryKey: keys.mrBrief(mr.id), queryFn: () => api.mergeRequest(mr.id, {}, true) })
  const files = details.data?.files ?? []
  const seen = files.filter((file) => file.seen).length
  const guide = details.data?.guide
  return (
    <Card tone="accent" kind="Merge request" label={`${thread.title ?? 'Untitled thread'} needs you`}>
      <span className="text-base font-medium">{mr.title}</span>
      <span className="font-mono text-xs text-fg-2">
        {thread.owner && `${thread.owner} · `}
        <span data-volatile>
          {short(mr.fork)} → {short(mr.target)}
        </span>
      </span>
      {guide && (
        <div className="flex flex-col gap-1.5 rounded-sm bg-surface-1 px-3 py-2.5">
          <span className="font-mono text-2xs font-medium tracking-[0.04em] text-fg-2 uppercase">Change guide</span>
          <Markdown text={guide.text} className="line-clamp-4 text-sm" />
        </div>
      )}
      {files.length > 0 && (
        <div className="flex items-center gap-2 text-sm text-fg-2">
          <span className="h-1 flex-1 overflow-hidden rounded-sm bg-surface-2" aria-hidden="true">
            <span className="block h-full bg-accent" style={{ width: `${(100 * seen) / files.length}%` }} />
          </span>
          <span className="font-mono text-xs">
            {seen}/{files.length} files seen
          </span>
        </div>
      )}
      <div className="grid grid-cols-2 gap-2">
        <Link to="/mr/$id" params={{ id: mr.id }} className={buttonClass('secondary', 'touch')}>
          Review files
        </Link>
        <Button
          variant="primary"
          size="touch"
          onClick={() =>
            void api.approve(mr.id).then(
              () => {
                toast(`Merged "${mr.title}"`, 'ok')
                return queries.invalidateQueries({ queryKey: keys.mrs })
              },
              (failure: unknown) => toast(message(failure), 'danger'),
            )
          }
        >
          Approve
        </Button>
      </div>
    </Card>
  )
}

function useDevice(): string {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  return status.data?.servers?.[0]?.name ?? 'this machine'
}

/** An agent waiting on the person: a permission or a plan, answered in place (ADR 0028). */
function AgentCard({ thread, review }: { thread: ThreadSummary; review: MrSummary | undefined }) {
  const status = useAgentStatus(thread.id)
  const device = useDevice()
  const [error, setError] = useState<string>()
  const act = (action: Promise<unknown>) => {
    setError(undefined)
    action.catch((failure: unknown) => setError(message(failure)))
  }
  const title = thread.title ?? 'Untitled thread'
  const label = `${title} needs you`
  const asked = status?.permissions[0]
  if (thread.review && review && !asked && status?.state !== 'awaiting_input') return <ReviewCard thread={thread} mr={review} />
  const where =
    thread.location === 'remote'
      ? `on ${thread.machine ?? 'the server'}`
      : thread.location === 'moved'
        ? `on ${thread.machine ?? 'another device'}`
        : `on ${device}`
  const meta = [status?.agent ?? thread.agent, title, where].filter(Boolean).join(' · ')
  if (asked) {
    const [first, second, ...rest] = asked.options
    return (
      <Card tone="warn" kind="Agent is asking" label={label}>
        <code className="font-mono text-base font-medium break-all">{asked.title}</code>
        <span className="font-mono text-xs text-fg-2">{meta}</span>
        <div className="grid grid-cols-2 gap-2">
          {first && (
            <Button variant="primary" size="touch" onClick={() => act(api.answer(thread.id, asked.id, first[0]))}>
              {first[1]}
            </Button>
          )}
          {second && (
            <Button size="touch" onClick={() => act(api.answer(thread.id, asked.id, second[0]))}>
              {second[1]}
            </Button>
          )}
        </div>
        {rest.map(([option, text]) => (
          <Button key={option} variant="ghost" size="touch" onClick={() => act(api.answer(thread.id, asked.id, option))}>
            {text}
          </Button>
        ))}
        {error && <p className="text-xs text-danger">{error}</p>}
      </Card>
    )
  }
  if (status?.state === 'awaiting_input')
    return (
      <Card tone="warn" kind="Plan to approve" label={label}>
        <span className="text-base font-medium">{title}</span>
        <span className="font-mono text-xs text-fg-2">{meta}</span>
        <div className="grid grid-cols-2 gap-2">
          <Link to="/t/$id" params={{ id: thread.id }} className={buttonClass('secondary', 'touch')}>
            Read the plan
          </Link>
          <Button variant="primary" size="touch" onClick={() => act(api.prompt(thread.id, APPROVED))}>
            Approve plan
          </Button>
        </div>
        {error && <p className="text-xs text-danger">{error}</p>}
      </Card>
    )
  return (
    <Card
      tone={thread.agent_state === 'error' ? 'danger' : 'warn'}
      kind={thread.agent_state === 'error' ? 'Agent stopped' : 'Waiting on you'}
      label={label}
    >
      <span className="text-base font-medium">{title}</span>
      <span className="font-mono text-xs text-fg-2">{meta}</span>
      <Link to="/t/$id" params={{ id: thread.id }} className={buttonClass('secondary', 'touch')}>
        Open
      </Link>
    </Card>
  )
}

/** Hands a thread's agent status to the queue, which orders the threads by what they wait for. */
function StatusOf({ thread, onStatus }: { thread: string; onStatus: (thread: string, status: AgentStatus | null) => void }) {
  const status = useAgentStatus(thread)
  useEffect(() => onStatus(thread, status), [thread, status, onStatus])
  return null
}

interface Waiting {
  thread: ThreadSummary
  kind: WaitKind
  status: AgentStatus | null
}

const ORDER: WaitKind[] = ['sign-in', 'question', 'plan', 'permission', 'review', 'stopped']

/** One waiting thread in the queue's list: its kind's icon, its title, what it asks in a line, and how long it has waited. */
function QueueRow({
  item,
  review,
  selected,
  onSelect,
  now,
}: {
  item: Waiting
  review: MrSummary | undefined
  selected: boolean
  onSelect: () => void
  now: number
}) {
  const { thread, kind, status } = item
  const asked = status?.permissions[0]
  const what =
    kind === 'sign-in'
      ? 'Needs signing in'
      : kind === 'question'
        ? status?.questions[0]?.message
        : kind === 'permission'
          ? asked?.title
          : kind === 'plan'
            ? 'A plan to approve'
            : kind === 'review'
              ? review && `Merge request: ${review.title}`
              : (status?.error ?? 'Stopped with an error')
  return (
    <li>
      <button
        type="button"
        aria-current={selected ? 'true' : undefined}
        onClick={onSelect}
        className={cx(
          'flex w-full items-start gap-3 rounded-sm px-2.5 py-2 text-left',
          selected ? 'bg-accent-tint shadow-[inset_2px_0_0_var(--accent)]' : 'hover:bg-surface-2',
        )}
      >
        <Icon
          name={WAIT_KINDS[kind].icon}
          className={cx('mt-0.5 h-3.5 w-3.5 shrink-0', kind === 'stopped' ? 'text-danger' : 'text-accent-fg')}
        />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className={cx('truncate', selected && 'font-medium')}>{thread.title ?? 'Untitled thread'}</span>
          {what && <span className="truncate text-xs text-fg-2">{what}</span>}
        </span>
        <span className="shrink-0 font-mono text-2xs whitespace-nowrap text-fg-3" data-volatile>
          {ago(thread.last_ms, now).replace('just now', 'now')}
        </span>
      </button>
    </li>
  )
}

/** A merge request waiting on the person, in the queue: how much is seen, then approving it or reviewing the files. */
function ReviewDetail({ mr, approve }: { mr: MrSummary; approve: () => void }) {
  const details = useQuery({ queryKey: keys.mrBrief(mr.id), queryFn: () => api.mergeRequest(mr.id, {}, true) })
  const files = details.data?.files ?? []
  const seen = files.filter((file) => file.seen).length
  const guide = details.data?.guide
  return (
    <div className="flex flex-col gap-3">
      {mr.summary && <Markdown text={mr.summary} className="text-sm" />}
      {guide && (
        <div className="flex flex-col gap-1.5 rounded-sm bg-surface-1 px-3 py-2.5">
          <span className="font-mono text-2xs font-medium tracking-[0.04em] text-fg-2 uppercase">Change guide</span>
          <Markdown text={guide.text} className="line-clamp-6 text-sm" />
        </div>
      )}
      {files.length > 0 && (
        <div className="flex items-center gap-2 text-sm text-fg-2">
          <span className="h-1 flex-1 overflow-hidden rounded-sm bg-surface-2" aria-hidden="true">
            <span className="block h-full bg-accent" style={{ width: `${(100 * seen) / files.length}%` }} />
          </span>
          <span className="font-mono text-xs">
            {seen}/{files.length} files seen
          </span>
        </div>
      )}
      <span className="flex flex-wrap gap-1.5">
        <Button variant="primary" onClick={approve}>
          Approve and merge <Kbd onAccent>A</Kbd>
        </Button>
        <Link to="/mr/$id" params={{ id: mr.id }} className={buttonClass('secondary')}>
          Review files
        </Link>
      </span>
    </div>
  )
}

/** The item chosen in the queue, answered in place with the same cards the thread shows. */
function QueueDetail({
  item,
  review,
  next,
  approve,
}: {
  item: Waiting
  review: MrSummary | undefined
  next: Waiting | undefined
  approve: () => void
}) {
  const { thread, kind, status } = item
  const now = useNow()
  const named = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const title = thread.title ?? 'Untitled thread'
  const asked = status?.permissions[0]
  const question = status?.questions[0]
  const agent = status?.agent ?? thread.agent ?? 'The agent'
  const project = named.data?.find((each) => each.repo === thread.repo)?.name ?? projectName(thread.repo)
  const said = {
    'sign-in': `${agent} needs signing in`,
    question: `${agent} asks`,
    plan: `${agent} wrote a plan`,
    permission: `${agent} asks to run`,
    review: 'A merge request waits for your review',
    stopped: `${agent} stopped`,
  }[kind]
  return (
    <div role="group" aria-label={`${review?.title ?? title} needs you`} className="flex max-w-[760px] flex-col gap-4">
      <span className="flex min-w-0 items-center gap-2 text-xs text-fg-2">
        <Dot tone={kind === 'stopped' ? 'danger' : 'accent'} />
        <span className="truncate">
          {said} {kind === 'review' ? 'into' : 'in'} <span className="text-fg-1">{title}</span> ·{' '}
          <span className="font-mono">{project}</span> · <span data-volatile>{ago(thread.last_ms, now)}</span>
        </span>
      </span>
      {kind === 'review' && review && <h2 className="text-lg font-medium">{review.title}</h2>}
      {kind === 'sign-in' && status?.sign_in && <SignInCard thread={thread.id} asked={status.sign_in} agent={agent} />}
      {kind === 'question' && question && <AnswerForm key={question.id} thread={thread.id} asked={question} />}
      {(kind === 'permission' || kind === 'plan') && asked && (
        <PermissionCard key={asked.id} thread={thread.id} asked={asked} agent={agent} first />
      )}
      {kind === 'plan' && !asked && (
        <span className="flex flex-wrap gap-1.5">
          <Button variant="primary" onClick={approve}>
            Approve plan <Kbd onAccent>A</Kbd>
          </Button>
          <Link to="/t/$id" params={{ id: thread.id }} className={buttonClass('secondary')}>
            Read the plan
          </Link>
        </span>
      )}
      {kind === 'review' && review && <ReviewDetail mr={review} approve={approve} />}
      {kind === 'stopped' && <p className="text-sm text-danger">{status?.error ?? 'The agent stopped with an error.'}</p>}
      <span className="flex flex-wrap items-center gap-3 text-xs text-fg-2">
        <Link to="/t/$id" params={{ id: thread.id }} className={buttonClass('ghost', 'sm')}>
          <Icon name="external" className="h-3.5 w-3.5" />
          Open thread <Kbd>O</Kbd>
        </Link>
        {next && (
          <span className="ml-auto truncate text-fg-3">
            Next: {WAIT_KINDS[next.kind].word} in <i>{next.thread.title ?? 'Untitled thread'}</i>
          </span>
        )}
      </span>
    </div>
  )
}

/**
 * The queue (the UI review): what waits on the person, grouped by kind on the left and
 * answered on the right; the keyboard moves through it, and once one is answered the
 * next takes its place.
 */
function Queue({
  waiting,
  openInto,
  replies,
}: {
  waiting: ThreadSummary[]
  openInto: (thread: ThreadSummary) => MrSummary | undefined
  replies: ReactNode[]
}) {
  const queries = useQueryClient()
  const navigate = useNavigate()
  const now = useNow()
  const [statuses, setStatuses] = useState<Record<string, AgentStatus | null>>({})
  const onStatus = useCallback(
    (thread: string, status: AgentStatus | null) =>
      setStatuses((had) => (had[thread] === status ? had : { ...had, [thread]: status })),
    [],
  )
  const items: Waiting[] = waiting
    .map((thread) => ({ thread, status: statuses[thread.id] ?? null, kind: waitKind(thread, statuses[thread.id] ?? null) }))
    .sort((a, b) => ORDER.indexOf(a.kind) - ORDER.indexOf(b.kind) || a.thread.last_ms - b.thread.last_ms)
  // Where the person was too, so once what they chose is answered and gone, the one after it, now in its place, is next.
  const [chosen, setChosen] = useState<{ id: string; at: number }>()
  const found = items.findIndex((item) => item.thread.id === chosen?.id)
  const index = found >= 0 ? found : Math.min(chosen?.at ?? 0, items.length - 1)
  const item = items[index]
  const choose = (to: number) => {
    const at = Math.max(0, Math.min(items.length - 1, to))
    const target = items[at]
    if (target) setChosen({ id: target.thread.id, at })
  }
  const [error, setError] = useState<string>()
  const act = (action: Promise<unknown>) => {
    setError(undefined)
    action.catch((failure: unknown) => setError(message(failure)))
  }
  const review = item && item.kind === 'review' ? openInto(item.thread) : undefined
  const approve = () => {
    if (!item) return
    if (item.kind === 'plan' && !item.status?.permissions[0]) act(api.prompt(item.thread.id, APPROVED))
    if (item.kind === 'review' && review)
      act(
        api.approve(review.id).then(() => {
          toast(`Merged "${review.title}"`, 'ok')
          return queries.invalidateQueries({ queryKey: keys.mrs })
        }),
      )
  }
  useShortcut('j', 'The next item that needs you', () => choose(index + 1))
  useShortcut('k', 'The item before', () => choose(index - 1))
  useShortcut('a', 'Approve the plan or merge request', approve)
  useShortcut('o', 'Open its thread', () => item && void navigate({ to: '/t/$id', params: { id: item.thread.id } }))
  const oldest = waiting.length > 0 ? Math.min(...waiting.map((thread) => thread.last_ms)) : now
  return (
    <section aria-label="waiting on you" className="flex min-h-0 flex-1">
      {waiting.map((thread) => (
        <StatusOf key={thread.id} thread={thread.id} onStatus={onStatus} />
      ))}
      <div className="flex w-[380px] shrink-0 flex-col gap-1 overflow-y-auto border-r border-line px-2 py-2.5">
        <span className="px-2.5 pb-1 text-2xs text-fg-2">
          {waiting.length} waiting · oldest <span data-volatile>{ago(oldest, now)}</span>
        </span>
        {ORDER.filter((kind) => items.some((each) => each.kind === kind)).map((kind) => (
          <section key={kind} aria-label={WAIT_KINDS[kind].heading}>
            <SectionLabel className="px-2.5">{WAIT_KINDS[kind].heading}</SectionLabel>
            <ul>
              {items
                .filter((each) => each.kind === kind)
                .map((each) => (
                  <QueueRow
                    key={each.thread.id}
                    item={each}
                    review={each.kind === 'review' ? openInto(each.thread) : undefined}
                    selected={each === item}
                    onSelect={() => choose(items.indexOf(each))}
                    now={now}
                  />
                ))}
            </ul>
          </section>
        ))}
        {replies.length > 0 && (
          <section aria-label="Replies">
            <SectionLabel className="px-2.5">Replies</SectionLabel>
            <ul>{replies}</ul>
          </section>
        )}
      </div>
      <div className="min-w-0 flex-1 overflow-y-auto px-8 py-6">
        {item && <QueueDetail key={item.thread.id} item={item} review={review} next={items[index + 1]} approve={approve} />}
        {error && <p className="mt-3 text-xs text-danger">{error}</p>}
      </div>
    </section>
  )
}

/** The keys the queue answers to, in its header. */
function QueueKeys() {
  return (
    <span className="hidden items-center gap-1.5 text-2xs text-fg-2 md:flex">
      <Kbd>J</Kbd>
      <Kbd>K</Kbd>
      move
      <span className="ml-2">
        <Kbd>1</Kbd>–<Kbd>9</Kbd>
      </span>
      answer
      <span className="ml-2">
        <Kbd>⏎</Kbd>
      </span>
      allow
      <span className="ml-2">
        <Kbd>A</Kbd>
      </span>
      approve
      <span className="ml-2">
        <Kbd>O</Kbd>
      </span>
      open thread
    </span>
  )
}

/** What waits on the person, each answerable in place: a queue on a desktop, a phone's first screen of cards (ADR 0028, the Phone board). */
export function NeedsYou() {
  useTopic('threads')
  useTopic('mrs')
  useTopic('inbox')
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const mrs = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests })
  const inbox = useQuery({ queryKey: keys.inbox(false), queryFn: () => api.inbox(false), retry: false })
  const shown = useInWorkspace()
  const waiting = sections((threads.data ?? []).filter(shown), useNow()).needs_you
  const openInto = (thread: ThreadSummary) => (mrs.data ?? []).find((mr) => mr.status === 'open' && mr.target === thread.id)
  const titleOf = (thread: string) => threads.data?.find((summary) => summary.id === thread)?.title ?? 'Untitled thread'
  const replies = (inbox.data ?? []).filter((entry) => !entry.read && ['comment', 'reply', 'mention'].includes(entry.kind))
  const phone = usePhone()
  const approvals = useApprovalsWaiting()
  // Plugin calls to approve show as cards, above the rest.
  if (!phone && waiting.length > 0 && approvals === 0)
    return (
      <Page flush label="needs you" title="Needs you" actions={<QueueKeys />}>
        <Queue
          waiting={waiting}
          openInto={openInto}
          replies={replies.map((entry) => (
            <li key={`${entry.server ?? ''}:${entry.id}`}>
              <Link
                to="/t/$id"
                params={{ id: entry.thread }}
                className="flex items-start gap-3 rounded-sm px-2.5 py-2 hover:bg-surface-2"
              >
                <Icon name="comment" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-info" />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate">{titleOf(entry.thread)}</span>
                  <span className="truncate text-xs text-fg-2">
                    {entry.actor}: {entry.text}
                  </span>
                </span>
              </Link>
            </li>
          ))}
        />
      </Page>
    )
  return (
    <Page flush label="needs you" title="Needs you">
      <LanNotifyBanner />
      <section aria-label="waiting on you" className="mx-auto flex w-full max-w-2xl flex-col gap-3 px-4 py-4">
        {threads.isPending && <Skeleton className="h-24" />}
        {threads.error && <p className="text-danger">{message(threads.error)}</p>}
        <PendingApprovals titleOf={titleOf} />
        {threads.isSuccess && waiting.length === 0 && replies.length === 0 && approvals === 0 && (
          <EmptyState title="Nothing waits on you">Agents that ask for something, reviews and replies show here.</EmptyState>
        )}
        <ul className="flex flex-col gap-3">
          {waiting.map((thread) => (
            <AgentCard key={thread.id} thread={thread} review={openInto(thread)} />
          ))}
          {replies.map((entry) => (
            <Card key={`${entry.server ?? ''}:${entry.id}`} tone="info" kind="Reply" label={`${entry.actor} replied`}>
              <span className="text-sm">
                <b className="font-medium">{entry.actor}</b>: {entry.text}
              </span>
              <span className="font-mono text-xs text-fg-2">{titleOf(entry.thread)}</span>
              <Link to="/t/$id" params={{ id: entry.thread }} className={buttonClass('secondary', 'touch')}>
                Reply
              </Link>
            </Card>
          ))}
        </ul>
      </section>
    </Page>
  )
}
