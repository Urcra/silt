import { useQuery } from '@tanstack/react-query'
import { Link, useRouterState } from '@tanstack/react-router'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { useAgentStatus } from '../agentStatus'
import { api, type ThreadSummary } from '../api'
import { useTopic } from '../bus'
import { switchBackend, useBackends } from '../desktop'
import { useNow } from '../now'
import { openPalette } from '../palette'
import { projectName, projects } from '../projects'
import { keys } from '../query'
import { type Section, section, sections, unread } from '../sections'
import { hosted } from '../served'
import { formatKeys } from '../shortcuts'
import { WAIT_KINDS, threadDoing, waitKind } from '../threadLook'
import { useThreadState } from '../threadState'
import { ago } from '../time'
import { Avatar, Dot, Icon, type IconName, Kbd, Menu, SectionLabel, Skeleton, Tag, cx } from '../ui'
import { useInWorkspace } from '../workspaceFilter'
import { ThreadMark } from './ThreadMark'
import { WorkspaceSwitcher } from './WorkspaceSwitcher'

const ROW = 'flex min-w-0 items-center gap-2 rounded-sm px-2 text-fg-1 hover:bg-surface-2 max-md:min-h-11'
const ACTIVE = 'bg-accent-tint text-accent-fg shadow-[inset_2px_0_0_var(--accent)] hover:bg-accent-tint'

/** What a thread is at, in words, for the line under its title: "Working", "Review requested · 3m". */
/** What a thread is at, in words, and since when; the time stands apart, as it changes on its own. */
function stateLine(thread: ThreadSummary, now: number): { said: string | null; when: string | null } {
  const doing = threadDoing(thread) ?? (thread.merged ? 'merged' : thread.mine.settled ? 'settled' : null)
  const when = ago(thread.last_ms, now)
  if (!doing) return { said: null, when: /^\d/.test(when) ? `${when} ago` : when }
  const said = doing[0]!.toUpperCase() + doing.slice(1)
  return { said, when: thread.agent_state === 'working' || thread.agent_state === 'starting' ? null : when }
}

function StateLine({ said, when }: { said: string | null; when: string | null }) {
  return (
    <span className="truncate text-2xs text-fg-2">
      {said}
      {said && when && ' · '}
      {when && <span data-volatile>{when}</span>}
    </span>
  )
}

/** One thread in the sidebar (ADR 0044): its state's mark and its title, then what it's at in words, so the title keeps the row's width. */
function ThreadItem({ thread, nested = false, current = true }: { thread: ThreadSummary; nested?: boolean; current?: boolean }) {
  const now = useNow()
  const title = thread.title ?? 'Untitled thread'
  const actions = useThreadState(thread)
  const item = useRef<HTMLLIElement>(null)
  const open = useRouterState({ select: (state) => state.location.pathname }) === `/t/${thread.id}`
  // The thread open shows in the list however far down its project is.
  useEffect(() => {
    if (open && current) item.current?.scrollIntoView?.({ block: 'nearest' })
  }, [open, current])
  return (
    <li ref={item} className="group relative flex items-center">
      <Link
        to="/t/$id"
        params={{ id: thread.id }}
        title={[title, threadDoing(thread)].filter(Boolean).join(' · ')}
        className={cx(ROW, 'flex-1 items-start py-[5px] pr-7', nested ? 'pl-[22px]' : 'pl-2')}
        activeProps={current ? { className: ACTIVE } : undefined}
      >
        <span className="flex h-[18px] items-center">
          <ThreadMark thread={thread} />
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span className={cx('truncate', unread(thread) && 'font-semibold')}>{title}</span>
          <StateLine {...stateLine(thread, now)} />
        </span>
        {thread.mine.pinned && <span className="sr-only"> pinned</span>}
        {/* Not amber, which the mark before the title keeps for what waits on someone. */}
        {unread(thread) && <span aria-label="unread" role="img" className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-info" />}
      </Link>
      <span className="absolute right-1 opacity-0 group-hover:opacity-100 focus-within:opacity-100">
        <Menu label={`actions for ${title}`} trigger={<Icon name="more" />} items={actions} />
      </span>
    </li>
  )
}

/**
 * A thread waiting on the person, one row of one height whatever it waits for: the kind's
 * icon, its title and how long it has waited. Answering is the queue's or the thread's.
 */
function NeedsYouItem({ thread }: { thread: ThreadSummary }) {
  const now = useNow()
  const status = useAgentStatus(thread.id)
  const kind = waitKind(thread, status)
  const { icon, word } = WAIT_KINDS[kind]
  const title = thread.title ?? 'Untitled thread'
  // A request waiting for review is named for itself, not for the thread it would merge into.
  const mrs = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests, enabled: kind === 'review' })
  const review = kind === 'review' ? (mrs.data ?? []).find((mr) => mr.target === thread.id && mr.status === 'open') : undefined
  const asked = status?.permissions[0]
  const detail = kind === 'permission' && asked ? `${title} · asks to run ${asked.title}` : review ? `Merge request into ${title}` : `${title} · ${word}`
  const row = (
    <>
      <Icon name={icon} className={cx('h-3.5 w-3.5 shrink-0', kind === 'stopped' ? 'text-danger' : 'text-accent-fg')} />
      <span className="min-w-0 flex-1 truncate">{review?.title ?? title}</span>
      <span className="sr-only"> {word}</span>
      <span className="min-w-7 shrink-0 text-right font-mono text-2xs whitespace-nowrap text-fg-3" data-volatile>
        {ago(thread.last_ms, now).replace('just now', 'now')}
      </span>
    </>
  )
  return (
    <li role="group" aria-label={`${review?.title ?? title} needs you`}>
      {review ? (
        <Link to="/mr/$id" params={{ id: review.id }} title={detail} className={cx(ROW, 'h-7')} activeProps={{ className: 'bg-elevated' }}>
          {row}
        </Link>
      ) : (
        <Link to="/t/$id" params={{ id: thread.id }} title={detail} className={cx(ROW, 'h-7')} activeProps={{ className: 'bg-elevated' }}>
          {row}
        </Link>
      )}
    </li>
  )
}

