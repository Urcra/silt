import { useQuery } from '@tanstack/react-query'
import { Link, Outlet, useMatch, useNavigate, useRouterState } from '@tanstack/react-router'
import { type KeyboardEvent, type PointerEvent, type ReactNode, useRef, useState } from 'react'
import { api, short } from '../api'
import { useBusConnected } from '../bus'
import { useNotifier } from '../notifier'
import { useNow } from '../now'
import { projectName } from '../projects'
import { keys } from '../query'
import { MainScroll } from '../scroll'
import { sections } from '../sections'
import { useActions } from '../shortcuts'
import { hosted, serverName } from '../served'
import { THEMES, applyTheme, storedTheme, type Theme } from '../theme'
import { Avatar, Dot, Icon, IconButton, Pill, StatusBar, Toaster, cx } from '../ui'
import { useWorkspace } from '../workspace'
import { useInWorkspace } from '../workspaceFilter'
import { ErrorBoundary } from './ErrorBoundary'
import { Palette } from './Palette'
import { Sidebar } from './Sidebar'

const THEME_ICON = { system: 'monitor', light: 'sun', dark: 'moon' } as const

function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(storedTheme)
  const next = THEMES[(THEMES.indexOf(theme) + 1) % THEMES.length]
  const switchTheme = () => {
    applyTheme(next)
    setTheme(next)
  }
  useActions([{ id: 'theme', label: `Switch to the ${next} theme`, run: switchTheme }])
  return <IconButton icon={THEME_ICON[theme]} label={`${theme} theme; switch to ${next}`} onClick={switchTheme} />
}

/** Where anyone can go from anywhere, by key or from the palette. */
function useGlobalActions() {
  const navigate = useNavigate()
  useActions([
    { id: 'new', label: 'New thread', keys: 'n', run: () => void navigate({ to: '/new' }) },
    { id: 'threads', label: 'All threads', keys: 't', run: () => void navigate({ to: '/' }) },
    { id: 'projects', label: 'Projects', keys: 'p', run: () => void navigate({ to: '/projects' }) },
    { id: 'orgs', label: 'Orgs', run: () => void navigate({ to: '/orgs' }) },
    { id: 'devices', label: 'Devices', run: () => void navigate({ to: '/devices' }) },
    { id: 'graph', label: 'Graph', keys: 'g', run: () => void navigate({ to: '/graph' }) },
    { id: 'search', label: 'Search everything', keys: 'mod+shift+f', run: () => void navigate({ to: '/search' }) },
    { id: 'mrs', label: 'Merge requests', keys: 'm', run: () => void navigate({ to: '/mrs' }) },
    { id: 'inbox', label: 'Inbox', keys: 'i', run: () => void navigate({ to: '/inbox' }) },
  ])
}

const PAGES: Record<string, string> = {
  '/': 'Threads',
  '/new': 'New thread',
  '/projects': 'Projects',
  '/orgs': 'Orgs',
  '/devices': 'Devices',
  '/needs-you': 'Needs you',
  '/graph': 'History graph',
  '/search': 'Search',
  '/mrs': 'Merge requests',
  '/inbox': 'Inbox',
}

/** Where this page is: the workspace, then its project and thread, or which list. */
function Crumbs() {
  const threadPage = useMatch({ from: '/t/$id', shouldThrow: false })
  const filePage = useMatch({ from: '/t/$id/files/$', shouldThrow: false })
  const livePage = useMatch({ from: '/t/$id/live', shouldThrow: false })
  const reconcilePage = useMatch({ from: '/t/$id/reconcile', shouldThrow: false })
  const thread = threadPage ?? filePage ?? livePage ?? reconcilePage
  const mr = useMatch({ from: '/mr/$id', shouldThrow: false })
  const mrs = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests, enabled: mr !== undefined })
  const mrTitle = mr && mrs.data?.find((candidate) => candidate.id === mr.params.id)?.title
  const path = useRouterState({ select: (state) => state.location.pathname })
  const chosen = useWorkspace()
  const workspace =
    chosen.kind === 'server' ? (chosen.org ?? serverName(chosen.server)) : chosen.kind === 'local' ? 'This machine' : ''
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads, enabled: thread !== undefined })
  const shown = thread && threads.data?.find((summary) => summary.id.endsWith(thread.params.id))
  const crumbs: { text: string; mono?: boolean }[] = thread
    ? [
        { text: shown ? projectName(shown.repo || 'no remote') : '', mono: true },
        { text: shown?.title ?? short(thread.params.id) },
        { text: livePage ? 'Live' : reconcilePage ? 'Reconcile' : '' },
      ]
    : mr
      ? [{ text: 'Merge requests' }, mrTitle ? { text: mrTitle } : { text: short(mr.params.id), mono: true }]
      : [{ text: workspace }, { text: PAGES[path.replace(/\/$/, '') || '/'] ?? '' }]
  const shownCrumbs = crumbs.filter((crumb) => crumb.text)
  return (
    <ol aria-label="breadcrumbs" className="flex min-w-0 items-center gap-2 text-xs text-fg-2">
      <li>
        <Link to="/" className="text-[15px] font-medium tracking-[-0.04em] text-fg-1">
          silt
        </Link>
      </li>
      {shownCrumbs.map((crumb, index) => (
        <li key={index} className={cx('flex min-w-0 items-center gap-2', index < shownCrumbs.length - 1 && 'hidden md:flex')}>
          <span aria-hidden="true" className="text-fg-3">
            /
          </span>
          <span
            className={cx('truncate', crumb.mono && 'font-mono', index === shownCrumbs.length - 1 && 'font-medium text-fg-1')}
          >
            {crumb.text}
          </span>
        </li>
      ))}
    </ol>
  )
}

