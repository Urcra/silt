import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, type OrgInfo } from '../api'
import { KNOWN_AGENTS, serverLine } from '../rules'
import { useNow } from '../now'
import { keys } from '../query'
import { sections } from '../sections'
import { hosted, serverName } from '../served'
import { formatKeys, useActions } from '../shortcuts'
import { Button, Dialog, Icon, Kbd, Popover, cx } from '../ui'
import { type Workspace, inWorkspace, orgsOf, setWorkspace, useWorkspace, workspaceValue } from '../workspace'

interface Choice {
  name: string
  /** What it holds, under its name. */
  sub: string
  /** How the palette and a screen reader name it. */
  long: string
  workspace: Workspace
  offline?: boolean
  /** Its rules, as short chips under its name (ADR 0064). */
  chips?: string[]
}

interface Group {
  label: string
  choices: Choice[]
}

/** A workspace's letter on a tile of its own colour, as the boards draw it. */
export function WorkspaceBadge({ name, local = false, size = 28 }: { name: string; local?: boolean; size?: number }) {
  const shades = ['bg-[var(--tobacco-500)]', 'bg-[var(--slate-600)]', 'bg-[var(--olive-600)]', 'bg-[var(--amber-700)]', 'bg-[var(--rust-600)]']
  // Not by port, which a server started for a test or a tunnel picks anew.
  const shade = shades[[...name.replace(/:\d+$/, '')].reduce((sum, letter) => sum + letter.charCodeAt(0), 0) % shades.length]
  return (
    <span
      aria-hidden="true"
      style={{ width: size, height: size }}
      className={cx(
        'inline-flex shrink-0 items-center justify-center rounded-md text-xs font-semibold',
        local ? 'bg-surface-3 text-fg-1' : cx(shade, 'text-[var(--paper-50)]'),
      )}
    >
      {local ? <Icon name="laptop" /> : name.slice(0, 1).toUpperCase()}
    </span>
  )
}

