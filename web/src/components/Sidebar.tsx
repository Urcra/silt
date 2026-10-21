import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { APPROVED, STATE_TAG } from '../agentState'
import { useAgentStatus } from '../agentStatus'
import { useNow } from '../now'
import { api, type ThreadSummary } from '../api'
import { useTopic } from '../bus'
import { projectName, projects } from '../projects'
import { keys, message } from '../query'
import { type Section, sections, unread } from '../sections'
import { useThreadState } from '../threadState'
import { Button, Icon, Menu, Skeleton, Tag } from '../ui'

/** An agent state worth showing next to its thread; a stopped one isn't. */
function shownState(thread: ThreadSummary) {
  return thread.agent_state && thread.agent_state !== 'stopped' && thread.agent_state !== 'idle'
    ? STATE_TAG[thread.agent_state]
    : null
}

function ThreadItem({ thread }: { thread: ThreadSummary }) {
  const state = shownState(thread)
  const title = thread.title ?? 'Untitled thread'
  const actions = useThreadState(thread)
  return (
    <li className="group flex items-center gap-2 pr-2 hover:bg-surface-2">
      <Link
        to="/t/$id"
        params={{ id: thread.id }}
        className={`block min-w-0 flex-1 truncate py-1 pl-4 text-fg-1 ${unread(thread) ? 'font-semibold' : ''}`}
        activeProps={{ className: 'bg-accent-tint text-accent-fg' }}
      >
        {unread(thread) && (
          <span aria-label="unread" role="img" className="mr-1.5 inline-block h-1.5 w-1.5 rounded-full bg-accent align-middle" />
        )}
        {thread.mine.pinned && <span className="sr-only">pinned </span>}
        {title}
      </Link>
      {state && <Tag tone={state.tone}>{state.label}</Tag>}
      <span className="opacity-0 group-hover:opacity-100 focus-within:opacity-100">
        <Menu label={`actions for ${title}`} trigger={<Icon name="more" />} items={actions} />
      </span>
    </li>
  )
}

/** A thread waiting on the person, with what it waits for answerable in place. */
export function NeedsYouItem({ thread }: { thread: ThreadSummary }) {
  const status = useAgentStatus(thread.id)
  const [error, setError] = useState<string>()
  const act = (action: Promise<unknown>) => {
    setError(undefined)
    action.catch((failure: unknown) => setError(message(failure)))
  }
  const asked = status?.permissions[0]
  const title = thread.title ?? 'Untitled thread'
  return (
    <li role="group" aria-label={`${title} needs you`}>
      <ul>
        <ThreadItem thread={thread} />
      </ul>
      {asked && (
        <div className="space-y-1 px-4 pb-2 text-xs">
          <p className="truncate text-fg-2" title={asked.title}>
            Asks to run: {asked.title}
          </p>
          <div className="flex flex-wrap gap-1">
            {asked.options.map(([option, label]) => (
              <Button key={option} className="px-1.5 py-0.5" onClick={() => act(api.answer(thread.id, asked.id, option))}>
                {label}
              </Button>
            ))}
          </div>
        </div>
      )}
      {status?.state === 'awaiting_input' && (
        <div className="px-4 pb-2">
          <Button variant="primary" className="px-1.5 py-0.5" onClick={() => act(api.prompt(thread.id, APPROVED))}>
            Approve plan
          </Button>
        </div>
      )}
      {thread.review && <p className="px-4 pb-2 text-xs text-fg-2">A merge request waits for your review</p>}
      {error && <p className="px-4 pb-2 text-xs text-danger">{error}</p>}
    </li>
  )
}

const HEADINGS: Partial<Record<Section, string>> = { needs_you: 'Needs you', pinned: 'Pinned', working: 'Working' }

function Heading({ children, tone = 'text-fg-3' }: { children: React.ReactNode; tone?: string }) {
  return <h2 className={`flex items-center justify-between px-4 pt-2 pb-1 font-mono text-[10px] tracking-wider uppercase ${tone}`}>{children}</h2>
}

/** Threads out of the way, behind their count. */
function Folded({ label, threads }: { label: string; threads: ThreadSummary[] }) {
  const [open, setOpen] = useState(false)
  if (threads.length === 0) return null
  return (
    <section aria-label={label.toLowerCase()} className="mb-2">
      <button type="button" aria-expanded={open} className="w-full text-left" onClick={() => setOpen(!open)}>
        <Heading>
          <span>{label}</span>
          <span>{threads.length}</span>
        </Heading>
      </button>
      {open && (
        <ul>
          {threads.map((thread) => (
            <ThreadItem key={thread.id} thread={thread} />
          ))}
        </ul>
      )}
    </section>
  )
}

export function Sidebar() {
  useTopic('threads')
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const named = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const nameOf = (repo: string) => named.data?.find((project) => project.repo === repo)?.name ?? projectName(repo)
  const now = useNow()
  const grouped = sections(threads.data ?? [], now)
  const link = 'mx-2 mb-2 flex items-center gap-2 rounded-sm px-2 py-1 text-fg-1 hover:bg-surface-2'
  return (
    <nav aria-label="projects" className="flex min-h-0 flex-1 flex-col overflow-y-auto py-2 text-sm">
      <Link to="/" className={link} activeOptions={{ exact: true }} activeProps={{ className: 'bg-accent-tint text-accent-fg' }}>
        <Icon name="thread" />
        All threads
      </Link>
      <Link to="/new" className={link} activeProps={{ className: 'bg-accent-tint text-accent-fg' }}>
        <Icon name="plus" />
        New thread
      </Link>
      <Link to="/projects" className={link} activeProps={{ className: 'bg-accent-tint text-accent-fg' }}>
        <Icon name="branch" />
        Projects
      </Link>
      {threads.isPending && (
        <div className="space-y-2 px-4">
          <Skeleton />
          <Skeleton className="h-4 w-2/3" />
        </div>
      )}
      {(['needs_you', 'pinned', 'working'] as const).map(
        (kind) =>
          grouped[kind].length > 0 && (
            <section key={kind} aria-label={HEADINGS[kind]!.toLowerCase()} className="mb-2">
              <Heading tone={kind === 'needs_you' ? 'text-accent-fg' : 'text-fg-3'}>{HEADINGS[kind]}</Heading>
              <ul>
                {grouped[kind].map((thread) =>
                  kind === 'needs_you' ? <NeedsYouItem key={thread.id} thread={thread} /> : <ThreadItem key={thread.id} thread={thread} />,
                )}
              </ul>
            </section>
          ),
      )}
      {projects(grouped.active).map(([repo, inRepo]) => (
        <section key={repo} aria-label={repo} className="mb-2">
          <Heading>
            <span className="truncate" title={repo}>
              {nameOf(repo)}
            </span>
            <span>{inRepo.length}</span>
          </Heading>
          <ul>
            {inRepo.map((thread) => (
              <ThreadItem key={thread.id} thread={thread} />
            ))}
          </ul>
        </section>
      ))}
      <Folded label="Snoozed" threads={grouped.snoozed} />
      <Folded label="Settled" threads={grouped.settled} />
    </nav>
  )
}
