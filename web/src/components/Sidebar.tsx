import { useQuery } from '@tanstack/react-query'
import { Link, useRouterState } from '@tanstack/react-router'
import { type ReactNode, useEffect, useRef, useState } from 'react'
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
import { threadDoing } from '../threadLook'
import { useThreadState } from '../threadState'
import { ago } from '../time'
import { Avatar, Button, Dot, Icon, type IconName, Kbd, Menu, SectionLabel, Skeleton, buttonClass, cx } from '../ui'
import { useInWorkspace } from '../workspaceFilter'
import { PaletteRow } from './Palette'
import { ThreadMark } from './ThreadMark'
import { WorkspaceSwitcher } from './WorkspaceSwitcher'

const ROW = 'flex min-w-0 items-center gap-2 rounded-sm px-2 text-fg-1 hover:bg-surface-2 max-md:min-h-11'
const ACTIVE = 'bg-accent-tint text-accent-fg shadow-[inset_2px_0_0_var(--accent)] hover:bg-accent-tint'

/** One thread in the sidebar: its state, its title and how long ago, on one line (ADR 0044). */
function ThreadItem({ thread, nested = false, aside, current = true }: { thread: ThreadSummary; nested?: boolean; aside?: ReactNode; current?: boolean }) {
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
        className={cx(ROW, 'flex-1 py-[5px]', nested ? 'pl-[26px]' : 'pl-2')}
        activeProps={current ? { className: ACTIVE } : undefined}
      >
        <ThreadMark thread={thread} />
        <span className={cx('min-w-0 flex-1 truncate', unread(thread) && 'font-semibold')}>{title}</span>
        {thread.mine.pinned && <span className="sr-only"> pinned</span>}
        {/* Not amber, which the dot before the title keeps for what waits on someone. */}
        {unread(thread) && <span aria-label="unread" role="img" className="h-1.5 w-1.5 shrink-0 rounded-full bg-info" />}
        {aside ?? (
          <span className="min-w-7 shrink-0 text-right font-mono text-2xs whitespace-nowrap text-fg-3 group-hover:invisible" data-volatile>
            {ago(thread.last_ms, now).replace('just now', 'now')}
          </span>
        )}
      </Link>
      <span className="absolute right-1 opacity-0 group-hover:opacity-100 focus-within:opacity-100">
        <Menu label={`actions for ${title}`} trigger={<Icon name="more" />} items={actions} />
      </span>
    </li>
  )
}

/** What a thread waiting on the person waits for, in a word or two. */
function waitsFor(thread: ThreadSummary, asks: boolean): string {
  if (thread.agent_state === 'awaiting_input') return 'plan to approve'
  if (thread.agent_state === 'awaiting_permission') return asks ? 'asks you' : 'asks to run'
  if (thread.agent_state === 'error') return 'stopped'
  return 'review'
}