function counted(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`
}

/** Every workspace this machine can show (ADR 0032), grouped by server. */
function useWorkspaces(): { groups: Group[]; waiting: (shown: Workspace) => number; current: Choice } {
  const now = useNow()
  const workspace = useWorkspace()
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const servers = status.data?.servers ?? []
  const orgs = useQueries({
    queries: servers.map((server) => ({
      queryKey: [...keys.orgs, 'of', server.url],
      queryFn: () => api.orgs(server.url),
      retry: false,
    })),
  })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const projects = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const owned = orgsOf(projects.data ?? [])
  const waiting = (shown: Workspace) =>
    sections((threads.data ?? []).filter((thread) => inWorkspace(thread, shown, owned)), now).needs_you.length
  const projectCount = (server: string | null, org: string | null) =>
    (projects.data ?? []).filter((project) => (server === null ? !project.server : project.server === server && (org === null || project.org === org)))
      .length
  const orgChoice = (url: string, host: string, org: OrgInfo, offline: boolean): Choice => ({
    name: org.name,
    sub: `${counted(projectCount(url, org.name), 'project', 'projects')} · ${org.role}`,
    long: `${org.name} on ${host}`,
    workspace: { kind: 'server', server: url, org: org.name },
    offline,
  })
  const groups: Group[] = [
    {
      label: 'Everywhere',
      choices: [{ name: 'Everything', sub: 'every server and this machine', long: 'everything', workspace: { kind: 'all' } }],
    },
    ...servers.map((server, index) => {
      const host = serverName(server.url)
      const offline = server.sync_error !== null
      return {
        label: `On ${host}`,
        choices: [
          {
            name: `All of ${host}`,
            sub: `${counted(projectCount(server.url, null), 'project', 'projects')} · signed in as ${server.user}`,
            long: `all of ${host}`,
            workspace: { kind: 'server', server: server.url, org: null } as Workspace,
            offline,
          },
          ...(orgs[index]?.data ?? []).map((org) => orgChoice(server.url, host, org, offline)),
        ],
      }
    }),
    {
      label: 'Not synced',
      choices: [
        {
          name: 'This machine',
          sub: `${counted(projectCount(null, null), 'project', 'projects')} · history stays here`,
          long: 'this machine',
          workspace: { kind: 'local' },
        },
      ],
    },
  ]
  const workspaces = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces, enabled: !hosted() })
  if (!hosted() && workspaces.data?.saved) {
    // Once the person made workspaces, they stand in for the servers and orgs (ADR 0064).
    const own: Group = {
      label: 'Workspaces',
      choices: workspaces.data.workspaces.map((view) => {
        const policy = view.effective
        const chips = [
          ...(view.org_policy ? [`${view.org_policy.org}'s rules`] : []),
          ...(policy.agents ? [policy.agents.map((key) => KNOWN_AGENTS.find((agent) => agent.key === key)?.name ?? key).join(', ') + ' only'] : []),
          ...(policy.max_preset && policy.max_preset !== 'full_access' ? ['asks first'] : []),
          ...(policy.isolated ? ['isolated'] : policy.sandbox ? ['sandboxed'] : []),
        ]
        const isDefault = view.workspace.id === workspaces.data.default
        return {
          name: view.workspace.name,
          sub: `${serverLine(view)} · ${counted(view.projects.length, 'project', 'projects')}${isDefault ? ' · default' : ''}`,
          long: `the ${view.workspace.name} workspace`,
          workspace: { kind: 'workspace', id: view.workspace.id, server: view.workspace.server?.url ?? null } as Workspace,
          offline: view.workspace.server ? servers.some((server) => server.url === view.workspace.server?.url && server.sync_error !== null) : false,
          chips,
        }
      }),
    }
    groups.splice(0, groups.length, { label: 'Everywhere', choices: [{ ...groups[0]!.choices[0]!, name: 'All workspaces', sub: 'for you only: agents never see across workspaces' }] }, own)
  }
  const all = groups.flatMap((group) => group.choices)
  const current = all.find((choice) => workspaceValue(choice.workspace) === workspaceValue(workspace)) ?? all[0]
  return { groups, waiting, current }
}

/**
 * The sidebar's header: the workspace this tab looks at (ADR 0032), opening
 * the others, each with how many threads need the person.
 */
