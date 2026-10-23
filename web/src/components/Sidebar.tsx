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
import { threadDoing } from '../threadLook'
import { useThreadState } from '../threadState'
import { ago } from '../time'
import { Avatar, Button, Dot, Icon, type IconName, Menu, SectionLabel, Skeleton, cx } from '../ui'
import { useInWorkspace } from '../workspaceFilter'
import { PaletteRow } from './Palette'
import { ThreadMark } from './ThreadMark'
import { WorkspaceSwitcher } from './WorkspaceSwitcher'

const ROW = 'flex min-w-0 items-center gap-2 rounded-sm px-2 text-fg-1 hover:bg-surface-2 max-md:min-h-11'
const ACTIVE = 'bg-accent-tint text-accent-fg shadow-[inset_2px_0_0_var(--accent)] hover:bg-accent-tint'

/** One thread in the sidebar: its state, its title and how long ago, on one line (ADR 0044). */
function ThreadItem({ thread, nested = false, aside }: { thread: ThreadSummary; nested?: boolean; aside?: ReactNode }) {
  const now = useNow()
  const title = thread.title ?? 'Untitled thread'
  const actions = useThreadState(thread)
  return (
    <li className="group relative flex items-center">
      <Link
        to="/t/$id"
        params={{ id: thread.id }}
        title={[title, threadDoing(thread)].filter(Boolean).join(' · ')}
        className={cx(ROW, 'flex-1 py-[5px]', nested ? 'pl-[26px]' : 'pl-2')}
        activeProps={{ className: ACTIVE }}
      >
        <ThreadMark thread={thread} />
        <span className={cx('min-w-0 flex-1 truncate', unread(thread) && 'font-semibold')}>{title}</span>
        {thread.mine.pinned && <span className="sr-only"> pinned</span>}
        {unread(thread) && <span aria-label="unread" role="img" className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />}
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
function waitsFor(thread: ThreadSummary): string {
  if (thread.agent_state === 'awaiting_input') return 'plan to approve'
  if (thread.agent_state === 'awaiting_permission') return 'asks to run'
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
  return (
    <li role="group" aria-label={`${title} needs you`}>
      <ul>
        <ThreadItem thread={thread} aside={<span className="shrink-0 text-2xs text-accent-fg group-hover:invisible">{waitsFor(thread)}</span>} />
      </ul>
      {asked && (
        <div className="space-y-1 pr-2 pb-2 pl-6 text-xs">
          <p className="truncate text-fg-2" title={asked.title}>
            Asks to run: <span className="font-mono">{asked.title}</span>
          </p>
          <div className="flex flex-wrap gap-1">
            {asked.options.map(([option, label], index) => (
              <Button key={option} size="sm" variant={index === 0 ? 'primary' : 'secondary'} onClick={() => act(api.answer(thread.id, asked.id, option))}>
                {label}
              </Button>
            ))}
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
  const shown = newest.filter((thread) => ['working', 'active'].includes(section(thread, now)))
  const rest = newest.filter((thread) => ['snoozed', 'settled'].includes(section(thread, now)))
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
              {showRest ? 'Show fewer' : `Show ${rest.length} more`}
            </button>
          )}
        </>
      )}
    </section>
  )
}

function NavItem({ to, icon, children, count, accent = false }: { to: string; icon: IconName; children: ReactNode; count?: ReactNode; accent?: boolean }) {
  return (
    <Link to={to} className={cx(ROW, 'h-[30px]')} activeProps={{ className: ACTIVE }}>
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
  const agents = `${running === 1 ? '1 agent' : `${running} agents`}${waiting > 0 ? `, ${waiting} waiting` : ' running'}`
  return (
    <div className="mt-auto flex items-center gap-2 border-t border-line px-2 pt-2.5 pb-1 text-xs text-fg-2">
      {hosted() ? (
        <span className="flex-1 truncate">On the server</span>
      ) : (
        <Link to="/settings/devices" className="flex min-w-0 flex-1 items-center gap-2 hover:text-fg-1" title={sync ?? undefined}>
          <Dot tone={sync ? 'danger' : signIn ? 'ok' : 'idle'} />
          <span className="truncate">
            {name} · {sync ? 'sync failing' : agents}
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
  const listed = mine.filter((thread) => LISTED.includes(section(thread, now)))
  const reviews = mine.filter((thread) => thread.review).length
  const unreadInbox = (inbox.data ?? []).filter((entry) => !entry.read).length
  return (
    <nav aria-label="projects" className="flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2 pt-2.5 pb-2 text-[13px]">
      <div className="flex items-center gap-2 px-1.5 pb-2.5">
        <Link to="/" className="text-[15px] font-medium tracking-[-0.04em] text-fg-1">
          silt
        </Link>
        {!hosted() && <WorkspaceSwitcher />}
      </div>
      {!hosted() && (
        <Link to="/" className={cx(ROW, 'h-[30px]')} activeOptions={{ exact: true, includeSearch: false }} activeProps={{ className: ACTIVE }}>
          <Icon name="plus" className="h-[15px] w-[15px] shrink-0" />
          <span className="flex-1">New thread</span>
          <span aria-hidden="true" className="font-mono text-2xs text-fg-3">{formatKeys('n')}</span>
        </Link>
      )}
      <PaletteRow className={cx(ROW, 'h-[30px] w-full text-left')} />
      <NavItem to="/threads" icon="list">
        All threads
      </NavItem>
      <NavItem to="/mrs" icon="merge" count={reviews || undefined}>
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
            Needs you
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
      <DeviceFooter threads={mine} />
    </nav>
  )
}
