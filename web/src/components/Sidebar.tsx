import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { type ReactNode, useState } from 'react'
import { APPROVED } from '../agentState'
import { useAgentStatus } from '../agentStatus'
import { api, type ThreadSummary } from '../api'
import { useTopic } from '../bus'
import { useNow } from '../now'
import { projectName, projects } from '../projects'
import { keys, message } from '../query'
import { type Section, section, sections, unread } from '../sections'
import { hosted } from '../served'
import { formatKeys } from '../shortcuts'
import { threadDoing, threadTone } from '../threadLook'
import { useThreadState } from '../threadState'
import { ago } from '../time'
import { Button, Dot, Icon, type IconName, Kbd, Menu, SectionLabel, Skeleton, buttonClass, cx } from '../ui'
import { useInWorkspace } from '../workspaceFilter'
import { WorkspaceSwitcher } from './WorkspaceSwitcher'

function ThreadItem({ thread, project, nested = false }: { thread: ThreadSummary; project?: string; nested?: boolean }) {
  const now = useNow()
  const title = thread.title ?? 'Untitled thread'
  const actions = useThreadState(thread)
  const meta = [project, thread.agent, threadDoing(thread)].filter(Boolean).join(' · ')
  return (
    <li className="group relative flex items-start hover:bg-surface-2">
      <Link
        to="/t/$id"
        params={{ id: thread.id }}
        className={cx('flex min-w-0 flex-1 flex-col justify-center gap-px py-1.5 pr-1 text-fg-1 max-md:min-h-11', nested ? 'pl-[30px]' : 'pl-3.5')}
        activeProps={{ className: 'bg-accent-tint before:absolute before:inset-y-1.5 before:left-0 before:w-0.5 before:bg-accent' }}
      >
        <span className={cx('flex min-w-0 items-center gap-1.5', unread(thread) && 'font-semibold')}>
          <Dot tone={threadTone(thread)} />
          <span className="truncate">{title}</span>
          {thread.mine.pinned && <span className="sr-only"> pinned</span>}
          {unread(thread) && <span aria-label="unread" role="img" className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
        </span>
        <span className="truncate font-mono text-2xs text-fg-2">
          {meta}
          {meta && ' · '}
          <span data-volatile>{ago(thread.last_ms, now)}</span>
        </span>
      </Link>
      <span className="mt-1 mr-1 opacity-0 group-hover:opacity-100 focus-within:opacity-100">
        <Menu label={`actions for ${title}`} trigger={<Icon name="more" />} items={actions} />
      </span>
    </li>
  )
}

/** A thread waiting on the person, with what it waits for answerable in place. */
export function NeedsYouItem({ thread, project }: { thread: ThreadSummary; project?: string }) {
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
        <ThreadItem thread={thread} project={project} />
      </ul>
      {asked && (
        <div className="space-y-1 pr-3 pb-2 pl-[26px] text-xs">
          <p className="truncate text-fg-2" title={asked.title}>
            Asks to run: <span className="font-mono">{asked.title}</span>
          </p>
          <div className="flex flex-wrap gap-1">
            {asked.options.map(([option, label], index) => (
              <Button
                key={option}
                size="sm"
                variant={index === 0 ? 'primary' : 'secondary'}
                onClick={() => act(api.answer(thread.id, asked.id, option))}
              >
                {label}
              </Button>
            ))}
          </div>
        </div>
      )}
      {status?.state === 'awaiting_input' && (
        <div className="pr-3 pb-2 pl-[26px]">
          <Button variant="primary" size="sm" onClick={() => act(api.prompt(thread.id, APPROVED))}>
            Approve plan
          </Button>
        </div>
      )}
      {error && <p className="pr-3 pb-2 pl-[26px] text-xs text-danger">{error}</p>}
    </li>
  )
}