export function WorkspaceSwitcher() {
  const queries = useQueryClient()
  const navigate = useNavigate()
  const [connecting, setConnecting] = useState(false)
  const { groups, waiting, current } = useWorkspaces()
  const choose = (shown: Workspace) => {
    setWorkspace(shown)
    for (const key of [keys.orgs, keys.devices, keys.notificationSettings]) void queries.invalidateQueries({ queryKey: key })
  }
  const choices = groups.flatMap((group) => group.choices)
  useActions(
    choices.map((choice, index) => ({
      id: `workspace:${workspaceValue(choice.workspace)}`,
      label: `Show ${choice.long}`,
      keys: index < 4 ? `alt+${index + 1}` : undefined,
      run: () => choose(choice.workspace),
    })),
  )
  const host = current.workspace.kind === 'server' ? serverName(current.workspace.server) : current.sub
  const workspaceServer = current.workspace.kind === 'workspace' && current.workspace.server ? serverName(current.workspace.server) : undefined
  const orgServer = current.workspace.kind === 'server' ? host : (workspaceServer ?? groups[1]?.label.match(/^On (.*)/)?.[1])
  return (
    <>
      <Popover
        label="Switch workspace"
        className="relative block min-w-0"
        triggerLabel={`workspace: ${current.long}`}
        triggerClassName="flex h-6 max-w-[170px] items-center gap-1 rounded-sm border border-line px-2 text-left text-xs text-fg-2 hover:bg-surface-2 hover:text-fg-1 max-md:min-h-11"
        panelClassName="w-[340px] max-w-[calc(100vw-20px)] p-1.5"
        trigger={
          <>
            <span className="truncate" title={host}>
              {current.name}
            </span>
            <Icon name="chevronDown" className="h-3 w-3 shrink-0" />
          </>
        }
      >
        {(close) => (
          <div className="flex flex-col gap-0.5">
            {groups.map((group, groupIndex) => (
              <section key={group.label} aria-label={group.label} className={cx(groupIndex > 0 && 'mt-1 border-t border-line-subtle pt-1')}>
                <h3 className="px-2.5 pt-1.5 pb-1 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">{group.label}</h3>
                {group.choices.map((choice) => {
                  const chosen = choice === current
                  const count = waiting(choice.workspace)
                  return (
                    <button
                      key={workspaceValue(choice.workspace)}
                      type="button"
                      aria-current={chosen || undefined}
                      onClick={() => {
                        choose(choice.workspace)
                        close()
                      }}
                      className={cx(
                        'grid w-full grid-cols-[28px_minmax(0,1fr)_auto] items-center gap-2.5 rounded-sm px-2.5 py-2 text-left text-fg-1 hover:bg-surface-2',
                        chosen && 'bg-accent-tint',
                      )}
                    >
                      <WorkspaceBadge name={choice.name} local={choice.workspace.kind === 'local'} />
                      <span className="flex min-w-0 flex-col">
                        <b className="truncate font-medium">{choice.name}</b>
                        <span className="truncate font-mono text-2xs text-fg-2">{choice.sub}</span>
                        {choice.chips && choice.chips.length > 0 && (
                          <span className="mt-1 flex flex-wrap gap-1">
                            {choice.chips.map((chip) => (
                              <span key={chip} className="rounded-sm border border-line px-1.5 text-2xs text-fg-1">
                                {chip}
                              </span>
                            ))}
                          </span>
                        )}
                      </span>
                      <span className="flex items-center gap-1.5">
                        {chosen && <Icon name="check" className="text-accent-fg" />}
                        {choice.offline && <span className="font-mono text-2xs text-fg-2">offline</span>}
                        {count > 0 && (
                          <span aria-label={`${count} need you`} className="rounded-sm bg-warn-tint px-1.5 text-2xs text-fg-1">
                            {count}
                          </span>
                        )}
                      </span>
                    </button>
                  )
                })}
              </section>
            ))}
            <div className="mt-1 flex flex-col border-t border-line-subtle pt-1">
              {!hosted() && (
                <Button
                  variant="ghost"
                  icon="settings"
                  className="justify-start px-2.5 py-1.5"
                  onClick={() => {
                    close()
                    void navigate({ to: '/settings/$section', params: { section: 'workspaces' } })
                  }}
                >
                  New or manage workspaces
                </Button>
              )}
              <Button
                variant="ghost"
                icon="plus"
                className="justify-start px-2.5 py-1.5"
                onClick={() => {
                  close()
                  void navigate({ to: '/orgs' })
                }}
              >
                {orgServer ? `Create an org on ${orgServer}` : 'Create an org'}
              </Button>
              <Button
                variant="ghost"
                icon="link"
                className="justify-start px-2.5 py-1.5"
                onClick={() => {
                  close()
                  setConnecting(true)
                }}
              >
                Connect another server
              </Button>
              <p className="flex gap-3 px-2.5 pt-1.5 pb-1 text-2xs text-fg-2">
                <span>
                  <Kbd>{formatKeys('alt+1')}</Kbd>–<Kbd>{formatKeys('alt+4')}</Kbd> switch
                </span>
                <span>
                  <Kbd>Esc</Kbd> close
                </span>
              </p>
            </div>
          </div>
        )}
      </Popover>
      <Dialog open={connecting} onClose={() => setConnecting(false)} title="Connect another server">
        <p className="text-sm">Run this in a terminal on this machine, with the server's address:</p>
        <pre className="mt-2 rounded-sm bg-surface-2 px-2 py-1 font-mono text-xs">silt auth login --server https://silt.example.com</pre>
        <p className="mt-2 text-sm text-fg-2">
          Each thread then syncs only with its own server; the first one you signed in to takes new projects' threads.
        </p>
      </Dialog>
    </>
  )
}
