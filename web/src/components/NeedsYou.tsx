import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { type ReactNode, useState } from 'react'
import { APPROVED } from '../agentState'
import { useAgentStatus } from '../agentStatus'
import { api, short, type MrSummary, type ThreadSummary } from '../api'
import { useTopic } from '../bus'
import { useNow } from '../now'
import { keys, message } from '../query'
import { sections } from '../sections'
import { hosted } from '../served'
import { Button, Dot, type DotTone, EmptyState, Skeleton, buttonClass, toast } from '../ui'
import { useInWorkspace } from '../workspaceFilter'
import { Markdown } from './Markdown'
import { Page } from './Page'

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
  const details = useQuery({ queryKey: keys.mr(mr.id), queryFn: () => api.mergeRequest(mr.id) })
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
  const where = thread.location === 'remote' ? 'on the server' : thread.location === 'moved' ? 'on another device' : `on ${device}`
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
    <Card tone={thread.agent_state === 'error' ? 'danger' : 'warn'} kind={thread.agent_state === 'error' ? 'Agent stopped' : 'Waiting on you'} label={label}>
      <span className="text-base font-medium">{title}</span>
      <span className="font-mono text-xs text-fg-2">{meta}</span>
      <Link to="/t/$id" params={{ id: thread.id }} className={buttonClass('secondary', 'touch')}>
        Open
      </Link>
    </Card>
  )
}

/** What waits on the person, each answerable in place: a phone's first screen (ADR 0028, the Phone board). */
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
  return (
    <Page flush label="needs you">
      <section aria-label="waiting on you" className="mx-auto flex w-full max-w-2xl flex-col gap-3 px-4 py-4">
        <h1 className="sr-only md:not-sr-only md:text-xl md:font-medium">Needs you</h1>
        {threads.isPending && <Skeleton className="h-24" />}
        {threads.error && <p className="text-danger">{message(threads.error)}</p>}
        {threads.isSuccess && waiting.length === 0 && replies.length === 0 && (
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