/** A thread waiting on the person, with what it waits for answerable in place. */
export function NeedsYouItem({ thread }: { thread: ThreadSummary; project?: string }) {
  const status = useAgentStatus(thread.id)
  const [error, setError] = useState<string>()
  const act = (action: Promise<unknown>) => {
    setError(undefined)
    action.catch((failure: unknown) => setError(message(failure)))
  }
  const asked = status?.permissions[0]
  const title = thread.title ?? 'Untitled thread'
  // A request waiting for review is named for itself, not for the thread it would merge into.
  const mrs = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests, enabled: thread.review && !asked })
  const review = thread.review && !asked ? (mrs.data ?? []).find((mr) => mr.target === thread.id && mr.status === 'open') : undefined
  if (review)
    return (
      <li role="group" aria-label={`${review.title} needs you`}>
        <Link to="/mr/$id" params={{ id: review.id }} className={cx(ROW, 'py-[5px] pl-2')} title={`Merge request into ${title}`}>
          <Icon name="merge" className="h-3.5 w-3.5 shrink-0 text-fg-2" />
          <span className="min-w-0 flex-1 truncate">{review.title}</span>
          <span className="shrink-0 text-2xs text-accent-fg">review</span>
        </Link>
      </li>
    )
  const first = asked?.options[0]
  return (
    <li role="group" aria-label={`${title} needs you`}>
      <ul>
        <ThreadItem thread={thread} current={false} aside={<span className="shrink-0 text-2xs text-accent-fg group-hover:invisible">{waitsFor(thread, (status?.questions?.length ?? 0) > 0)}</span>} />
      </ul>
      {asked && (
        <div className="space-y-1 pr-2 pb-2 pl-6 text-xs">
          <p className="truncate text-fg-2" title={asked.plan ? 'A plan to approve' : asked.title}>
            {asked.plan ? 'A plan to approve' : <>Asks to run: <span className="font-mono">{asked.title}</span></>}
          </p>
          {/* The first answer here; every other, and the plan, on the thread's own card. */}
          <div className="flex min-w-0 gap-1">
            {first && !asked.plan && (
              <Button size="sm" variant="primary" className="min-w-0" title={first[1]} onClick={() => act(api.answer(thread.id, asked.id, first[0]))}>
                <span className="truncate">{first[1]}</span>
              </Button>
            )}
            <Link to="/t/$id" params={{ id: thread.id }} className={buttonClass('secondary', 'sm')}>
              {asked.plan ? 'Read the plan' : 'Open'}
            </Link>
          </div>
        </div>
      )}
      {status?.state === 'awaiting_input' && (
        <div className="pr-2 pb-2 pl-6">
          <Button variant="primary" size="sm" onClick={() => act(api.prompt(thread.id, APPROVED))}>
            Approve plan
          </Button>
        </div>
      )}
      {error && <p className="pr-2 pb-2 pl-6 text-xs text-danger">{error}</p>}
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
  const running = threads.filter((thread) => ['working', 'starting', 'awaiting_permission', 'awaiting_input'].includes(thread.agent_state ?? '')).length
  const waiting = threads.filter((thread) => ['awaiting_permission', 'awaiting_input'].includes(thread.agent_state ?? '')).length
  const signIn = status.data?.servers?.[0]
  const name = signIn?.name ?? 'this machine'
  const sync = status.data?.sync_error
  const agents = running === 0 ? null : `${running === 1 ? '1 agent' : `${running} agents`}${waiting > 0 ? `, ${waiting} waiting` : ' running'}`
  return (
    <div className="flex items-center gap-2 border-t border-line px-2 pt-2.5 pb-1 text-xs text-fg-2">
      {hosted() ? (
        <span className="flex-1 truncate">On the server</span>
      ) : (
        <Link to="/settings/devices" className="flex min-w-0 flex-1 items-center gap-2 hover:text-fg-1" title={sync ?? undefined}>
          <Dot tone={sync ? 'danger' : signIn ? 'ok' : 'idle'} />
          <span className="truncate">
            {[name, sync ? 'sync failing' : agents].filter(Boolean).join(' · ')}
          </span>
        </Link>
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
      </div>
      {!hosted() && (
        <Link to="/new" className={cx(ROW, 'h-[30px]', (path === '/' || path === '/new') && ACTIVE)}>
          <Icon name="plus" className="h-[15px] w-[15px] shrink-0" />
          <span className="flex-1">New thread</span>
          <Kbd>{formatKeys('n')}</Kbd>
        </Link>
      )}
      <PaletteRow className={cx(ROW, 'h-[30px] w-full text-left')} />
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
        <section aria-label="needs you" className="mt-2">
          <SectionLabel className="px-2 text-accent-fg" aside={<span className="font-mono">{grouped.needs_you.length}</span>}>
            <Link to="/needs-you" className={cx('rounded-sm hover:underline', path === '/needs-you' && 'underline underline-offset-4')}>
              Needs you
            </Link>
          </SectionLabel>
          <ul>
            {grouped.needs_you.map((thread) => (
              <NeedsYouItem key={thread.id} thread={thread} />
            ))}
          </ul>
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
