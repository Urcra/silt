import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useParams } from '@tanstack/react-router'
import { type ReactNode, useState } from 'react'
import { PRESETS } from '../agentState'
import { api, type ChatSettings, type PageConnector, type Preset } from '../api'
import { keys, message } from '../query'
import { hosted, serverName } from '../served'
import { Button, Dot, EmptyState, Input, cx, toast } from '../ui'
import { ChipSelect } from './ChipSelect'
import { Devices } from './Devices'
import { LocalModelsSettings } from './LocalModels'
import { NotificationBanner, NotificationSettings } from './Inbox'
import { Orgs } from './Orgs'
import { InSettings, SETTINGS_HEADING } from '../settingsContext'
import { Page } from './Page'
import { DesignSystemsSettings } from './DesignSystems'
import { ThemeSettings } from './Themes'
import { AddProjectPage, Projects } from './Projects'
import { UpdatesSettings } from './Updates'
import { WorkspacesSettings } from './Workspaces'

const GROUPS: { label: string; sections: { id: string; label: string }[] }[] = [
  {
    label: 'Workspace',
    sections: [
      ...(hosted() ? [] : [{ id: 'workspaces', label: 'Workspaces' }]),
      { id: 'projects', label: 'Projects' },
      { id: 'orgs', label: 'Orgs' },
      ...(hosted() ? [] : [{ id: 'design-systems', label: 'Design systems' }]),
    ],
  },
  {
    label: 'You',
    sections: [
      { id: 'devices', label: 'Devices and sync' },
      ...(hosted() ? [] : [{ id: 'chats', label: 'Chats' }]),
      ...(hosted() ? [] : [{ id: 'models', label: 'Local models' }]),
      { id: 'notifications', label: 'Notifications' },
      { id: 'appearance', label: 'Appearance' },
      ...(hosted() ? [] : [{ id: 'update', label: 'Updates' }]),
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

/** Where this machine keeps chats' folders, and what a new chat may do without asking (ADR 0059). */
function Chats() {
  const queries = useQueryClient()
  const settings = useQuery({ queryKey: keys.chatSettings, queryFn: api.chatSettings })
  const [dir, setDir] = useState<string>()
  const [error, setError] = useState<string>()
  const save = (next: ChatSettings) => {
    setError(undefined)
    api.setChatSettings(next).then(
      () => {
        void queries.invalidateQueries({ queryKey: keys.chatSettings })
        toast('Chat settings saved', 'ok')
      },
      (failure: unknown) => setError(message(failure)),
    )
  }
  const current = settings.data
  const shownDir = dir ?? current?.dir ?? ''
  return (
    <section aria-label="Chats" className="flex flex-col gap-3">
      <h2 className={SETTINGS_HEADING}>Chats</h2>
      <p className="text-sm text-fg-2">
        A chat is a thread outside any project, for questions and research. Each one works in a folder of its own, is private to you, and goes to your other
        devices like any thread.
      </p>
      {current && (
        <>
          <div className="flex flex-col gap-1.5 text-sm">
            <span className="font-medium">Default permissions</span>
            <span className="text-xs text-fg-2">A chat's folder holds only what its agent wrote there, so chats start with full access.</span>
            <span>
              <ChipSelect
                label="default permissions for chats"
                icon="lock"
                value={current.preset}
                onChange={(preset) => save({ ...current, preset: preset as Preset })}
                options={PRESETS.map((choice) => ({ id: choice.id, name: choice.label, description: choice.says }))}
              />
            </span>
          </div>
          <form
            className="flex flex-col gap-1.5 text-sm"
            onSubmit={(event) => {
              event.preventDefault()
              save({ ...current, dir: shownDir.trim() || null })
            }}
          >
            <span className="font-medium">Folder</span>
            <span className="text-xs text-fg-2">Where new chats' folders go. Left empty, they stay in Silt's data directory, out of sight.</span>
            <span className="flex gap-1.5">
              <Input aria-label="chats folder" placeholder="~/.local/share/silt/chats" value={shownDir} onChange={(event) => setDir(event.target.value)} className="min-w-0 flex-1 font-mono text-xs" />
              <Button type="submit">Save</Button>
            </span>
          </form>
        </>
      )}
      {error && <p className="text-xs text-danger">{error}</p>}
    </section>
  )
}

function Appearance() {
  return (
    <section aria-label="Appearance" className="flex flex-col gap-3">
      <ThemeSettings />
      {!hosted() && <ArtifactLoading />}
      {!hosted() && <PageConnectors />}
    </section>
  )
}

/** Whether artifacts' libraries and fonts load through Silt rather than from the CDNs (ADR 0071). */
function ArtifactLoading() {
  const queries = useQueryClient()
  const settings = useQuery({ queryKey: keys.artifactSettings, queryFn: api.artifactSettings })
  if (!settings.data) return null
  const proxy = settings.data.proxy
  const change = (next: boolean) =>
    api.setArtifactSettings({ proxy: next }).then(
      () => void queries.invalidateQueries({ queryKey: keys.artifactSettings }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <label className="flex items-start gap-2 pt-2 text-sm text-fg-1">
      <input type="checkbox" checked={proxy} onChange={(event) => void change(event.target.checked)} className="mt-1" />
      <span className="flex flex-col gap-0.5">
        Load artifacts' libraries and fonts through Silt
        <span className="text-xs text-fg-2">
          Silt fetches each file once and keeps it, so opening an artifact never contacts a CDN, and pages you've opened before work offline. Off, the page loads
          them from the CDNs.
        </span>
      </span>
    </label>
  )
}

/**
 * The MCP servers pages may call on this machine (ADR 0080): each one's tools are off until ticked, and pages reach
 * only those, with every action confirmed.
 */
function PageConnectors() {
  const queries = useQueryClient()
  const key = ['page-connectors']
  const list = useQuery({ queryKey: key, queryFn: api.pageConnectors })
  const [open, setOpen] = useState<string | null>(null)
  // A header's value being typed, for the keyring.
  const [secret, setSecret] = useState<{ name: string; header: string; value: string } | null>(null)
  const keep = (name: string, header: string, value: string | null) =>
    api.setConnectorSecret(name, header, value).then(
      () => {
        setSecret(null)
        void queries.invalidateQueries({ queryKey: key })
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const tools = useQuery({ queryKey: ['page-connectors', open], queryFn: () => api.connectorTools(open!), enabled: open !== null, retry: false })
  const save = (next: PageConnector[]) =>
    api.setPageConnectors(next).then(
      () => void queries.invalidateQueries({ queryKey: key }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const importing = () =>
    api.importPageConnectors().then(
      () => void queries.invalidateQueries({ queryKey: key }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const connectors = list.data ?? []
  return (
    <div className="flex flex-col gap-2 pt-4" data-testid="page-connectors">
      <h3 className="text-sm font-medium text-fg-1">Connectors for pages</h3>
      <p className="text-xs text-fg-2">
        MCP servers an artifact's page may call on this computer, as you. Each tool is off until you tick it, and a page asks you before every action.
      </p>
      {connectors.length === 0 && <p className="text-xs text-fg-3">None yet.</p>}
      <ul className="flex flex-col gap-2">
        {connectors.map((connector) => (
          <li key={connector.name} className="rounded-md border border-line p-2 text-sm">
            <div className="flex items-center gap-2">
              <span className="min-w-0 flex-1 truncate font-medium">{connector.name}</span>
              <code className="truncate text-xs text-fg-3">{connector.url ?? [connector.command, ...connector.args].join(' ')}</code>
              <Button size="sm" variant="ghost" onClick={() => setOpen(open === connector.name ? null : connector.name)}>
                Tools
              </Button>
              <Button size="sm" variant="ghost" onClick={() => void save(connectors.filter((other) => other.name !== connector.name))}>
                Remove
              </Button>
            </div>
            {connector.url && (
              <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
                {connector.header_secret.map((header) => (
                  <span key={header} className="inline-flex items-center gap-1 rounded-sm border border-line px-1.5 py-0.5">
                    <span className="font-mono">{header}</span>
                    <span className="text-fg-3">in the keyring</span>
                    <button type="button" className="text-fg-2 hover:text-fg-1" aria-label={`forget ${connector.name}'s ${header}`} onClick={() => void keep(connector.name, header, null)}>
                      ×
                    </button>
                  </span>
                ))}
                {secret?.name === connector.name ? (
                  <form
                    className="flex flex-wrap items-center gap-1"
                    onSubmit={(event) => {
                      event.preventDefault()
                      if (secret.header.trim() && secret.value) void keep(connector.name, secret.header.trim(), secret.value)
                    }}
                  >
                    <input
                      aria-label="the header"
                      value={secret.header}
                      onChange={(event) => setSecret({ ...secret, header: event.target.value })}
                      className="h-7 w-32 rounded-sm border border-line bg-surface-1 px-1.5 font-mono"
                    />
                    <input
                      aria-label="its value"
                      type="password"
                      autoComplete="off"
                      autoFocus
                      value={secret.value}
                      onChange={(event) => setSecret({ ...secret, value: event.target.value })}
                      placeholder="Bearer …"
                      className="h-7 w-48 rounded-sm border border-line bg-surface-1 px-1.5"
                    />
                    <Button size="sm" type="submit" disabled={!secret.value}>
                      Keep in the keyring
                    </Button>
                  </form>
                ) : (
                  <Button size="sm" variant="ghost" onClick={() => setSecret({ name: connector.name, header: 'Authorization', value: '' })}>
                    Add a secret header
                  </Button>
                )}
              </div>
            )}
            {open === connector.name && (
              <div className="mt-2 flex flex-col gap-1">
                {tools.isPending && <span className="text-xs text-fg-3">Starting {connector.name}…</span>}
                {tools.error && <span className="text-xs text-danger">{message(tools.error)}</span>}
                {tools.data?.map((tool) => (
                  <label key={tool.name} className="flex items-start gap-2 text-xs">
                    <input
                      type="checkbox"
                      checked={connector.tools[tool.name] === true}
                      onChange={(event) =>
                        void save(connectors.map((other) => (other.name === connector.name ? { ...other, tools: { ...other.tools, [tool.name]: event.target.checked } } : other)))
                      }
                    />
                    <span>
                      <span className="font-mono text-fg-1">{tool.name}</span> <span className="text-fg-3">{tool.read_only ? 'reads' : 'acts'}</span>
                      {tool.description && <span className="block text-fg-2">{tool.description}</span>}
                    </span>
                  </label>
                ))}
              </div>
            )}
          </li>
        ))}
      </ul>
      <div>
        <Button size="sm" onClick={() => void importing()}>
          Import from Claude Code
        </Button>
      </div>
    </div>
  )
}

function Notifications() {
  return (
    <section aria-label="Notifications" className="flex flex-col gap-3">
      <h2 className={SETTINGS_HEADING}>Notifications</h2>
      <NotificationBanner />
      <NotificationSettings />
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
    chats: <Chats />,
    models: <LocalModelsSettings />,
    workspaces: <WorkspacesSettings />,
    appearance: <Appearance />,
    update: <UpdatesSettings />,
    'design-systems': <DesignSystemsSettings />,
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