/** One project's threads, newest first, folded away by its name, with what's settled behind a count. */
function ProjectGroup({ repo, name, clone, threads, now }: { repo: string; name: string; clone?: string; threads: ThreadSummary[]; now: number }) {
  const [open, setOpen] = useState(true)
  const [showRest, setShowRest] = useState(false)
  const newest = [...threads].sort((a, b) => b.last_ms - a.last_ms)
  const shown = newest.filter((thread) => ['needs_you', 'working', 'active'].includes(section(thread, now)))
  const rest = newest.filter((thread) => ['snoozed', 'settled'].includes(section(thread, now)))
  const restKinds = new Set(rest.map((thread) => section(thread, now)))
  const restSaid = restKinds.size > 1 ? 'settled or snoozed' : [...restKinds][0]
  const working = threads.filter((thread) => thread.agent_state === 'working' || thread.agent_state === 'starting').length
  return (
    <section aria-label={repo} className="mb-0.5">
      <div className="group/project flex items-center gap-1 pr-1">
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className="flex min-w-0 flex-1 items-center gap-1.5 rounded-sm px-2 py-1 text-left text-fg-1 hover:bg-surface-2 max-md:min-h-11"
          title={repo}
        >
          <Icon name={open ? 'chevronDown' : 'chevronRight'} className="h-3 w-3 shrink-0 text-fg-2" />
          <span className="truncate font-mono font-medium">{name}</span>
          <span className="ml-auto shrink-0 font-mono text-2xs text-fg-3">{working > 0 ? `${working} working` : !open ? threads.length : ''}</span>
        </button>
        {!hosted() && (
          <Link
            to="/"
            search={clone ? { project: clone } : {}}
            aria-label={`New thread in ${name}`}
            className="inline-flex h-6 w-6 items-center justify-center rounded-sm text-fg-2 opacity-0 group-hover/project:opacity-100 hover:bg-surface-2 hover:text-fg-1 focus-visible:opacity-100 max-md:min-h-11 max-md:min-w-11 max-md:opacity-100"
          >
            <Icon name="plus" className="h-3.5 w-3.5" />
          </Link>
        )}
      </div>
      {open && (
        <>
          <ul>
            {(showRest ? [...shown, ...rest] : shown).map((thread) => (
              <ThreadItem key={thread.id} thread={thread} nested />
            ))}
          </ul>
          {rest.length > 0 && (
            <button
              type="button"
              aria-expanded={showRest}
              onClick={() => setShowRest(!showRest)}
              className="block py-1 pr-2 pl-[26px] text-left text-2xs text-fg-2 hover:text-fg-1 max-md:min-h-11"
            >
              {showRest ? 'Show fewer' : `Show ${rest.length} ${restSaid}`}
            </button>
          )}
        </>
      )}
    </section>
  )
}