/** Who is signed in, and on which machine, at the top bar's right. */
function Identity() {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const signIn = status.data?.servers?.[0]
  const device = signIn?.name ?? 'this machine'
  return (
    <>
      {!hosted() && (
        <Link to="/devices" title={signIn ? `${device} on ${serverName(signIn.url)}` : 'Not signed in to a server'} className="hidden md:inline-flex">
          <Pill className="hover:text-fg-1">
            <Dot tone={signIn && !signIn.sync_error ? 'ok' : 'idle'} />
            {device}
          </Pill>
        </Link>
      )}
      {signIn && <Avatar name={signIn.user} size={24} />}
    </>
  )
}

/** The status bar's account of the connection, the agents at work and the version. */
function AppStatus() {
  const connected = useBusConnected()
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const running = (threads.data ?? []).filter((thread) =>
    ['working', 'starting', 'awaiting_permission', 'awaiting_input'].includes(thread.agent_state ?? ''),
  ).length
  const waiting = (threads.data ?? []).filter((thread) => ['awaiting_permission', 'awaiting_input'].includes(thread.agent_state ?? '')).length
  const sync = status.data?.sync_error
  return (
    <StatusBar label="status" className="hidden md:flex">
      <span className="inline-flex items-center gap-1">
        <Dot tone={connected ? 'ok' : 'danger'} />
        {hosted() ? 'server' : 'daemon'} {connected ? 'connected' : 'reconnecting'}
      </span>
      {sync && (
        <span className="inline-flex min-w-0 items-center gap-1 text-danger" title={sync}>
          <Dot tone="danger" />
          sync failing
        </span>
      )}
      <span>{running === 1 ? '1 agent running' : `${running} agents running`}</span>
      {waiting > 0 && <span className="text-accent-fg">{waiting} waiting on you</span>}
      {status.data?.version && <span className="ml-auto">silt {status.data.version}</span>}
    </StatusBar>
  )
}

const SIDEBAR_KEY = 'silt-sidebar-width'
const SIDEBAR = { min: 200, max: 420, start: 260 }

function storedWidth(): number {
  try {
    const stored = Number(localStorage.getItem(SIDEBAR_KEY))
    return stored >= SIDEBAR.min && stored <= SIDEBAR.max ? stored : SIDEBAR.start
  } catch {
    return SIDEBAR.start
  }
}

/** The sidebar's width, dragged or moved with the arrow keys on its edge. */
function useSidebarWidth() {
  const [width, setWidth] = useState(storedWidth)
  const set = (next: number) => {
    const clamped = Math.min(SIDEBAR.max, Math.max(SIDEBAR.min, Math.round(next)))
    setWidth(clamped)
    try {
      localStorage.setItem(SIDEBAR_KEY, String(clamped))
    } catch {
      // The width then lasts for this page only.
    }
  }
  const drag = (event: PointerEvent<HTMLDivElement>) => {
    const start = { x: event.clientX, width }
    const handle = event.currentTarget
    handle.setPointerCapture(event.pointerId)
    const move = (moved: globalThis.PointerEvent) => set(start.width + moved.clientX - start.x)
    const stop = () => {
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('pointerup', stop)
    }
    handle.addEventListener('pointermove', move)
    handle.addEventListener('pointerup', stop)
  }
  const key = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = { ArrowLeft: -16, ArrowRight: 16 }[event.key]
    if (step) {
      event.preventDefault()
      set(width + step)
    }
  }
  return { width, drag, key }
}

