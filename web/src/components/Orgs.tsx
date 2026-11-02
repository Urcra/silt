import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, type OrgChange, type OrgInfo } from '../api'
import { keys, message } from '../query'
import { Button, EmptyState, Input, Select, Skeleton, Tag, toast } from '../ui'
import { SETTINGS_HEADING } from '../settingsContext'
import { Page } from './Page'

/** Sends an org change and refreshes what depends on it. */
function useOrgChange() {
  const queries = useQueryClient()
  return (change: OrgChange, said: string) =>
    api.orgChange(change).then(
      () => {
        toast(said, 'ok')
        void queries.invalidateQueries({ queryKey: keys.projects })
        return queries.invalidateQueries({ queryKey: keys.orgs })
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
}

/** A text field and a button that sends what's typed. */
function AddForm({ label, button, onAdd }: { label: string; button: string; onAdd: (value: string) => Promise<unknown> }) {
  const [value, setValue] = useState('')
  return (
    <form
      className="flex items-center gap-2"
      onSubmit={(event) => {
        event.preventDefault()
        void onAdd(value.trim()).then(() => setValue(''))
      }}
    >
      <Input aria-label={label} placeholder={label} className="w-44 text-xs" value={value} onChange={(event) => setValue(event.target.value)} />
      <Button type="submit" disabled={!value.trim()}>
        {button}
      </Button>
    </form>
  )
}

function Org({ org, others }: { org: OrgInfo; others: OrgInfo[] }) {
  const change = useOrgChange()
  const owns = org.may_change
  const [owner, setOwner] = useState(false)
  const teams = ['private', 'org', ...org.teams.map((team) => `team:${team.name}`)]
  const targets = others.filter((other) => other.name !== org.name && other.may_change)
  return (
    <section aria-label={org.name} className="space-y-3 rounded-md border border-line bg-elevated p-4 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="text-base font-medium">{org.name}</h3>
        <Tag tone={owns ? 'accent' : 'neutral'}>{org.role}</Tag>
        <label className="ml-auto flex items-center gap-2 text-xs text-fg-2">
          New threads are seen by
          <Select
            aria-label={`default visibility of ${org.name}`}
            className="text-xs"
            disabled={!owns}
            value={org.default_visibility}
            onChange={(event) => void change({ change: 'default_visibility', org: org.name, visibility: event.target.value }, 'Default changed')}
          >
            {teams.map((visibility) => (
              <option key={visibility} value={visibility}>
                {visibility === 'private' ? 'only their owner' : visibility === 'org' ? 'the whole org' : `team ${visibility.slice(5)}`}
              </option>
            ))}
          </Select>
        </label>
        <label className="flex items-center gap-1 text-xs text-fg-2" title="Turned off, every link made until then stops for good">
          <input
            type="checkbox"
            disabled={!owns}
            checked={org.links}
            onChange={(event) =>
              void change({ change: 'links', org: org.name, on: event.target.checked }, event.target.checked ? 'Links allowed' : 'Links stopped')
            }
          />
          Read-only links
        </label>
      </div>

      <div>
        <h4 className="mb-1 font-mono text-[11px] tracking-wider text-fg-3 uppercase">Members</h4>
        <ul aria-label={`members of ${org.name}`} className="divide-y divide-line-subtle">
          {org.members.map((member) => (
            <li key={member.user} className="flex items-center gap-2 py-1">
              <span>{member.user}</span>
              {member.role === 'owner' && <Tag>owner</Tag>}
              {owns && (
                <Button
                  variant="ghost"
                  className="ml-auto"
                  onClick={() => void change({ change: 'remove_member', org: org.name, user: member.user }, `Removed ${member.user}`)}
                >
                  Remove
                </Button>
              )}
            </li>
          ))}
        </ul>
        {owns && (
          <div className="mt-2 flex flex-wrap items-center gap-3">
            <AddForm
              label="user to add"
              button="Add member"
              onAdd={(user) => change({ change: 'add_member', org: org.name, user, owner }, `Added ${user}`)}
            />
            <label className="flex items-center gap-1 text-xs text-fg-2">
              <input type="checkbox" checked={owner} onChange={(event) => setOwner(event.target.checked)} />
              as an owner
            </label>
          </div>
        )}
      </div>

      <div>
        <h4 className="mb-1 font-mono text-[11px] tracking-wider text-fg-3 uppercase">Teams</h4>
        {org.teams.length === 0 && <p className="text-xs text-fg-3">No teams yet.</p>}
        <ul className="space-y-2">
          {org.teams.map((team) => (
            <li key={team.name} aria-label={`team ${team.name}`} className="space-y-1">
              <div className="flex items-center gap-2">
                <span className="font-medium">{team.name}</span>
                <span className="text-xs text-fg-3">{team.members.join(', ') || 'nobody yet'}</span>
                {owns && (
                  <Button
                    variant="ghost"
                    className="ml-auto"
                    onClick={() => void change({ change: 'remove_team', org: org.name, team: team.name }, `Removed team ${team.name}`)}
                  >
                    Remove team
                  </Button>
                )}
              </div>
              {owns && (
                <div className="flex flex-wrap gap-1">
                  {org.members.map((member) =>
                    team.members.includes(member.user) ? (
                      <Button
                        key={member.user}
                        className="px-1.5 py-0.5"
                        onClick={() =>
                          void change({ change: 'remove_from_team', org: org.name, team: team.name, user: member.user }, `${member.user} left ${team.name}`)
                        }
                      >
                        − {member.user}
                      </Button>
                    ) : (
                      <Button
                        key={member.user}
                        variant="ghost"
                        className="px-1.5 py-0.5"
                        onClick={() =>
                          void change({ change: 'add_to_team', org: org.name, team: team.name, user: member.user }, `${member.user} joined ${team.name}`)
                        }
                      >
                        + {member.user}
                      </Button>
                    ),
                  )}
                </div>
              )}
            </li>
          ))}
        </ul>
        {owns && (
          <div className="mt-2">
            <AddForm label="team name" button="Create team" onAdd={(team) => change({ change: 'create_team', org: org.name, team }, `Created team ${team}`)} />
          </div>
        )}
      </div>

      <div>
        <h4 className="mb-1 font-mono text-[11px] tracking-wider text-fg-3 uppercase">Projects</h4>
        {org.projects.length === 0 && <p className="text-xs text-fg-3">No projects yet.</p>}
        <ul className="divide-y divide-line-subtle">
          {org.projects.map((repo) => (
            <li key={repo} className="flex items-center gap-2 py-1">
              <span className="font-mono text-xs">{repo}</span>
              {owns && targets.length > 0 && (
                <Select
                  aria-label={`move ${repo}`}
                  className="ml-auto text-xs"
                  value=""
                  onChange={(event) => void change({ change: 'move_project', repo, org: event.target.value }, `Moved ${repo}`)}
                >
                  <option value="">Move to…</option>
                  {targets.map((target) => (
                    <option key={target.name} value={target.name}>
                      {target.name}
                    </option>
                  ))}
                </Select>
              )}
            </li>
          ))}
        </ul>
      </div>
    </section>
  )
}

/** The person's orgs: members, teams, defaults and projects, changed by their owners (ADR 0013). */
export function Orgs() {
  const orgs = useQuery({ queryKey: keys.orgs, queryFn: () => api.orgs() })
  const change = useOrgChange()
  return (
    <div className="flex flex-col gap-4">
      <h2 className={SETTINGS_HEADING}>Orgs</h2>
      {orgs.isPending && <Skeleton className="h-24 w-full" />}
      {orgs.error && <p className="text-sm text-danger">{message(orgs.error)}</p>}
      {orgs.isSuccess && orgs.data.length === 0 && <EmptyState title="You're in no org yet" />}
      {(orgs.data ?? []).map((org) => (
        <Org key={org.name} org={org} others={orgs.data ?? []} />
      ))}
      {orgs.isSuccess && (
        <section aria-label="create an org" className="text-sm">
          <p className="mb-1 text-xs text-fg-2">Server admins can create orgs.</p>
          <AddForm label="org name" button="Create org" onAdd={(org) => change({ change: 'create', org }, `Created ${org}`)} />
        </section>
      )}
    </div>
  )
}

/** Orgs in a plain page of the shell, until it's drawn as its board (ADR 0039). */
export function OrgsRoute() {
  return (
    <Page width="wide">
      <Orgs />
    </Page>
  )
}