function NavItem({
  to,
  icon,
  children,
  count,
  accent = false,
  also = false,
}: {
  to: string
  icon: IconName
  children: ReactNode
  count?: ReactNode
  accent?: boolean
  /** Current on a page under it too, such as one merge request under the list. */
  also?: boolean
}) {
  return (
    <Link to={to} className={cx(ROW, 'h-[30px]', also && ACTIVE)} activeProps={{ className: ACTIVE }}>
      <Icon name={icon} className="h-[15px] w-[15px] shrink-0" />
      <span className="flex-1 truncate">{children}</span>
      {count !== undefined && <span className={cx('font-mono text-2xs', accent ? 'text-accent-fg' : 'text-fg-2')}>{count}</span>}
    </Link>
  )
}

/** This machine at the bottom: its name, how many agents run on it, and Settings. */
function DeviceFooter({ threads }: { threads: ThreadSummary[] }) {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const running = threads.filter((thread) => ['working', 'starting'].includes(thread.agent_state ?? '')).length
  const waiting = threads.filter((thread) => ['awaiting_permission', 'awaiting_input'].includes(thread.agent_state ?? '')).length
  const signIn = status.data?.servers?.[0]
  const name = signIn?.name ?? 'this machine'
  const sync = status.data?.sync_error
  const { backends, current } = useBackends()
  return (
    <div className="flex flex-col border-t border-line">
    {backends.length > 1 && (
      <div className="flex items-center gap-2 px-2 pt-2 text-xs text-fg-2">
        <Icon name="laptop" className="h-3.5 w-3.5 shrink-0" />
        <span className="shrink-0">Runs on</span>
        <Menu
          label="where Silt runs"
          trigger={
            <span className="inline-flex min-w-0 items-center gap-1 text-fg-1">
              <span className="truncate">{backends.find((backend) => backend.id === current)?.label ?? 'This computer'}</span>
              <Icon name="chevronDown" className="h-3 w-3 shrink-0" />
            </span>
          }
          items={backends.map((backend) => ({ label: backend.label, onSelect: () => switchBackend(backend.id), disabled: backend.id === current }))}
        />
      </div>
    )}
    <div className="flex items-center gap-2 px-2 pt-2.5 pb-1 text-xs text-fg-2">
      {hosted() ? (
        <span className="flex-1 truncate">On the server</span>
      ) : (
        <Link to="/settings/devices" className="flex min-w-0 flex-1 items-center gap-2 hover:text-fg-1" title={sync ?? undefined}>
          <Dot tone={sync ? 'danger' : signIn ? 'ok' : 'idle'} />
          <span className="truncate">{sync ? `${name} · sync failing` : name}</span>
        </Link>
      )}
      {waiting > 0 && (
        <Tag tone="accent" className="shrink-0 font-sans">
          {waiting} waiting
        </Tag>
      )}
      {running > 0 && (
        <Tag tone="ok" className="shrink-0 font-sans">
          {running} running
        </Tag>
      )}
      <Link
        to="/settings"
        aria-label="Settings"
        className="inline-flex h-7 w-7 items-center justify-center rounded-sm hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11 max-md:min-w-11"
        activeProps={{ className: 'bg-accent-tint text-accent-fg' }}
      >
        <Icon name="settings" className="h-[15px] w-[15px]" />
      </Link>
      {signIn && <Avatar name={signIn.user} size={22} />}
    </div>
    </div>
  )
}

const LISTED: Section[] = ['working', 'active', 'snoozed', 'settled']

