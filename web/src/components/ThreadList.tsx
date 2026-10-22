import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { useAgentStatus } from '../agentStatus'
import { api, short, type MrSummary, type ThreadSummary } from '../api'
import { useTopic } from '../bus'
import { useNow } from '../now'
import { projectName, projects } from '../projects'
import { keys, message } from '../query'
import { sections } from '../sections'
import { threadTone, threadWhere } from '../threadLook'
import { ago } from '../time'
import { Dot, EmptyState, Icon, SectionLabel, Skeleton, Tabs, buttonClass, cx } from '../ui'
import { useInWorkspace } from '../workspaceFilter'
import { Page } from './Page'

type Filter = 'all' | 'mine' | 'shared' | 'elsewhere'

const FILTERS: { id: Filter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'mine', label: 'Mine' },
  { id: 'shared', label: 'Shared with me' },
  { id: 'elsewhere', label: 'On other devices' },
]

/** What a needs-you card says it is, before its title. */
function NeedsYouCard({ thread, review }: { thread: ThreadSummary; review: MrSummary | undefined }) {
  const status = useAgentStatus(thread.id)
  const asked = status?.permissions[0]
  const project = projectName(thread.repo || 'no remote')
  const card = 'flex flex-col gap-1.5 rounded-md border border-line bg-elevated px-3.5 py-3 text-fg-1 hover:border-line-strong'
  const kind = (tone: 'warn' | 'accent' | 'danger' | 'info', label: string) => (
    <span className="inline-flex items-center gap-1.5 font-mono text-2xs font-medium tracking-[0.04em] text-fg-2 uppercase">
      <Dot tone={tone} />
      {label}
    </span>
  )
  if (thread.review && review) {
    return (
      <Link to="/mr/$id" params={{ id: review.id }} className={card} aria-label={`Merge request: ${review.title}`}>
        {kind('accent', 'Merge request')}
        <span className="font-medium">{review.title}</span>
        <span className="font-mono text-2xs text-fg-2">
          <span data-volatile>
            {short(review.fork)} → {short(review.target)}
          </span>{' '}
          · into {thread.title ?? 'Untitled thread'}
        </span>
      </Link>
    )
  }
  return (
    <Link to="/t/$id" params={{ id: thread.id }} className={card} aria-label={`${thread.title ?? 'Untitled thread'} needs you`}>
      {thread.agent_state === 'error'
        ? kind('danger', 'Agent stopped')
        : thread.agent_state === 'awaiting_input'
          ? kind('warn', 'Plan to approve')
          : kind('warn', 'Agent is asking')}
      <span className="truncate font-medium">
        {asked ? (
          <>
            Allow <code className="font-mono text-xs">{asked.title}</code>?
          </>
        ) : (
          (thread.title ?? 'Untitled thread')
        )}
      </span>
      <span className="truncate font-mono text-2xs text-fg-2">
        {[thread.agent, asked ? thread.title : null, project].filter(Boolean).join(' · ')}
      </span>
    </Link>
  )
}

function ThreadRow({
  thread,
  now,
  review,
  recovered,
}: {
  thread: ThreadSummary
  now: number
  review: MrSummary | undefined
  /** It holds this machine's work after its copy differed from the server's (ADR 0009). */
  recovered: boolean
}) {
  const where = recovered ? { label: 'Recovered', tone: 'danger' as const } : threadWhere(thread)
  const meta = [
    thread.agent,
    `${thread.messages} ${thread.messages === 1 ? 'message' : 'messages'}`,
    `${thread.edits} ${thread.edits === 1 ? 'edit' : 'edits'}`,
    thread.collaborators.length > 0 && `shared with ${thread.collaborators.join(', ')}`,
    review && 'MR open',
  ].filter(Boolean)
  return (
    <li
      className={cx(
        'grid grid-cols-[16px_minmax(0,1fr)_auto] items-center gap-3 border-b border-line-subtle px-3 py-2 last:border-b-0 hover:bg-surface-2 md:grid-cols-[16px_minmax(0,1fr)_150px_90px_110px]',
        thread.forked_from && 'md:pl-9',
      )}
    >
      {thread.forked_from ? <Icon name="fork" label="Fork" className="text-fg-2" /> : <Dot tone={threadTone(thread)} />}
      <div className="flex min-w-0 flex-col gap-0.5">
        <Link to="/t/$id" params={{ id: thread.id }} className="truncate font-medium text-fg-1 hover:text-accent-fg">
          {thread.title ?? 'Untitled thread'}
        </Link>
        <span className="truncate font-mono text-2xs text-fg-2">
          <Link to="/t/$id" params={{ id: thread.id }} className="hover:text-fg-1" data-volatile>
            {short(thread.id)}
          </Link>
          {thread.forked_from && (
            <>
              {' · fork of '}
              <span data-volatile>{short(thread.forked_from)}</span>
            </>
          )}
          {meta.length > 0 && ` · ${meta.join(' · ')}`}
        </span>
      </div>
      <span className="hidden items-center gap-1.5 text-xs md:inline-flex">
        <Dot tone={where.tone} />
        {where.label}
      </span>
      <span className="hidden font-mono text-2xs text-fg-2 md:inline" data-volatile>
        {ago(thread.last_ms, now)}
      </span>
      <span className="flex justify-end">
        {recovered ? (
          <Link to="/t/$id/reconcile" params={{ id: thread.id }} className={buttonClass('secondary')}>
            Review
          </Link>
        ) : review ? (
          <Link to="/mr/$id" params={{ id: review.id }} className={buttonClass('secondary')}>
            Review
          </Link>
        ) : (
          <Link to="/t/$id" params={{ id: thread.id }} className={buttonClass('secondary')}>
            Open
          </Link>
        )}
      </span>
    </li>
  )
}

