import { useQuery } from '@tanstack/react-query'
import { Link, Outlet, useMatch, useNavigate, useParams } from '@tanstack/react-router'
import { type KeyboardEvent, type PointerEvent, useRef, useState } from 'react'
import { api, short } from '../api'
import { useNotifier } from '../notifier'
import { projectName } from '../projects'
import { keys } from '../query'
import { MainScroll } from '../scroll'
import { useActions } from '../shortcuts'
import { THEMES, applyTheme, storedTheme, type Theme } from '../theme'
import { IconButton, Toaster, cx } from '../ui'
import { ErrorBoundary } from './ErrorBoundary'
import { MergeRequest } from './MergeRequests'
import { Palette } from './Palette'
import { Sidebar } from './Sidebar'
import { ThreadView } from './ThreadView'

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
    { id: 'mrs', label: 'Merge requests', keys: 'm', run: () => void navigate({ to: '/mrs' }) },
    { id: 'inbox', label: 'Inbox', keys: 'i', run: () => void navigate({ to: '/inbox' }) },
  ])
}

/** Where this page is: its project and thread, or which list. */
function Breadcrumbs() {
  const thread = useMatch({ from: '/t/$id', shouldThrow: false })
  const mr = useMatch({ from: '/mr/$id', shouldThrow: false })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads, enabled: thread !== undefined })
  const shown = thread && threads.data?.find((summary) => summary.id === thread.params.id)
  const crumbs: string[] = thread
    ? [shown ? projectName(shown.repo || 'no remote') : '', shown?.title ?? short(thread.params.id)]
    : mr
      ? ['Merge request', short(mr.params.id)]
      : []
  return (
    <ol aria-label="breadcrumbs" className="flex min-w-0 items-center gap-2 text-xs text-fg-2">
      <li>
        <Link to="/" className="font-medium text-fg-1">
          Silt
        </Link>
      </li>
      {crumbs.filter(Boolean).map((crumb, index) => (
        <li key={index} className="flex min-w-0 items-center gap-2">
          <span aria-hidden="true" className="text-fg-3">
            /
          </span>
          <span className="truncate">{crumb}</span>
        </li>
      ))}
    </ol>
  )
}

const SIDEBAR_KEY = 'silt-sidebar-width'
const SIDEBAR = { min: 180, max: 420, start: 240 }

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

export function Layout() {
  useGlobalActions()
  useNotifier()
  const sidebar = useSidebarWidth()
  const [sidebarShown, setSidebarShown] = useState(false)
  const scroll = useRef<HTMLElement>(null)
  return (
    <div className="grid h-dvh grid-rows-[40px_1fr] bg-bg text-fg-1">
      <header className="flex min-w-0 items-center gap-3 border-b border-line px-3">
        <IconButton
          icon="thread"
          label={sidebarShown ? 'hide projects' : 'show projects'}
          className="md:hidden"
          aria-expanded={sidebarShown}
          onClick={() => setSidebarShown(!sidebarShown)}
        />
        <Breadcrumbs />
        <nav className="ml-auto flex items-center gap-4 text-xs">
          <Palette />
          <Link to="/mrs" className="text-fg-2 hover:text-fg-1" activeProps={{ className: 'text-accent-fg' }}>
            Merge requests
          </Link>
          <Link to="/inbox" className="text-fg-2 hover:text-fg-1" activeProps={{ className: 'text-accent-fg' }}>
            Inbox
          </Link>
          <Link to="/orgs" className="text-fg-2 hover:text-fg-1" activeProps={{ className: 'text-accent-fg' }}>
            Orgs
          </Link>
          <Link to="/devices" className="text-fg-2 hover:text-fg-1" activeProps={{ className: 'text-accent-fg' }}>
            Devices
          </Link>
          <ThemeToggle />
        </nav>
      </header>
      <div className="flex min-h-0">
        <aside
          style={{ width: sidebar.width }}
          className={cx(
            'shrink-0 flex-col border-r border-line bg-bg',
            sidebarShown ? 'fixed inset-y-10 left-0 z-30 flex shadow-overlay md:static md:shadow-none' : 'hidden md:flex',
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
          className="hidden w-1 shrink-0 cursor-col-resize hover:bg-accent-tint focus-visible:bg-accent md:block"
        />
        <main ref={scroll} className="relative min-w-0 flex-1 overflow-y-auto">
          <div className="mx-auto max-w-6xl p-6">
            <ErrorBoundary>
              <MainScroll.Provider value={scroll}>
                <Outlet />
              </MainScroll.Provider>
            </ErrorBoundary>
          </div>
        </main>
      </div>
      <Toaster />
    </div>
  )
}

export function ThreadPage() {
  const { id } = useParams({ from: '/t/$id' })
  return <ThreadView id={id} />
}

export function MergeRequestPage() {
  const { id } = useParams({ from: '/mr/$id' })
  return <MergeRequest id={id} />
}
