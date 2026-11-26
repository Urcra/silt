import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { api, short, type MrSummary, type ThreadSummary } from '../api'
import { useTopic } from '../bus'
import { useNow } from '../now'
import { projectName, projects } from '../projects'
import { keys, message } from '../query'
import { needsYou } from '../sections'
import { hosted } from '../served'
import { inBackground, inProgress, threadWhere } from '../threadLook'
import { ago } from '../time'
import { Avatar, EmptyState, Icon, Input, Skeleton, Tabs, Tag, type Tone, buttonClass, cx } from '../ui'
import { useInWorkspace } from '../workspaceFilter'
import { Page } from './Page'
import { ThreadMark } from './ThreadMark'

type Filter = 'all' | 'needs_you' | 'working' | 'mine' | 'shared' | 'elsewhere'

type Sort = 'title' | 'last'

const working = inProgress

/** A thread's state as a pill, in words; amber only for what waits on the person. */
function threadState(
  thread: ThreadSummary,
  fromIt: MrSummary | undefined,
  recovered: boolean,
): { label: string; tone: Tone } | null {
  if (recovered) return { label: 'Recovered: review it', tone: 'accent' }
  switch (thread.agent_state) {
    case 'awaiting_permission':
      return { label: 'Waiting on you', tone: 'accent' }
    case 'awaiting_input':
      return { label: 'Plan to approve', tone: 'accent' }
    case 'error':
      return { label: 'Stopped', tone: 'danger' }
    case 'working':
    case 'starting':
      return { label: thread.terminal ? 'Working in a terminal' : 'Working', tone: 'ok' }
  }
  if (inBackground(thread)) return { label: 'In the background', tone: 'ok' }
  if (thread.review) return { label: 'Review requested', tone: 'accent' }
  if (fromIt) return { label: 'MR open', tone: 'info' }
  if (thread.merged) return { label: 'Merged', tone: 'neutral' }
  if (thread.mine.settled) return { label: 'Settled', tone: 'neutral' }
  return null
}

/** Where its work is, in a word: here, on the server, or on another device. */
function machine(thread: ThreadSummary): string {
  if (thread.location === 'remote') return 'server'
  if (thread.location === 'moved') return 'another device'
  return threadWhere(thread).label.startsWith('Here') ? 'here' : threadWhere(thread).label
}

function ThreadRow({
  thread,
  now,
  into,
  fromIt,
  recovered,
  depth,
}: {
  thread: ThreadSummary
  now: number
  /** An open merge request into it. */
  into: MrSummary | undefined
  /** An open merge request from it. */
  fromIt: MrSummary | undefined
  /** It holds this machine's work after its copy differed from the server's (ADR 0009). */
  recovered: boolean
  /** How many forks deep under a thread also listed. */
  depth: number
}) {
  const state = threadState(thread, fromIt, recovered)
  const to = recovered ? ('/t/$id/reconcile' as const) : ('/t/$id' as const)
  const mr = into ?? fromIt
  return (
    <tr
      className={cx(
        'group relative border-b border-line-subtle hover:bg-surface-1',
        (into || recovered) && 'bg-accent-tint hover:bg-accent-tint',
      )}
    >
      <td className="py-1.5 pr-3 pl-3">
        <span className="flex min-w-0 items-center gap-2" style={{ paddingLeft: depth * 18 }}>
          {depth > 0 && (
            <span aria-hidden="true" className="-mt-3 h-3.5 w-2.5 shrink-0 rounded-bl-sm border-b border-l border-line-strong" />
          )}
          {thread.forked_from ? (
            <Icon name="fork" label="Fork" className="h-3.5 w-3.5 shrink-0 text-fg-2" />
          ) : (
            <ThreadMark thread={thread} />
          )}
          <span className="flex min-w-0 flex-col">
            <Link to={to} params={{ id: thread.id }} className="truncate after:absolute after:inset-0">
              {thread.title ?? 'Untitled thread'}
            </Link>
            <span className="relative truncate font-mono text-2xs text-fg-3">
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
        </span>
      </td>
      <td className="px-3 whitespace-nowrap">
        {state ? (
          <Tag tone={state.tone} className="font-sans">
            {state.label}
          </Tag>
        ) : (
          <span className="text-fg-3">—</span>
        )}
      </td>
      <td className="px-3 whitespace-nowrap text-fg-2 max-md:hidden">{thread.agent ?? '—'}</td>
      <td className="px-3 whitespace-nowrap text-fg-2 max-lg:hidden">{machine(thread)}</td>
      <td className="px-3 max-lg:hidden">
        {thread.collaborators.length > 0 ? (
          <span
            className="flex items-center gap-1.5 whitespace-nowrap text-fg-2"
            title={`Shared with ${thread.collaborators.join(', ')}`}
          >
            <Avatar name={thread.collaborators[0]!} size={16} />
            {thread.collaborators.join(', ')}
          </span>
        ) : (
          <span className="text-fg-3">—</span>
        )}
      </td>
      <td className="px-3 max-md:hidden">
        {mr ? (
          <Link
            to="/mr/$id"
            params={{ id: mr.id }}
            className="relative z-10 flex min-w-0 items-center gap-1.5 text-xs text-fg-2 hover:text-fg-1"
          >
            <Icon name="merge" className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">{into ? 'into this' : mr.title}</span>
          </Link>
        ) : (
          <span className="text-fg-3">—</span>
        )}
      </td>
      <td className="pr-3 pl-3 text-right font-mono text-2xs whitespace-nowrap text-fg-3">
        <span data-volatile>{ago(thread.last_ms, now)}</span>
      </td>
    </tr>
  )
}

