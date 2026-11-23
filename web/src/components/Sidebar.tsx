import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate, useRouterState } from '@tanstack/react-router'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { useAgentStatus } from '../agentStatus'
import { api, type ThreadSummary } from '../api'
import { isChat } from '../chats'
import { useTopic } from '../bus'
import { type Backend, inDesktop, relaunchDesktop, switchBackend, useBackends } from '../desktop'
import { useNow } from '../now'
import { openPalette } from '../palette'
import { projectName, projects } from '../projects'
import { keys } from '../query'
import { type Section, inListOrder, old, section, sections, unread } from '../sections'
import { hosted, serverName } from '../served'
import { formatKeys } from '../shortcuts'
import { WAIT_KINDS, inProgress, threadDoing, waitKind } from '../threadLook'
import { useThreadState } from '../threadState'
import { ago } from '../time'
import { Button, Dot, type DotTone, Icon, type IconName, Kbd, Menu, Popover, SectionLabel, Skeleton, Spinner, cx } from '../ui'
import { pillOf, useUpdate } from '../updates'
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
  return { said, when: inProgress(thread) ? null : when }
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
        className={cx(ROW, 'flex-1 items-start py-[calc((var(--row)-20px)/2)] pr-7', nested ? 'pl-[22px]' : 'pl-2')}
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
  const detail =
    kind === 'permission' && asked
      ? `${title} · asks to run ${asked.title}`
      : review
        ? `Merge request into ${title}`
        : `${title} · ${word}`
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
        <Link
          to="/mr/$id"
          params={{ id: review.id }}
          title={detail}
          className={cx(ROW, 'h-7')}
          activeProps={{ className: 'bg-elevated' }}
        >
          {row}
        </Link>
      ) : (
        <Link
          to="/t/$id"
          params={{ id: thread.id }}
          title={detail}
          className={cx(ROW, 'h-7')}
          activeProps={{ className: 'bg-elevated' }}
        >
          {row}
        </Link>
      )}
    </li>
  )
}

/**
 * One project's threads, those busy in the last few minutes by title and then newest first, folded away
 * by its name, with what's old, settled or snoozed behind a count. An older project is one row until opened, and then has nothing to hold back.
 */
