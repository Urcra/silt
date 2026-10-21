import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import { Fragment, useState } from 'react'
import { api } from '../api'
import { useNow } from '../now'
import { keys } from '../query'
import { sections } from '../sections'
import { serverName } from '../served'
import { useActions } from '../shortcuts'
import { Dialog, Select } from '../ui'
import { type Workspace, inWorkspace, orgsOf, setWorkspace, useWorkspace, workspaceOf, workspaceValue } from '../workspace'

const CONNECT = 'connect'

/**
 * Picks the workspace this tab looks at (ADR 0032): an org on one of the
 * servers this machine is signed in to, a whole server, this machine's own
 * threads, or everything, each with how many threads need the person.
 */
export function WorkspaceSwitcher() {
  const workspace = useWorkspace()
  const queries = useQueryClient()
  const now = useNow()
  const [connecting, setConnecting] = useState(false)
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
  const named = (name: string, shown: Workspace) => {
    const count = waiting(shown)
    return count > 0 ? `${name} · ${count} need you` : name
  }
  const choose = (shown: Workspace) => {
    setWorkspace(shown)
    for (const key of [keys.orgs, keys.devices, keys.notificationSettings]) void queries.invalidateQueries({ queryKey: key })
  }
  type Choice = { label: string; long: string; workspace: Workspace }
  const groups: { server?: string; choices: Choice[] }[] = [
    { choices: [{ label: 'Everything', long: 'everything', workspace: { kind: 'all' } }] },
    ...servers.map((server, index) => {
      const host = serverName(server.url)
      return {
        server: host,
        choices: [
          { label: `All of ${host}`, long: `all of ${host}`, workspace: { kind: 'server', server: server.url, org: null } as Workspace },
          ...(orgs[index]?.data ?? []).map((org) => ({
            label: org.name,
            long: `${org.name} on ${host}`,
            workspace: { kind: 'server', server: server.url, org: org.name } as Workspace,
          })),
        ],
      }
    }),
    { choices: [{ label: 'This machine', long: 'this machine', workspace: { kind: 'local' } }] },
  ]
  useActions(
    groups
      .flatMap((group) => group.choices)
      .map((choice) => ({
        id: `workspace:${workspaceValue(choice.workspace)}`,
        label: `Show ${choice.long}`,
        run: () => choose(choice.workspace),
      })),
  )
  const option = (choice: Choice) => (
    <option key={workspaceValue(choice.workspace)} value={workspaceValue(choice.workspace)}>
      {named(choice.label, choice.workspace)}
    </option>
  )
  return (
    <>
      <Select
        aria-label="workspace"
        className="max-w-48 text-xs"
        value={workspaceValue(workspace)}
        onChange={(event) => {
          if (event.target.value === CONNECT) setConnecting(true)
          else choose(workspaceOf(event.target.value))
        }}
      >
        {groups.map((group, index) =>
          group.server ? (
            <optgroup key={group.server} label={group.server}>
              {group.choices.map(option)}
            </optgroup>
          ) : (
            <Fragment key={index}>{group.choices.map(option)}</Fragment>
          ),
        )}
        <option value={CONNECT}>Connect another server…</option>
      </Select>
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
