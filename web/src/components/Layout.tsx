import { useQuery } from '@tanstack/react-query'
import { Link, Outlet, useNavigate, useRouterState } from '@tanstack/react-router'
import { type ReactNode, useLayoutEffect, useRef, useState } from 'react'
import { api } from '../api'
import { useBusConnected } from '../bus'
import { useDragWidth } from '../dragWidth'
import { useNotifier } from '../notifier'
import { keys } from '../query'
import { MainScroll } from '../scroll'
import { Drawer } from '../shell'
import { hosted } from '../served'
import { useActions } from '../shortcuts'
import { useTextActions, useTextSync } from '../text'
import { THEMES, useAppearance, useThemeSync } from '../theme'
import { Dot, Icon, Toaster, cx } from '../ui'
import { useLanUnpaired } from '../lan'
import { LanAskPrompt, LanUnpaired } from './LanPhones'
import { ErrorBoundary } from './ErrorBoundary'
import { Palette } from './Palette'
import { Sidebar } from './Sidebar'
import { SwitchFailures } from './Machines'

/**
 * The themes from the palette (ADR 0098): the next mode, and each theme,
 * previewed while it's highlighted; Settings › Appearance has them too.
 */
function useThemeActions() {
  useThemeSync()
  useTextSync()
  const navigate = useNavigate()
  useTextActions(() => void navigate({ to: '/settings/$section', params: { section: 'text' } }))
  const { catalog, settings, save } = useAppearance()
  const next = THEMES[(THEMES.indexOf(settings.mode) + 1) % THEMES.length]
  const themes = Array.isArray(catalog.data?.themes) ? catalog.data.themes : []
  useActions([
    { id: 'theme', label: `Switch to the ${next} theme`, run: () => void save({ ...settings, mode: next }) },
    ...themes.map((theme) => ({
      id: `theme:${theme.id}`,
      label: `Theme: ${theme.name}`,
      hint: theme.appearance,
      preview: { theme: theme.id, appearance: theme.appearance },
      run: () => void save(theme.appearance === 'light' ? { ...settings, light: theme.id } : { ...settings, dark: theme.id }),
    })),
  ])
}

/** Where anyone can go from anywhere, by key or from the palette. */
function useGlobalActions() {
  const navigate = useNavigate()
  useActions([
    ...(hosted()
      ? []
      : [
          { id: 'new', label: 'New thread in this project', keys: 'n', run: () => void navigate({ to: '/new', search: {} }) },
          {
            id: 'new-elsewhere',
            label: 'New thread in another project',
            keys: 'shift+n',
            run: () => void navigate({ to: '/new', search: { pick: 'projects' } }),
          },
          { id: 'new-chat', label: 'New chat', keys: 'c', run: () => void navigate({ to: '/chat' }) },
        ]),
    { id: 'threads', label: 'All threads', keys: 't', run: () => void navigate({ to: '/threads' }) },
    { id: 'artifacts', label: 'Artifacts', run: () => void navigate({ to: '/artifacts' }) },
    { id: 'settings', label: 'Settings', run: () => void navigate({ to: '/settings' }) },
    { id: 'projects', label: 'Projects', keys: 'p', run: () => void navigate({ to: '/settings/projects' }) },
    { id: 'orgs', label: 'Orgs', run: () => void navigate({ to: '/settings/orgs' }) },
    { id: 'devices', label: 'Devices and sync', run: () => void navigate({ to: '/settings/devices' }) },
    { id: 'graph', label: 'Graph', keys: 'g', run: () => void navigate({ to: '/graph' }) },
    { id: 'search', label: 'Search everything', keys: 'mod+shift+f', run: () => void navigate({ to: '/search' }) },
    { id: 'mrs', label: 'Merge requests', keys: 'm', run: () => void navigate({ to: '/mrs' }) },
    { id: 'inbox', label: 'Inbox', keys: 'i', run: () => void navigate({ to: '/inbox' }) },
    { id: 'needs-you', label: 'What needs you', keys: 'q', run: () => void navigate({ to: '/needs-you' }) },
  ])
}

const SIDEBAR = { min: 200, max: 420, start: 260 }

function Tab({
  to,
  icon,
  children,
  at,
}: {
  to: string
  icon: 'inbox' | 'list' | 'merge' | 'search'
  children: ReactNode
  at: (path: string) => boolean
}) {
  const current = at(useRouterState({ select: (state) => state.location.pathname }))
  return (
    <Link
      to={to}
      aria-current={current ? 'page' : undefined}
      className={cx(
        'flex min-h-14 flex-col items-center justify-center gap-0.5 text-2xs',
        current ? 'text-accent-fg' : 'text-fg-2',
      )}
    >
      <Icon name={icon} className="h-5 w-5" />
      <span>{children}</span>
    </Link>
  )
}