/** One project's threads, folded away by its name, with what's out of the way behind a count. */
function ProjectGroup({ repo, name, threads, now }: { repo: string; name: string; threads: ThreadSummary[]; now: number }) {
  const [open, setOpen] = useState(true)
  const [showRest, setShowRest] = useState(false)
  const shown = threads.filter((thread) => ['working', 'active'].includes(section(thread, now)))
  const snoozed = threads.filter((thread) => section(thread, now) === 'snoozed')
  const settled = threads.filter((thread) => section(thread, now) === 'settled')
  const rest = [snoozed.length > 0 && `${snoozed.length} snoozed`, settled.length > 0 && `${settled.length} settled`].filter(Boolean).join(' · ')
  return (
    <section aria-label={repo} className="mb-1">
      <div className="flex items-center pr-3">
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className="flex min-w-0 flex-1 items-center gap-1.5 px-3 py-1 text-left text-fg-1 max-md:min-h-11"
          title={repo}
        >
          <Icon name={open ? 'chevronDown' : 'chevronRight'} className="h-3 w-3 text-fg-2" />
          <span className="truncate font-mono font-medium">{name}</span>
          {!open && <span className="ml-auto font-mono text-2xs text-fg-2">{threads.length}</span>}
        </button>
        <Link
          to="/graph"
          aria-label={`${name} history graph`}
          className="inline-flex items-center justify-center text-fg-2 hover:text-fg-1 max-md:min-h-11 max-md:min-w-11"
        >
          <Icon name="graph" className="h-3.5 w-3.5" />
        </Link>
      </div>
      {open && (
        <>
          <ul>
            {shown.map((thread) => (
              <ThreadItem key={thread.id} thread={thread} nested />
            ))}
          </ul>
          {rest && (
            <button
              type="button"
              aria-expanded={showRest}
              onClick={() => setShowRest(!showRest)}
              className="block py-1 pr-3 pl-[30px] text-left text-2xs text-fg-2 hover:text-fg-1"
            >
              {rest}
            </button>
          )}
          {showRest &&
            (
              [
                ['snoozed', snoozed],
                ['settled', settled],
              ] as const
            ).map(
              ([label, folded]) =>
                folded.length > 0 && (
                  <section key={label} aria-label={label}>
                    <ul>
                      {folded.map((thread) => (
                        <ThreadItem key={thread.id} thread={thread} nested />
                      ))}
                    </ul>
                  </section>
                ),
            )}
        </>
      )}
    </section>
  )
}

function NavItem({ to, icon, children, count }: { to: string; icon: IconName; children: ReactNode; count?: ReactNode }) {
  return (
    <Link
      to={to}
      activeOptions={{ exact: to === '/' }}
      className="group/nav relative flex items-center gap-2 px-3 py-1 text-fg-1 hover:bg-surface-2 data-[status=active]:bg-accent-tint data-[status=active]:text-accent-fg data-[status=active]:before:absolute data-[status=active]:before:inset-y-1 data-[status=active]:before:left-0 data-[status=active]:before:w-0.5 data-[status=active]:before:bg-accent max-md:min-h-11"
    >
      <Icon name={icon} className="h-3.5 w-3.5 text-fg-2 group-data-[status=active]/nav:text-accent-fg" />
      {children}
      {count !== undefined && <span className="ml-auto font-mono text-2xs text-fg-2">{count}</span>}
    </Link>
  )
}

/** This machine at the bottom: its name and how many agents run on it. */
function DeviceFooter({ threads }: { threads: ThreadSummary[] }) {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const running = threads.filter((thread) =>
    ['working', 'starting', 'awaiting_permission', 'awaiting_input'].includes(thread.agent_state ?? ''),
  ).length
  const name = status.data?.servers?.[0]?.name ?? 'this machine'
  return (
    <div className="mt-auto border-t border-line px-3 pt-2.5 pb-2 text-xs text-fg-2">
      {!hosted() && (
        <Link to="/devices" className="flex items-center gap-2 hover:text-fg-1">
          <Dot tone="ok" />
          <span className="truncate">
            {name} · {running === 1 ? '1 agent running' : `${running} agents running`}
          </span>
        </Link>
      )}
      <p className="mt-1.5 flex gap-3 text-2xs">
        <Link to="/projects" className="hover:text-fg-1" activeProps={{ className: 'text-accent-fg' }}>
          Projects
        </Link>
        <Link to="/orgs" className="hover:text-fg-1" activeProps={{ className: 'text-accent-fg' }}>
          Orgs
        </Link>
        <Link to="/devices" className="hover:text-fg-1" activeProps={{ className: 'text-accent-fg' }}>
          Devices
        </Link>
      </p>
    </div>
  )
}

