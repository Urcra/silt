import { useQuery } from '@tanstack/react-query'
import { Link, useParams } from '@tanstack/react-router'
import { type ReactNode, useState } from 'react'
import { api } from '../api'
import { keys } from '../query'
import { hosted, serverName } from '../served'
import { THEMES, type Theme, applyTheme, storedTheme } from '../theme'
import { Dot, EmptyState, cx } from '../ui'
import { Devices } from './Devices'
import { NotificationBanner, NotificationSettings } from './Inbox'
import { Orgs } from './Orgs'
import { InSettings } from '../settingsContext'
import { Page } from './Page'
import { AddProjectPage, Projects } from './Projects'

const SECTIONS = [
  { id: 'projects', label: 'Projects' },
  { id: 'devices', label: 'Devices' },
  { id: 'sync', label: 'Sync' },
  { id: 'orgs', label: 'Orgs' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'appearance', label: 'Appearance' },
] as const

/** The servers this machine syncs with, and how to add one (ADR 0032). */
function Sync() {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const servers = status.data?.servers ?? []
  return (
    <section aria-label="Sync" className="flex flex-col gap-3">
      <h2 className="text-sm font-medium">Sync</h2>
      {servers.length === 0 && (
        <EmptyState title="Threads stay on this machine">
          Sign in to a server to hand threads off and share them: <code className="font-mono">silt auth login --server &lt;url&gt;</code>
        </EmptyState>
      )}
      {servers.length > 0 && (
        <ul className="overflow-hidden rounded-md border border-line bg-elevated">
          {servers.map((server) => (
            <li key={server.url} className="flex flex-wrap items-center gap-2 border-b border-line-subtle px-4 py-3 text-sm last:border-b-0">
              <Dot tone={status.data?.sync_error ? 'danger' : 'ok'} />
              <span className="font-medium">{serverName(server.url)}</span>
              <span className="font-mono text-2xs text-fg-2">
                {server.name} · signed in as {server.user}
              </span>
            </li>
          ))}
        </ul>
      )}
      {status.data?.sync_error && <p className="text-xs text-danger">{status.data.sync_error}</p>}
      <p className="text-xs text-fg-2">
        Another server: <code className="font-mono">silt auth login --server &lt;url&gt;</code>. Threads kept only on this machine go to a server with{' '}
        <code className="font-mono">silt send</code>.
      </p>
    </section>
  )
}

function Appearance() {
  const [theme, setTheme] = useState<Theme>(storedTheme)
  return (
    <section aria-label="Appearance" className="flex flex-col gap-3">
      <h2 className="text-sm font-medium">Appearance</h2>
      <div role="radiogroup" aria-label="theme" className="inline-flex self-start overflow-hidden rounded-sm border border-line text-xs">
        {THEMES.map((choice) => (
          <button
            key={choice}
            type="button"
            role="radio"
            aria-checked={theme === choice}
            onClick={() => {
              applyTheme(choice)
              setTheme(choice)
            }}
            className={cx('px-3 py-1 capitalize max-md:min-h-11', theme === choice ? 'bg-accent-tint text-accent-fg' : 'text-fg-2 hover:bg-surface-2')}
          >
            {choice}
          </button>
        ))}
      </div>
    </section>
  )
}

function Notifications() {
  return (
    <section aria-label="Notifications" className="flex flex-col gap-3">
      <h2 className="text-sm font-medium">Notifications</h2>
      <NotificationBanner />
      <NotificationSettings open />
    </section>
  )
}

/** One page for what was Projects, Devices and Orgs, and the rest of the settings (ADR 0044). */
export function Settings() {
  const { section } = useParams({ strict: false }) as { section?: string }
  const chosen = section === 'add' ? 'projects' : (SECTIONS.find((candidate) => candidate.id === section)?.id ?? 'projects')
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const signIn = status.data?.servers?.[0]
  const bodies: Record<string, ReactNode> = {
    projects: section === 'add' ? <AddProjectPage /> : <Projects />,
    devices: <Devices />,
    sync: <Sync />,
    orgs: <Orgs />,
    notifications: <Notifications />,
    appearance: <Appearance />,
  }
  return (
    <Page title="Settings" meta={signIn ? `${signIn.name} · signed in to ${serverName(signIn.url)} as ${signIn.user}` : hosted() ? undefined : 'this machine'} label="settings">
      <div className="flex flex-wrap gap-6">
        <nav aria-label="settings" className="flex flex-[1_1_180px] flex-col gap-px md:max-w-[200px]">
          {SECTIONS.map((candidate) => (
            <Link
              key={candidate.id}
              to="/settings/$section"
              params={{ section: candidate.id }}
              aria-current={candidate.id === chosen ? 'page' : undefined}
              className={cx(
                'rounded-sm px-2.5 py-1.5 max-md:min-h-11',
                candidate.id === chosen ? 'bg-surface-2 font-medium text-fg-1' : 'text-fg-2 hover:bg-surface-2 hover:text-fg-1',
              )}
            >
              {candidate.label}
            </Link>
          ))}
        </nav>
        <div className="min-w-0 flex-[999_1_480px] md:max-w-[880px]">
          <InSettings.Provider value>{bodies[chosen]}</InSettings.Provider>
        </div>
      </div>
    </Page>
  )
}