function Tab({ to, icon, children, exact = false }: { to: string; icon: 'alert' | 'thread' | 'merge' | 'search'; children: ReactNode; exact?: boolean }) {
  return (
    <Link
      to={to}
      activeOptions={{ exact }}
      className="flex min-h-14 flex-col items-center justify-center gap-0.5 text-2xs text-fg-2"
      activeProps={{ className: 'text-accent-fg' }}
    >
      <Icon name={icon} className="h-5 w-5" />
      <span>{children}</span>
    </Link>
  )
}

/** A phone's way around: what waits on the person first (ADR 0028). */
function TabBar() {
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const shown = useInWorkspace()
  const waiting = sections((threads.data ?? []).filter(shown), useNow()).needs_you.length
  return (
    <nav
      aria-label="tabs"
      className="grid grid-cols-4 border-t border-line bg-bg pb-[env(safe-area-inset-bottom)] md:hidden"
    >
      <Tab to="/needs-you" icon="alert">
        Needs you
        {waiting > 0 && <span className="ml-1 rounded-full bg-accent px-1 text-on-accent">{waiting}</span>}
      </Tab>
      <Tab to="/" icon="thread" exact>
        Threads
      </Tab>
      <Tab to="/mrs" icon="merge">
        Reviews
      </Tab>
      <Tab to="/search" icon="search">
        Search
      </Tab>
    </nav>
  )
}

/**
 * The shell every screen sits in (ADR 0035): the top bar, the project
 * sidebar, the view, and the status bar; on a phone, the sidebar is a
 * drawer and a tab bar takes the status bar's place.
 */
export function Layout() {
  useGlobalActions()
  useNotifier()
  const sidebar = useSidebarWidth()
  const scroll = useRef<HTMLElement>(null)
  const path = useRouterState({ select: (state) => state.location.pathname })
  // The drawer was opened on this page; going elsewhere closes it.
  const [drawerOn, setDrawerOn] = useState<string | null>(null)
  const sidebarShown = drawerOn === path
  const setSidebarShown = (shown: boolean) => setDrawerOn(shown ? path : null)
  return (
    <div className="grid h-dvh grid-cols-[minmax(0,1fr)] grid-rows-[40px_minmax(0,1fr)_auto] bg-bg text-fg-1">
      <header className="flex min-w-0 items-center gap-3 border-b border-line px-3 text-xs">
        <IconButton
          icon="thread"
          label={sidebarShown ? 'hide projects' : 'show projects'}
          className="md:hidden"
          aria-expanded={sidebarShown}
          onClick={() => setSidebarShown(!sidebarShown)}
        />
        <Crumbs />
        <div className="ml-auto flex items-center gap-2">
          <Palette />
          <ThemeToggle />
          <Identity />
        </div>
      </header>
      <div className="flex min-h-0">
        <aside
          style={{ width: sidebar.width }}
          className={cx(
            'shrink-0 flex-col border-r border-line bg-bg',
            sidebarShown ? 'fixed top-10 bottom-0 left-0 z-30 flex max-w-[85vw] shadow-overlay md:static md:shadow-none' : 'hidden md:flex',
          )}
        >
          <ErrorBoundary>
            <Sidebar />
          </ErrorBoundary>
        </aside>
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="resize projects"
          aria-valuenow={sidebar.width}
          aria-valuemin={SIDEBAR.min}
          aria-valuemax={SIDEBAR.max}
          tabIndex={0}
          onPointerDown={sidebar.drag}
          onKeyDown={sidebar.key}
          className="-ml-px hidden w-1 shrink-0 cursor-col-resize hover:bg-accent-tint focus-visible:bg-accent md:block"
        />
        {sidebarShown && (
          <button
            type="button"
            aria-label="close projects"
            onClick={() => setSidebarShown(false)}
            className="fixed inset-0 top-10 z-20 bg-backdrop md:hidden"
          />
        )}
        <main ref={scroll} inert={sidebarShown || undefined} className="relative flex min-w-0 flex-1 flex-col overflow-y-auto">
          <ErrorBoundary>
            <MainScroll.Provider value={scroll}>
              <Outlet />
            </MainScroll.Provider>
          </ErrorBoundary>
        </main>
      </div>
      <AppStatus />
      <TabBar />
      <Toaster />
    </div>
  )
}
