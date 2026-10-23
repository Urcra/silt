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
import { hosted } from '../served'
import { threadDoing, threadWhere } from '../threadLook'
import { ago } from '../time'
import { Dot, EmptyState, Icon, Skeleton, Tabs, buttonClass, cx } from '../ui'
import { useInWorkspace } from '../workspaceFilter'
import { Page } from './Page'
import { ThreadMark } from './ThreadMark'

type Filter = 'all' | 'working' | 'mine' | 'shared' | 'elsewhere'

const FILTERS: { id: Filter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'working', label: 'Working' },
  { id: 'mine', label: 'Mine' },
  { id: 'shared', label: 'Shared with me' },
  { id: 'elsewhere', label: 'On other devices' },
]

const ROW = 'flex items-center gap-2.5 rounded-sm px-2.5 py-2 text-fg-1 hover:bg-surface-2 max-md:min-h-11'

/** A thread waiting on the person: what it waits for, then where to answer it. */
function NeedsYouRow({ thread, review }: { thread: ThreadSummary; review: MrSummary | undefined }) {
  const status = useAgentStatus(thread.id)
  const asked = status?.permissions[0]
  const now = useNow()
  const what = review
    ? 'merge request to review'
    : thread.agent_state === 'error'
      ? 'agent stopped'
      : thread.agent_state === 'awaiting_input'
        ? 'plan to approve'
        : asked
          ? `asks to run ${asked.title}`
          : 'asks you'
  return (
    <li>
      {review ? (
        <Link to="/mr/$id" params={{ id: review.id }} className={ROW} aria-label={`Merge request: ${review.title}`}>
          <Dot tone="warn" />
          <span className="min-w-0 flex-1 truncate">{review.title}</span>
          <span className="truncate text-xs text-accent-fg">{what}</span>
          <span className="w-8 shrink-0 text-right font-mono text-2xs text-fg-3" data-volatile>
            {ago(thread.last_ms, now)}
          </span>
        </Link>
      ) : (
        <Link to="/t/$id" params={{ id: thread.id }} className={ROW} aria-label={`${thread.title ?? 'Untitled thread'} needs you`}>
          <Dot tone={thread.agent_state === 'error' ? 'danger' : 'warn'} />
          <span className="min-w-0 flex-1 truncate">{thread.title ?? 'Untitled thread'}</span>
          <span className="max-w-[50%] truncate font-mono text-xs text-accent-fg">{what}</span>
          <span className="w-8 shrink-0 text-right font-mono text-2xs text-fg-3" data-volatile>
            {ago(thread.last_ms, now)}
          </span>
        </Link>
      )}
    </li>
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
  const where = recovered ? 'recovered: review it' : (threadDoing(thread) ?? threadWhere(thread).label.toLowerCase())
  const meta = [
    thread.agent,
    where,
    thread.collaborators.length > 0 && `shared with ${thread.collaborators.join(', ')}`,
    review && 'merge request open',
  ].filter(Boolean)
  const to = recovered ? ('/t/$id/reconcile' as const) : ('/t/$id' as const)
  return (
    <li className={cx('group relative', thread.forked_from && 'pl-5')}>
      <div className={ROW}>
        {thread.forked_from ? <Icon name="fork" label="Fork" className="h-3.5 w-3.5 shrink-0 text-fg-2" /> : <ThreadMark thread={thread} />}
        <span className="flex min-w-0 flex-1 flex-col gap-px">
          <Link to={to} params={{ id: thread.id }} className="truncate after:absolute after:inset-0">
            {thread.title ?? 'Untitled thread'}
          </Link>
          <span className="relative truncate font-mono text-2xs text-fg-2">
            <Link to="/t/$id" params={{ id: thread.id }} className="hover:text-fg-1" data-volatile>
              {short(thread.id)}
            </Link>
            {thread.forked_from && (
              <>
                {' · fork of '}
                <span data-volatile>{short(thread.forked_from)}</span>
              </>
            )}
            {` · ${thread.messages} ${thread.messages === 1 ? 'message' : 'messages'} · ${thread.edits} ${thread.edits === 1 ? 'edit' : 'edits'}`}
          </span>
        </span>
        <span className={cx('hidden max-w-[40%] truncate text-xs md:inline', recovered || review ? 'text-accent-fg' : 'text-fg-2')}>{meta.join(' · ')}</span>
        <span className="w-8 shrink-0 text-right font-mono text-2xs text-fg-3" data-volatile>
          {ago(thread.last_ms, now)}
        </span>
      </div>
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
  for (const thread of [...threads].sort((a, b) => b.last_ms - a.last_ms)) if (!thread.forked_from || !listed.has(thread.forked_from)) add(thread)
  return out
}

/** All threads (ADR 0044): a list page, what needs the person first, then every thread by project, newest first. */
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
  const openInto = (thread: ThreadSummary) => (mrs.data ?? []).find((mr) => mr.status === 'open' && (mr.target === thread.id || mr.fork === thread.id))
  const working = all.filter((thread) => thread.agent_state === 'working' || thread.agent_state === 'starting').length
  const filtered = all.filter((thread) => {
    if (filter === 'working') return thread.agent_state === 'working' || thread.agent_state === 'starting'
    if (filter === 'mine') return thread.owner === '' || thread.owner === me
    if (filter === 'shared') return thread.owner !== '' && thread.owner !== me
    if (filter === 'elsewhere') return thread.location !== 'here'
    return true
  })
  const grouped = projects(filtered)
  return (
    <Page
      flush
      label="threads"
      title="All threads"
      meta={threads.isSuccess ? `${grouped.length} ${grouped.length === 1 ? 'project' : 'projects'} · ${all.length} ${all.length === 1 ? 'thread' : 'threads'} · ${working} working` : undefined}
      actions={
        <>
          <Link to="/graph" className={buttonClass('ghost')}>
            <Icon name="fork" />
            Graph
          </Link>
          {!hosted() && (
            <Link to="/" className={buttonClass('primary')}>
              <Icon name="plus" />
              New thread
            </Link>
          )}
        </>
      }
    >
      <div className="mx-auto flex w-full max-w-[960px] flex-col gap-4 px-4 pt-4 pb-8 md:px-6">
        {waiting.length > 0 && (
          <section aria-labelledby="needs-you" className="flex flex-col">
            <h2 id="needs-you" className="px-2.5 pb-1 font-mono text-2xs tracking-[0.06em] text-accent-fg uppercase">
              Needs you · {waiting.length}
            </h2>
            <ul>
              {waiting.map((thread) => (
                <NeedsYouRow key={thread.id} thread={thread} review={thread.review ? openInto(thread) : undefined} />
              ))}
            </ul>
          </section>
        )}
        <Tabs label="which threads" tabs={FILTERS} value={filter} onChange={setFilter} />
        <div data-testid="threads" className="flex flex-col gap-4">
          {threads.error && <p className="py-3 text-danger">{message(threads.error)}</p>}
          {threads.isPending && <Skeleton className="mt-3 h-24" />}
          {threads.isSuccess && filtered.length === 0 && (
            <EmptyState title="Start a thread, or record one">
              Run <code className="font-mono">silt claude</code> in a repository, or start an agent from New thread.
            </EmptyState>
          )}
          {grouped.map(([repo, inRepo]) => (
            <section key={repo} aria-label={`${nameOf(repo)} threads`} className="flex flex-col">
              <h2 className="px-2.5 pb-1 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">{nameOf(repo)}</h2>
              <ul>
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
      </div>
    </Page>
  )
}