/** A phone's way around (ADR 0044): threads, reviews, the inbox and search. */
function TabBar() {
  const inbox = useQuery({ queryKey: keys.inbox(false), queryFn: () => api.inbox(false), retry: false })
  const unread = (inbox.data ?? []).filter((entry) => !entry.read).length
  return (
    <nav aria-label="tabs" className="grid grid-cols-4 border-t border-line bg-bg pb-[env(safe-area-inset-bottom)] md:hidden">
      <Tab to="/" icon="list" at={(path) => path === '/' || path === '/threads' || path.startsWith('/t/')}>
        Threads
      </Tab>
      <Tab to="/mrs" icon="merge" at={(path) => path === '/mrs' || path.startsWith('/mr/')}>
        Reviews
      </Tab>
      <Tab to="/inbox" icon="inbox" at={(path) => path === '/inbox'}>
        <span>
          Inbox
          {unread > 0 && <span className="ml-1 rounded-full bg-accent px-1 text-on-accent">{unread}</span>}
        </span>
      </Tab>
      <Tab to="/search" icon="search" at={(path) => path === '/search'}>
        Search
      </Tab>
    </nav>
  )
}

/** Only while the connection to the daemon or server is down. */
function Reconnecting() {
  const connected = useBusConnected()
  if (connected) return null
  return (
    <p
      role="status"
      className="flex items-center gap-1.5 border-b border-line-subtle bg-danger-tint px-4 py-1 text-xs text-danger"
    >
      <Dot tone="danger" />
      {hosted() ? 'Server' : 'Daemon'} not connected; reconnecting
    </p>
  )
}

/**
 * The shell every screen sits in (ADR 0044): the sidebar and the page, with
 * no top bar or status bar; each page has its own header. On a phone the
 * sidebar is a drawer, and a tab bar runs along the bottom.
 */
export function Layout() {
  useGlobalActions()
  useThemeActions()
  useNotifier()
  const unpaired = useLanUnpaired()
  const aside = useRef<HTMLElement>(null)
  const sidebar = useDragWidth('silt-sidebar-width', SIDEBAR, 'right', '--sidebar', aside)
  const scroll = useRef<HTMLElement>(null)
  const path = useRouterState({ select: (state) => state.location.pathname })
  // The drawer was opened on this page; going elsewhere closes it.
  const [drawerOn, setDrawerOn] = useState<string | null>(null)
  const sidebarShown = drawerOn === path
  // Each page opens at its top, not where the one before was scrolled to; a thread then goes to its end, or
  // where a link names, itself. Before the page's own effects run, so it never undoes theirs.
  useLayoutEffect(() => {
    if (scroll.current) scroll.current.scrollTop = 0
  }, [path])
  const setSidebarShown = (shown: boolean) => setDrawerOn(shown ? path : null)
  return (
    <div className="grid h-dvh grid-cols-[minmax(0,1fr)] grid-rows-[minmax(0,1fr)_auto] bg-bg text-fg-1 md:grid-rows-[minmax(0,1fr)]">
      <div className="flex min-h-0">
        <aside
          ref={aside}
          style={sidebar.style}
          className={cx(
            'w-(--sidebar) shrink-0 flex-col border-r border-line bg-bg',
            sidebarShown
              ? 'fixed inset-y-0 left-0 z-30 flex max-w-[85vw] shadow-overlay md:static md:shadow-none'
              : 'hidden md:flex',
          )}
        >
          <ErrorBoundary>
            <Sidebar />
          </ErrorBoundary>
        </aside>
        <div
          {...sidebar.handle}
          aria-label="resize projects"
          className="-ml-px hidden w-1 shrink-0 cursor-col-resize hover:bg-accent-tint focus-visible:bg-accent md:block"
        />
        {sidebarShown && (
          <button
            type="button"
            aria-label="close projects"
            onClick={() => setSidebarShown(false)}
            className="fixed inset-0 z-20 bg-backdrop md:hidden"
          />
        )}
        <main ref={scroll} inert={sidebarShown || undefined} className="relative flex min-w-0 flex-1 flex-col overflow-y-auto">
          <Reconnecting />
          <ErrorBoundary>
            <MainScroll.Provider value={scroll}>
              <Drawer.Provider value={{ open: sidebarShown, toggle: () => setSidebarShown(!sidebarShown) }}>
                {unpaired ? <LanUnpaired /> : <Outlet />}
              </Drawer.Provider>
            </MainScroll.Provider>
          </ErrorBoundary>
        </main>
      </div>
      <TabBar />
      <Palette />
      <Toaster />
      <SwitchFailures />
      <LanAskPrompt />
    </div>
  )
}