/** Each fork right after the thread it came from, when that is in the list too, with how deep it sits. */
function forksUnderSources(
  threads: ThreadSummary[],
  order: (a: ThreadSummary, b: ThreadSummary) => number,
): [ThreadSummary, number][] {
  const listed = new Set(threads.map((thread) => thread.id))
  const forksOf = (source: string) => threads.filter((thread) => thread.forked_from === source).sort(order)
  const out: [ThreadSummary, number][] = []
  const add = (thread: ThreadSummary, depth: number) => {
    out.push([thread, depth])
    for (const fork of forksOf(thread.id)) add(fork, depth + 1)
  }
  for (const thread of [...threads].sort(order)) if (!thread.forked_from || !listed.has(thread.forked_from)) add(thread, 0)
  return out
}

/** A column heading that sorts the table by it. */
function SortHeading({
  id,
  sort,
  descending,
  onSort,
  children,
  className,
}: {
  id: Sort
  sort: Sort
  descending: boolean
  onSort: (id: Sort) => void
  children: string
  className?: string
}) {
  const current = sort === id
  return (
    <th scope="col" aria-sort={current ? (descending ? 'descending' : 'ascending') : 'none'} className={className}>
      <button
        type="button"
        onClick={() => onSort(id)}
        className="inline-flex items-center gap-1 tracking-[0.06em] whitespace-nowrap uppercase hover:text-fg-1"
      >
        {children}
        {current && <span aria-hidden="true">{descending ? '↓' : '↑'}</span>}
      </button>
    </th>
  )
}

const HEADING = 'px-3 pb-2 text-left font-mono text-2xs font-normal tracking-[0.06em] text-fg-2 uppercase'