function ProjectGroup({
  repo,
  name,
  clone,
  threads,
  now,
  older = false,
}: {
  repo: string
  name: string
  clone?: string
  threads: ThreadSummary[]
  now: number
  older?: boolean
}) {
  const [open, setOpen] = useState(!older)
  const [showRest, setShowRest] = useState(false)
  const ordered = [...threads].sort(inListOrder(now))
  const held = (thread: ThreadSummary) => old(thread, now) || ['snoozed', 'settled'].includes(section(thread, now))
  const shown = ordered.filter((thread) => !held(thread))
  const rest = ordered.filter(held)
  const restKinds = new Set(rest.map((thread) => (old(thread, now) ? 'older' : section(thread, now))))
  const restSaid = restKinds.size > 1 ? 'more' : [...restKinds][0]
  const working = threads.filter(inProgress).length
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
          <span className="ml-auto shrink-0 font-mono text-2xs text-fg-3">
            {working > 0 ? `${working} working` : !open ? threads.length : ''}
          </span>
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
            {(showRest || older ? [...shown, ...rest] : shown).map((thread) => (
              <ThreadItem key={thread.id} thread={thread} nested />
            ))}
          </ul>
          {rest.length > 0 && !older && (
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

/** The chats, in a project's order, with what's old, settled or snoozed behind a count, as a project's threads are (ADR 0059). */
function ChatGroup({ threads, now }: { threads: ThreadSummary[]; now: number }) {
  const [showRest, setShowRest] = useState(false)
  const ordered = [...threads].sort(inListOrder(now))
  const held = (thread: ThreadSummary) => old(thread, now) || ['snoozed', 'settled'].includes(section(thread, now))
  const shown = ordered.filter((thread) => !held(thread))
  const rest = ordered.filter(held)
  return (
    <section aria-label="chats" className="mt-2">
      <SectionLabel
        className="px-2"
        aside={
          !hosted() && (
            <Link
              to="/chat"
              aria-label="New chat"
              className="inline-flex items-center rounded-sm p-0.5 text-fg-2 hover:text-fg-1 max-md:min-h-11 max-md:px-2"
            >
              <Icon name="plus" className="h-3.5 w-3.5" />
            </Link>
          )
        }
      >
        Chats
      </SectionLabel>
      <ul>
        {(showRest ? [...shown, ...rest] : shown).map((thread) => (
          <ThreadItem key={thread.id} thread={thread} />
        ))}
      </ul>
      {rest.length > 0 && (
        <button
          type="button"
          aria-expanded={showRest}
          onClick={() => setShowRest(!showRest)}
          className="block py-1 pr-2 pl-2 text-left text-2xs text-fg-2 hover:text-fg-1 max-md:min-h-11"
        >
          {showRest ? 'Show fewer' : `Show ${rest.length} more`}
        </button>
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
    <Link to={to} className={cx(ROW, 'h-(--row)', also && ACTIVE)} activeProps={{ className: ACTIVE }}>
      <Icon name={icon} className="h-[15px] w-[15px] shrink-0" />
      <span className="flex-1 truncate">{children}</span>
      {count !== undefined && <span className={cx('font-mono text-2xs', accent ? 'text-accent-fg' : 'text-fg-2')}>{count}</span>}
    </Link>
  )
}

/** What the update strip says, and how it looks, by the update's state (ADR 0100). */
const STRIP = {
  update: { said: 'A new Silt is out', tone: 'bg-accent-tint text-accent-fg' },
  running: { said: 'Updating Silt', tone: 'bg-accent-tint text-accent-fg' },
  waiting: { said: 'Restart pending', tone: 'bg-accent-tint text-accent-fg' },
  relaunch: { said: 'Silt is updated', tone: 'bg-accent-tint text-accent-fg' },
  failed: { said: 'Update failed', tone: 'bg-danger-tint text-danger' },
}

/** The sidebar's word on an update (ADR 0100), above the machine: out, under way, waiting, done but for a relaunch, or failed. */
function UpdateStrip() {
  const update = useUpdate()
  const pill = pillOf(update.data)
  if (!pill) return null
  const { said, tone } = STRIP[pill.kind]
  const percent = update.data?.run?.percent ?? 0
  const icon =
    pill.kind === 'running' ? (
      <Spinner />
    ) : pill.kind === 'update' ? (
      <Icon name="arrowUp" className="h-3.5 w-3.5 shrink-0" />
    ) : pill.kind === 'failed' ? (
      <Icon name="alert" className="h-3.5 w-3.5 shrink-0" />
    ) : (
      <Icon name="reload" className="h-3.5 w-3.5 shrink-0" />
    )
  const relaunch = pill.kind === 'relaunch' && inDesktop()
  const body = (
    <>
      <span className="flex items-center gap-2">
        {icon}
        <span className="flex-1 truncate">{said}</span>
        {pill.kind === 'running' && <span className="font-mono text-2xs">{percent}%</span>}
        {relaunch && <span className="rounded-sm bg-accent px-2 py-px font-medium text-on-accent">Relaunch</span>}
      </span>
      {pill.kind === 'running' && (
        <span className="block h-[3px] overflow-hidden rounded-sm bg-surface-3">
          <span className="block h-full bg-accent" style={{ width: `${percent}%` }} />
        </span>
      )}
    </>
  )
  const look = cx('flex flex-col gap-1.5 rounded-md px-2 py-1.5 text-left text-xs hover:brightness-95 max-md:min-h-11', tone)
  if (relaunch) {
    return (
      <button type="button" className={look} onClick={relaunchDesktop} title="Relaunch the desktop app on the new build">
        {body}
      </button>
    )
  }
  return (
    <Link to="/settings/$section" params={{ section: 'update' }} className={look} data-testid="update-strip">
      {body}
    </Link>
  )
}

/** A machine's tile: its kind's icon, and how it's connected. */
function MachineTile({ ssh, tone, size = 26 }: { ssh: boolean; tone: DotTone | null; size?: number }) {
  return (
    <span
      className="relative inline-flex shrink-0 items-center justify-center rounded-md bg-surface-2 text-fg-1"
      style={{ width: size, height: size }}
    >
      <Icon name={ssh ? 'terminal' : 'laptop'} className="h-3.5 w-3.5" />
      {tone && <Dot tone={tone} className="absolute -right-0.5 -bottom-0.5 h-2 w-2 shadow-[0_0_0_2px_var(--bg-elevated)]" />}
    </span>
  )
}

/** How a backend stands, under its name in the machine menu. */
function backendLine(backend: Backend): { said: string; tone: DotTone } {
  if (backend.kind === 'ssh') {
    if (backend.state === 'connected') return { said: 'over SSH · connected', tone: 'ok' }
    if (backend.state === 'connecting') return { said: `over SSH · ${backend.step ?? 'connecting'}…`, tone: 'idle' }
    if (backend.state === 'failed')
      return { said: `couldn’t connect${backend.reason ? `: ${backend.reason}` : ''}`, tone: 'danger' }
    return { said: 'over SSH · not connected', tone: 'idle' }
  }
  return { said: backend.kind === 'wsl' ? 'WSL' : 'this computer', tone: 'ok' }
}

/**
 * The machine this window shows Silt from (ADRs 0045, 0108, 0115): its name and how it stands, opening
 * the others the desktop app knows, adding one, and Devices and sync.
 */
function MachineMenu({ said, failing, running }: { said: string; failing: string | null; running: number }) {
  const navigate = useNavigate()
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const { backends, current } = useBackends()
  const shown = backends.find((backend) => backend.id === current)
  const name = shown?.label ?? 'This computer'
  const device = status.data?.servers?.[0]?.name
  const server = status.data?.servers?.[0]
  const here = [device, running > 0 ? `${running} running` : 'nothing running'].filter(Boolean).join(' · ')
  const rows =
    backends.length > 0
      ? backends.map((backend) => {
          const line = backendLine(backend)
          const chosen = backend.id === current
          return {
            id: backend.id,
            label: backend.label,
            ssh: backend.kind === 'ssh',
            chosen,
            ...line,
            said: chosen ? here : line.said,
          }
        })
      : [{ id: 'here', label: 'This computer', ssh: false, chosen: true, said: here, tone: 'ok' as DotTone }]
  const go = (section: string, close: () => void) => {
    close()
    void navigate({ to: '/settings/$section', params: { section } })
  }
  return (
    <Popover
      label="Switch machine"
      className="relative block min-w-0 flex-1"
      triggerLabel={`machine: ${name}`}
      triggerTitle={failing ?? undefined}
      triggerClassName="flex w-full min-w-0 items-center gap-2 rounded-md px-1.5 py-1 text-left text-fg-1 hover:bg-surface-2 aria-expanded:bg-surface-2 max-md:min-h-12"
      panelClassName="w-[316px] max-w-[calc(100vw-20px)] p-1.5"
      trigger={
        <>
          <MachineTile ssh={shown?.kind === 'ssh'} tone={failing ? 'danger' : server ? 'ok' : 'idle'} />
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate font-medium">{name}</span>
            <span className={cx('truncate text-2xs', failing ? 'text-danger' : 'text-fg-2')}>{said}</span>
          </span>
          <Icon name="selector" className="h-3 w-3 shrink-0 text-fg-2" />
        </>
      }
    >
      {(close) => (
        <div className="flex flex-col gap-0.5">
          {rows.map((row) => (
            <button
              key={row.id}
              type="button"
              aria-current={row.chosen || undefined}
              onClick={() => {
                close()
                if (!row.chosen && row.id !== 'here') switchBackend(row.id)
              }}
              className={cx(
                'grid w-full grid-cols-[28px_minmax(0,1fr)_auto] items-center gap-2.5 rounded-sm px-2.5 py-1.5 text-left text-fg-1 hover:bg-surface-2',
                row.chosen && 'bg-accent-tint',
              )}
            >
              <MachineTile ssh={row.ssh} tone={row.tone} size={28} />
              <span className="flex min-w-0 flex-col">
                <b className="truncate font-medium">{row.label}</b>
                <span className={cx('truncate font-mono text-2xs', row.tone === 'danger' ? 'text-danger' : 'text-fg-2')}>
                  {row.said}
                </span>
              </span>
              {row.chosen ? <Icon name="check" label="shown here" className="text-accent-fg" /> : <span />}
            </button>
          ))}
          {inDesktop() && (
            <Button variant="ghost" icon="plus" className="justify-start px-2.5 py-1.5" onClick={() => go('machines', close)}>
              Add a machine
            </Button>
          )}
          <div className="mt-1 flex flex-col border-t border-line-subtle pt-1">
            <button
              type="button"
              onClick={() => go('devices', close)}
              className="flex items-center gap-2 rounded-sm px-2.5 py-1.5 text-left text-fg-1 hover:bg-surface-2 max-md:min-h-11"
            >
              <Icon name="cloud" className="h-4 w-4 shrink-0 text-fg-2" />
              <span className="flex-1">Devices and sync</span>
              {server && (
                <span className="inline-flex min-w-0 items-center gap-1.5 text-2xs text-fg-2">
                  <Dot tone={failing ? 'danger' : 'ok'} />
                  <span className="truncate">{serverName(server.url)}</span>
                </span>
              )}
            </button>
          </div>
        </div>
      )}
    </Popover>
  )
}

/** The bottom of the sidebar: an update, the machine this window shows, and Settings. */
function MachineFooter({ threads }: { threads: ThreadSummary[] }) {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const running = threads.filter(inProgress).length
  const signIn = status.data?.servers?.[0]
  const sync = status.data?.sync_error
  const { backends } = useBackends()
  const switching = backends.find((backend) => backend.switching)
  const said = [
    sync ? 'Sync failing' : signIn ? 'Synced' : 'Not synced',
    running > 0 ? `${running} running` : 'nothing running',
  ].join(' · ')
  return (
    <div className="flex flex-col gap-1 border-t border-line pt-1.5">
      {!hosted() && <UpdateStrip />}
      <div className="flex items-center gap-1">
        {hosted() ? (
          <span className="flex min-w-0 flex-1 items-center gap-2 px-1.5 py-1">
            <span className="inline-flex h-[26px] w-[26px] shrink-0 items-center justify-center rounded-md bg-surface-2">
              <Icon name="cloud" className="h-3.5 w-3.5" />
            </span>
            <span className="truncate font-medium">On the server</span>
          </span>
        ) : (
          <MachineMenu said={said} failing={sync ?? null} running={running} />
        )}
        <Link
          to="/settings"
          aria-label="Settings"
          title={`Settings (${formatKeys('mod+,')})`}
          className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-sm text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11 max-md:min-w-11"
          activeProps={{ className: 'bg-accent-tint text-accent-fg' }}
        >
          <Icon name="settings" className="h-[15px] w-[15px]" />
        </Link>
      </div>
      {switching && (
        <div role="status" className="flex items-center gap-2 px-2 text-xs text-fg-2">
          <span className="inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center">
            <Spinner />
          </span>
          <span className="truncate">Connecting to {switching.label}…</span>
        </div>
      )}
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
  // Chats have a section of their own above the projects (ADR 0059).
  const chats = listed.filter((thread) => isChat(thread.repo))
  const inProjects = listed.filter((thread) => !isChat(thread.repo))
  const [showOlder, setShowOlder] = useState(false)
  const byProject = projects(inProjects)
  const isOlder = ([, inRepo]: [string, ThreadSummary[]]) => inRepo.every((thread) => old(thread, now))
  const recentProjects = byProject.filter((project) => !isOlder(project))
  const olderProjects = byProject.filter(isOlder)
  const path = useRouterState({ select: (state) => state.location.pathname })
  const reviews = mine.filter((thread) => thread.review).length
  const unreadInbox = (inbox.data ?? []).filter((entry) => !entry.read).length
  return (
    <>
      <nav
        aria-label="projects"
        className="flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2 pt-2.5 pb-2 text-ui [&>*]:shrink-0"
      >
        {/* The workspace is the sidebar's title: everything under it is in it (ADR 0115). */}
        <div className="flex items-center pb-2">
          {hosted() ? (
            <Link to="/" className="px-1.5 py-1 text-heading font-medium tracking-[-0.04em] text-fg-1">
              silt
            </Link>
          ) : (
            <WorkspaceSwitcher />
          )}
        </div>
        {!hosted() && (
          <Link to="/new" className={cx(ROW, 'h-(--row)', (path === '/' || path === '/new') && ACTIVE)}>
            <Icon name="plus" className="h-[15px] w-[15px] shrink-0" />
            <span className="flex-1">New thread</span>
            <Kbd>{formatKeys('n')}</Kbd>
          </Link>
        )}
        {!hosted() && (
          <Link to="/chat" className={cx(ROW, 'h-(--row)')} activeProps={{ className: ACTIVE }}>
            <Icon name="comment" className="h-[15px] w-[15px] shrink-0" />
            <span className="flex-1">New chat</span>
            <Kbd>{formatKeys('c')}</Kbd>
          </Link>
        )}
        <button type="button" title="Search and commands" onClick={openPalette} className={cx(ROW, 'h-(--row) w-full text-left')}>
          <Icon name="search" className="h-[15px] w-[15px] shrink-0" />
          <span className="flex-1">Search</span>
          <Kbd>{formatKeys('mod+k')}</Kbd>
        </button>
        <NavItem to="/threads" icon="list">
          All threads
        </NavItem>
        <NavItem to="/artifacts" icon="image">
          Artifacts
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
            <SectionLabel
              className="px-2 pt-1 text-accent-fg"
              aside={<span className="font-mono">{grouped.needs_you.length}</span>}
            >
              Needs you
            </SectionLabel>
            <ul>
              {grouped.needs_you.map((thread) => (
                <NeedsYouItem key={thread.id} thread={thread} />
              ))}
            </ul>
            <Link
              to="/needs-you"
              className={cx(
                'flex items-center justify-between rounded-sm px-2 py-1.5 text-xs text-accent-fg hover:bg-elevated max-md:min-h-11',
                path === '/needs-you' && 'bg-elevated',
              )}
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
              {[...grouped.pinned].sort(inListOrder(now)).map((thread) => (
                <ThreadItem key={thread.id} thread={thread} />
              ))}
            </ul>
          </section>
        )}
        {chats.length > 0 && <ChatGroup threads={chats} now={now} />}
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
        {threads.isSuccess && inProjects.length === 0 && grouped.needs_you.length === 0 && (
          <p className="px-2 py-1 text-xs text-fg-2">None yet.</p>
        )}
        {recentProjects.map(([repo, inRepo]) => (
          <ProjectGroup key={repo} repo={repo} name={nameOf(repo)} clone={cloneOf(repo)} threads={inRepo} now={now} />
        ))}
        {olderProjects.length > 0 && (
          <button
            type="button"
            aria-expanded={showOlder}
            onClick={() => setShowOlder(!showOlder)}
            title="Projects with no thread touched in the last two weeks"
            className="flex items-center gap-1.5 rounded-sm px-2 py-1 text-left text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11"
          >
            <Icon name={showOlder ? 'chevronDown' : 'chevronRight'} className="h-3 w-3 shrink-0" />
            <span className="truncate">Older projects</span>
            <span className="ml-auto shrink-0 font-mono text-2xs text-fg-3">{olderProjects.length}</span>
          </button>
        )}
        {showOlder &&
          olderProjects.map(([repo, inRepo]) => (
            <ProjectGroup key={repo} repo={repo} name={nameOf(repo)} clone={cloneOf(repo)} threads={inRepo} now={now} older />
          ))}
      </nav>
      {/* Outside the list, so a long one never scrolls Settings and this machine out of reach. */}
      <div className="shrink-0 px-2 pb-2">
        <MachineFooter threads={mine} />
      </div>
    </>
  )
}
