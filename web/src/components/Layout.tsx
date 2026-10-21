import { Link, Outlet, useParams } from '@tanstack/react-router'
import { useState } from 'react'
import { THEMES, applyTheme, storedTheme, type Theme } from '../theme'
import { MergeRequest } from './MergeRequests'
import { ThreadView } from './ThreadView'

function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(storedTheme)
  const next = THEMES[(THEMES.indexOf(theme) + 1) % THEMES.length]
  return (
    <button
      className="ml-auto rounded-sm border border-line px-2 py-0.5 text-xs text-fg-2 hover:bg-surface-2"
      aria-label={`theme: ${theme}; switch to ${next}`}
      onClick={() => {
        applyTheme(next)
        setTheme(next)
      }}
    >
      {theme === 'system' ? 'System theme' : theme === 'light' ? 'Light' : 'Dark'}
    </button>
  )
}

export function Layout() {
  return (
    <main className="mx-auto max-w-6xl p-6">
      <header className="mb-6 flex items-baseline gap-6 border-b border-line pb-3">
        <h1 className="text-2xl font-semibold">Silt</h1>
        <nav className="flex gap-4 text-sm">
          <Link to="/" className="text-accent-fg">
            Threads
          </Link>
          <Link to="/mrs" className="text-accent-fg">
            Merge requests
          </Link>
          <Link to="/inbox" className="text-accent-fg">
            Inbox
          </Link>
        </nav>
        <ThemeToggle />
      </header>
      <Outlet />
    </main>
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
