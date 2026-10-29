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
import { InSettings, SETTINGS_HEADING } from '../settingsContext'
import { Page } from './Page'
import { AddProjectPage, Projects } from './Projects'

const GROUPS: { label: string; sections: { id: string; label: string }[] }[] = [
  {
    label: 'Workspace',
    sections: [
      { id: 'projects', label: 'Projects' },
      { id: 'orgs', label: 'Orgs' },
    ],
  },
  {
    label: 'You',
    sections: [
      { id: 'devices', label: 'Devices and sync' },
      { id: 'notifications', label: 'Notifications' },
      { id: 'appearance', label: 'Appearance' },
    ],
  },
]

const SECTIONS = GROUPS.flatMap((group) => group.sections)

/** The servers this machine syncs with, and how to add one (ADR 0032). */
function Sync() {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const servers = status.data?.servers ?? []
  return (
    <section aria-label="Sync" className="flex flex-col gap-3">
      <h2 className={SETTINGS_HEADING}>Sync</h2>
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
      <h2 className={SETTINGS_HEADING}>Appearance</h2>
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
      <h2 className={SETTINGS_HEADING}>Notifications</h2>
      <NotificationBanner />
      <NotificationSettings open />
    </section>
  )
}

/** One page for what was Projects, Devices and Orgs, and the rest of the settings (ADR 0044). */
export function Settings() {
  const { section } = useParams({ strict: false }) as { section?: string }
  // Sync is part of Devices and sync; its old address still lands there.
  const chosen = section === 'add' ? 'projects' : section === 'sync' ? 'devices' : (SECTIONS.find((candidate) => candidate.id === section)?.id ?? 'projects')
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const signIn = status.data?.servers?.[0]
  const bodies: Record<string, ReactNode> = {
    projects: section === 'add' ? <AddProjectPage /> : <Projects />,
    devices: <Devices>{!hosted() && <Sync />}</Devices>,
    orgs: <Orgs />,
    notifications: <Notifications />,
    appearance: <Appearance />,
  }
  return (
    <Page title="Settings" meta={signIn ? `${signIn.user} on ${serverName(signIn.url)}` : hosted() ? undefined : 'this machine'} label="settings">
      <div className="flex flex-col gap-6 md:flex-row md:gap-10">
        <nav aria-label="settings" className="flex flex-col gap-px md:w-[184px] md:shrink-0">
          {GROUPS.map((group) => (
            <div key={group.label} className="flex flex-col gap-px">
              <p className="px-2.5 pt-2.5 pb-1.5 font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase">{group.label}</p>
              {group.sections.map((candidate) => (
                <Link
                  key={candidate.id}
                  to="/settings/$section"
                  params={{ section: candidate.id }}
                  aria-current={candidate.id === chosen ? 'page' : undefined}
                  className={cx(
                    'rounded-md px-2.5 py-1.5 max-md:min-h-11',
                    candidate.id === chosen ? 'bg-accent-tint font-medium text-fg-1 shadow-[inset_2px_0_0_var(--accent)]' : 'text-fg-2 hover:bg-surface-2 hover:text-fg-1',
                  )}
                >
                  {candidate.label}
                </Link>
              ))}
            </div>
          ))}
        </nav>
        <div className="min-w-0 flex-1 md:max-w-[760px]">
          <InSettings.Provider value>{bodies[chosen]}</InSettings.Provider>
        </div>
      </div>
    </Page>
  )
}