/** All threads (ADR 0044, the UI review): one table, by project, with what each thread is at in columns and its forks under it. */
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
  const [text, setText] = useState('')
  const [sort, setSort] = useState<Sort>('last')
  const [descending, setDescending] = useState(true)
  const shown = useInWorkspace()
  const me = status.data?.servers?.[0]?.user ?? ''
  const all = (threads.data ?? []).filter(shown)
  const open = (mrs.data ?? []).filter((mr) => mr.status === 'open')
  const into = (thread: ThreadSummary) => open.find((mr) => mr.target === thread.id)
  const fromIt = (thread: ThreadSummary) => open.find((mr) => mr.fork === thread.id)
  const matches: Record<Filter, (thread: ThreadSummary) => boolean> = {
    all: () => true,
    needs_you: needsYou,
    working,
    mine: (thread) => thread.owner === '' || thread.owner === me,
    shared: (thread) => thread.owner !== '' && thread.owner !== me,
    elsewhere: (thread) => thread.location !== 'here',
  }
  const count = (id: Filter, label: string, accent = false) => ({
    id,
    label: (
      <>
        {label}
        {threads.isSuccess && (
          <span className={cx('font-mono font-normal', accent ? 'text-accent-fg' : 'text-fg-2')}>
            {all.filter(matches[id]).length}
          </span>
        )}
      </>
    ),
  })
  const filters = [
    count('all', 'All'),
    count('needs_you', 'Needs you', all.some(needsYou)),
    count('working', 'Working'),
    { id: 'mine' as const, label: 'Mine' },
    { id: 'shared' as const, label: 'Shared with me' },
    { id: 'elsewhere' as const, label: 'On other devices' },
  ]
  const wanted = text.trim().toLowerCase()
  const filtered = all.filter(
    (thread) =>
      matches[filter](thread) && (!wanted || (thread.title ?? '').toLowerCase().includes(wanted) || thread.id.includes(wanted)),
  )
  const order = (a: ThreadSummary, b: ThreadSummary) => {
    const by = sort === 'title' ? (a.title ?? '').localeCompare(b.title ?? '') : a.last_ms - b.last_ms
    return descending ? -by : by
  }
  const onSort = (id: Sort) => {
    if (id === sort) setDescending(!descending)
    else {
      setSort(id)
      setDescending(id === 'last')
    }
  }
  const grouped = projects(filtered)
  return (
    <Page
      flush
      label="threads"
      title="All threads"
      meta={
        <span className="font-sans">
          <Tabs label="which threads" tabs={filters} value={filter} onChange={setFilter} />
        </span>
      }
      actions={
        <>
          <Input
            aria-label="filter threads"
            placeholder="Filter threads"
            value={text}
            onChange={(event) => setText(event.target.value)}
            className="w-48 max-md:hidden"
          />
          <Link to="/graph" className={buttonClass('ghost')}>
            <Icon name="fork" />
            Graph
          </Link>
          {!hosted() && (
            <Link to="/new" className={buttonClass('primary')}>
              <Icon name="plus" />
              New thread
            </Link>
          )}
        </>
      }
    >
      <div data-testid="threads" className="w-full overflow-x-auto px-4 py-4 md:pr-6 md:pl-5">
        {threads.error && <p className="py-3 text-danger">{message(threads.error)}</p>}
        {threads.isPending && <Skeleton className="mt-3 h-24" />}
        {threads.isSuccess && filtered.length === 0 && (
          <EmptyState title={all.length === 0 ? 'Start a thread, or record one' : 'No thread matches'}>
            {all.length === 0 ? (
              <>
                Run <code className="font-mono">silt claude</code> in a repository, or start an agent from New thread.
              </>
            ) : (
              'Try another filter.'
            )}
          </EmptyState>
        )}
        {grouped.length > 0 && (
          <table className="w-full table-fixed border-collapse text-sm">
            <colgroup>
              <col />
              <col className="w-36" />
              <col className="w-24 max-md:hidden" />
              <col className="w-28 max-lg:hidden" />
              <col className="w-32 max-lg:hidden" />
              <col className="w-52 max-md:hidden" />
              <col className="w-32" />
            </colgroup>
            <thead className="border-b border-line">
              <tr>
                <SortHeading id="title" sort={sort} descending={descending} onSort={onSort} className={HEADING}>
                  Thread
                </SortHeading>
                <th scope="col" className={HEADING}>
                  State
                </th>
                <th scope="col" className={cx(HEADING, 'max-md:hidden')}>
                  Agent
                </th>
                <th scope="col" className={cx(HEADING, 'max-lg:hidden')}>
                  Machine
                </th>
                <th scope="col" className={cx(HEADING, 'max-lg:hidden')}>
                  Shared
                </th>
                <th scope="col" className={cx(HEADING, 'max-md:hidden')}>
                  Merge request
                </th>
                <SortHeading id="last" sort={sort} descending={descending} onSort={onSort} className={cx(HEADING, 'text-right')}>
                  Last activity
                </SortHeading>
              </tr>
            </thead>
            {grouped.map(([repo, inRepo]) => (
              <tbody key={repo} aria-label={`${nameOf(repo)} threads`}>
                <tr>
                  <th colSpan={7} scope="rowgroup" className="px-3 pt-3 pb-1 text-left font-normal">
                    <span className="flex items-baseline gap-2">
                      <span className="font-mono font-medium text-fg-1">{nameOf(repo)}</span>
                      <span className="font-mono text-2xs text-fg-3">
                        {inRepo.length} {inRepo.length === 1 ? 'thread' : 'threads'} · {inRepo.filter(working).length} working
                      </span>
                    </span>
                  </th>
                </tr>
                {forksUnderSources(inRepo, order).map(([thread, depth]) => (
                  <ThreadRow
                    key={thread.id}
                    thread={thread}
                    now={now}
                    into={into(thread)}
                    fromIt={fromIt(thread)}
                    depth={depth}
                    recovered={(status.data?.recoveries ?? []).some((recovery) => recovery.recovery === thread.id)}
                  />
                ))}
              </tbody>
            ))}
          </table>
        )}
      </div>
    </Page>
  )
}