/** Each fork right after the thread it came from, when that is in the list too. */
function forksUnderSources(threads: ThreadSummary[]): ThreadSummary[] {
  const listed = new Set(threads.map((thread) => thread.id))
  const forksOf = (source: string) => threads.filter((thread) => thread.forked_from === source)
  const out: ThreadSummary[] = []
  const add = (thread: ThreadSummary) => {
    out.push(thread)
    for (const fork of forksOf(thread.id)) add(fork)
  }
  for (const thread of threads) if (!thread.forked_from || !listed.has(thread.forked_from)) add(thread)
  return out
}

/**
 * The workspace home (ADR 0035, the Threads board): what needs the person,
 * then every thread by project.
 */
export function ThreadList() {
  useTopic('threads')
  useTopic('mrs')
  const now = useNow()
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const mrs = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests })
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const named = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const nameOf = (repo: string) => named.data?.find((project) => project.repo === repo)?.name ?? projectName(repo)
  const [filter, setFilter] = useState<Filter>('all')
  const shown = useInWorkspace()
  const me = status.data?.servers?.[0]?.user ?? ''
  const all = (threads.data ?? []).filter(shown)
  const waiting = sections(all, now).needs_you
  const openInto = (thread: ThreadSummary) =>
    (mrs.data ?? []).find((mr) => mr.status === 'open' && (mr.target === thread.id || mr.fork === thread.id))
  const filtered = all.filter((thread) => {
    if (filter === 'mine') return thread.owner === '' || thread.owner === me
    if (filter === 'shared') return thread.owner !== '' && thread.owner !== me
    if (filter === 'elsewhere') return thread.location !== 'here'
    return true
  })
  const count = (wanted: (thread: ThreadSummary) => boolean, one: string, many: string) => {
    const counted = waiting.filter(wanted).length
    return counted > 0 && `${counted} ${counted === 1 ? one : many}`
  }
  const kinds = [
    count((thread) => thread.review, 'approval', 'approvals'),
    count((thread) => thread.agent_state === 'awaiting_permission', 'permission', 'permissions'),
    count((thread) => thread.agent_state === 'awaiting_input', 'plan', 'plans'),
    count((thread) => thread.agent_state === 'error', 'error', 'errors'),
  ].filter(Boolean)
  return (
    <Page flush label="threads">
      {waiting.length > 0 && (
        <section aria-labelledby="needs-you" className="flex flex-col gap-2.5 px-4 pt-5 pb-2 md:px-6">
          <div className="flex items-baseline gap-3">
            <h2 id="needs-you" className="text-lg font-medium">
              Needs you
            </h2>
            <span className="text-xs text-fg-2">{kinds.join(' · ')}</span>
          </div>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-2.5">
            {waiting.map((thread) => (
              <NeedsYouCard key={thread.id} thread={thread} review={openInto(thread)} />
            ))}
          </div>
        </section>
      )}
      <div className="mt-3 flex min-h-10 flex-wrap items-center gap-2 border-y border-line px-4 py-1.5 md:px-6">
        <Tabs label="which threads" tabs={FILTERS} value={filter} onChange={setFilter} />
        <span className="ml-auto flex items-center gap-2">
          <Link to="/graph" className={buttonClass('ghost')}>
            <Icon name="fork" />
            Graph
          </Link>
          <Link to="/new" className={buttonClass('primary')}>
            <Icon name="plus" />
            New thread
          </Link>
        </span>
      </div>
      <div data-testid="threads" className="px-4 pt-2 pb-6 md:px-6">
        {threads.error && <p className="py-3 text-danger">{message(threads.error)}</p>}
        {threads.isPending && <Skeleton className="mt-3 h-24" />}
        {threads.isSuccess && filtered.length === 0 && (
          <div className="mt-4">
            <EmptyState title="Start a thread, or record one">
              Run <code className="font-mono">silt claude</code> in a repository, or start an agent from New thread.
            </EmptyState>
          </div>
        )}
        {projects(filtered).map(([repo, inRepo]) => (
          <section key={repo} aria-label={`${nameOf(repo)} threads`}>
            <SectionLabel className="px-0 pt-3">{nameOf(repo)}</SectionLabel>
            <ul className="overflow-hidden rounded-md border border-line bg-elevated">
              {forksUnderSources(inRepo).map((thread) => (
                <ThreadRow
                  key={thread.id}
                  thread={thread}
                  now={now}
                  review={openInto(thread)}
                  recovered={(status.data?.recoveries ?? []).some((recovery) => recovery.recovery === thread.id)}
                />
              ))}
            </ul>
          </section>
        ))}
      </div>
    </Page>
  )
}