/** The sidebar every page shares (ADR 0044): starting, finding and going, then what needs the person, then each project's threads. */
export function Sidebar() {
  useTopic('threads')
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const named = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const inbox = useQuery({ queryKey: keys.inbox(false), queryFn: () => api.inbox(false), retry: false })
  const nameOf = (repo: string) => named.data?.find((project) => project.repo === repo)?.name ?? projectName(repo)
  const cloneOf = (repo: string) => named.data?.find((project) => project.repo === repo && project.clone)?.clone ?? undefined
  const now = useNow()
  const shown = useInWorkspace()
  const mine = (threads.data ?? []).filter(shown)
  const grouped = sections(mine, now)
  // A thread that needs the person stays in its project too, where it lives.
  const listed = mine.filter((thread) => [...LISTED, 'needs_you'].includes(section(thread, now)))
  const path = useRouterState({ select: (state) => state.location.pathname })
  const reviews = mine.filter((thread) => thread.review).length
  const unreadInbox = (inbox.data ?? []).filter((entry) => !entry.read).length
  return (
    <>
    <nav aria-label="projects" className="flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2 pt-2.5 pb-2 text-[13px] [&>*]:shrink-0">
      <div className="flex items-center gap-2 px-1.5 pb-2.5">
        <Link to="/" className="text-[15px] font-medium tracking-[-0.04em] text-fg-1">
          silt
        </Link>
        {!hosted() && <WorkspaceSwitcher />}
        <button
          type="button"
          aria-label="command palette"
          title={`Search and commands (${formatKeys('mod+k')})`}
          onClick={openPalette}
          className="ml-auto inline-flex h-6 w-6 items-center justify-center rounded-sm text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11 max-md:min-w-11"
        >
          <Icon name="search" className="h-[15px] w-[15px]" />
        </button>
      </div>
      {!hosted() && (
        <Link to="/new" className={cx(ROW, 'h-[30px]', (path === '/' || path === '/new') && ACTIVE)}>
          <Icon name="plus" className="h-[15px] w-[15px] shrink-0" />
          <span className="flex-1">New thread</span>
          <Kbd>{formatKeys('n')}</Kbd>
        </Link>
      )}
      <NavItem to="/threads" icon="list">
        All threads
      </NavItem>
      <NavItem to="/mrs" icon="merge" count={reviews || undefined} also={path.startsWith('/mr/')}>
        Merge requests
      </NavItem>
      <NavItem to="/inbox" icon="inbox" count={unreadInbox || undefined} accent>
        Inbox
      </NavItem>
      {threads.isPending && (
        <div className="space-y-2 px-2 pt-3">
          <Skeleton />
          <Skeleton className="h-4 w-2/3" />
        </div>
      )}
      {grouped.needs_you.length > 0 && (
        <section aria-label="needs you" className="mt-3 mb-1 rounded-md border border-accent bg-accent-tint px-1 pt-1 pb-0.5">
          <SectionLabel className="px-2 pt-1 text-accent-fg" aside={<span className="font-mono">{grouped.needs_you.length}</span>}>
            Needs you
          </SectionLabel>
          <ul>
            {grouped.needs_you.map((thread) => (
              <NeedsYouItem key={thread.id} thread={thread} />
            ))}
          </ul>
          <Link
            to="/needs-you"
            className={cx('flex items-center justify-between rounded-sm px-2 py-1.5 text-xs text-accent-fg hover:bg-elevated max-md:min-h-11', path === '/needs-you' && 'bg-elevated')}
          >
            Open the queue
            <Kbd>{formatKeys('q')}</Kbd>
          </Link>
        </section>
      )}
      {grouped.pinned.length > 0 && (
        <section aria-label="pinned" className="mt-2">
          <SectionLabel className="px-2">Pinned</SectionLabel>
          <ul>
            {grouped.pinned.map((thread) => (
              <ThreadItem key={thread.id} thread={thread} />
            ))}
          </ul>
        </section>
      )}
      <SectionLabel
        className="mt-2 px-2"
        aside={
          !hosted() && (
            <Link
              to="/"
              search={{ pick: 'folder' }}
              aria-label="Add a project"
              className="inline-flex items-center rounded-sm p-0.5 text-fg-2 hover:text-fg-1 max-md:min-h-11 max-md:px-2"
            >
              <Icon name="plus" className="h-3.5 w-3.5" />
            </Link>
          )
        }
      >
        Projects
      </SectionLabel>
      {threads.isSuccess && listed.length === 0 && grouped.needs_you.length === 0 && <p className="px-2 py-1 text-xs text-fg-2">None yet.</p>}
      {projects(listed).map(([repo, inRepo]) => (
        <ProjectGroup key={repo} repo={repo} name={nameOf(repo)} clone={cloneOf(repo)} threads={inRepo} now={now} />
      ))}
    </nav>
    {/* Outside the list, so a long one never scrolls Settings and this machine out of reach. */}
    <div className="shrink-0 px-2 pb-2">
      <DeviceFooter threads={mine} />
    </div>
    </>
  )
}