const LISTED: Section[] = ['working', 'active', 'snoozed', 'settled']

export function Sidebar() {
  useTopic('threads')
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const named = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const inbox = useQuery({ queryKey: keys.inbox(false), queryFn: () => api.inbox(false), retry: false })
  const nameOf = (repo: string) => named.data?.find((project) => project.repo === repo)?.name ?? projectName(repo)
  const now = useNow()
  const shown = useInWorkspace()
  const mine = (threads.data ?? []).filter(shown)
  const grouped = sections(mine, now)
  const listed = mine.filter((thread) => LISTED.includes(section(thread, now)))
  const reviews = mine.filter((thread) => thread.review).length
  const unreadInbox = (inbox.data ?? []).filter((entry) => !entry.read).length
  return (
    <nav aria-label="projects" className="flex min-h-0 flex-1 flex-col overflow-y-auto pt-2.5 text-sm">
      <div className="flex flex-col gap-2 px-2.5 pb-2">
        {!hosted() && <WorkspaceSwitcher />}
        <Link to="/new" className={cx(buttonClass('primary'), 'w-full justify-between px-2.5 py-1.5')}>
          <span className="inline-flex items-center gap-1.5">
            <Icon name="plus" />
            New thread
          </span>
          <Kbd onAccent>{formatKeys('n')}</Kbd>
        </Link>
      </div>
      <div className="flex flex-col pb-1">
        <NavItem to="/" icon="thread">
          All threads
        </NavItem>
        <NavItem to="/inbox" icon="inbox" count={unreadInbox || undefined}>
          Inbox
        </NavItem>
        <NavItem to="/mrs" icon="merge" count={reviews || undefined}>
          Merge requests
        </NavItem>
        <NavItem to="/search" icon="search">
          Search
        </NavItem>
      </div>
      {threads.isPending && (
        <div className="space-y-2 px-3">
          <Skeleton />
          <Skeleton className="h-4 w-2/3" />
        </div>
      )}
      {grouped.needs_you.length > 0 && (
        <section aria-label="needs you">
          <SectionLabel className="text-accent-fg" aside={<span className="font-mono">{grouped.needs_you.length}</span>}>
            Needs you
          </SectionLabel>
          <ul>
            {grouped.needs_you.map((thread) => (
              <NeedsYouItem key={thread.id} thread={thread} project={nameOf(thread.repo || 'no remote')} />
            ))}
          </ul>
        </section>
      )}
      {grouped.pinned.length > 0 && (
        <section aria-label="pinned">
          <SectionLabel>Pinned</SectionLabel>
          <ul>
            {grouped.pinned.map((thread) => (
              <ThreadItem key={thread.id} thread={thread} project={nameOf(thread.repo || 'no remote')} />
            ))}
          </ul>
        </section>
      )}
      <SectionLabel
        className="mt-1"
        aside={
          <Link
            to="/projects"
            hash="add"
            className="inline-flex items-center font-sans text-2xs tracking-normal text-accent-fg normal-case hover:text-fg-1 max-md:min-h-11 max-md:px-2"
          >
            + Add
          </Link>
        }
      >
        Projects
      </SectionLabel>
      {projects(listed).map(([repo, inRepo]) => (
        <ProjectGroup key={repo} repo={repo} name={nameOf(repo)} threads={inRepo} now={now} />
      ))}
      <DeviceFooter threads={mine} />
    </nav>
  )
}
