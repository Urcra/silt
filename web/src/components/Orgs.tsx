import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, type OrgChange, type OrgInfo, type Policy, type Preset } from '../api'
import { KNOWN_AGENTS, PRESET_LABELS, rulesLine } from '../rules'
import { keys, message } from '../query'
import { Button, EmptyState, Input, Select, Skeleton, Tag, cx, toast } from '../ui'
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

/**
 * What agents started in the org's projects may do, on every member's machine (ADR 0065). Members' own rules can
 * add to it, never loosen it.
 */
function OrgPolicyForm({ org }: { org: OrgInfo }) {
  const change = useOrgChange()
  const [draft, setDraft] = useState<Policy>()
  const policy = draft ?? org.policy
  const owns = org.may_change
  const set = (next: Partial<Policy>) => setDraft({ ...policy, ...next })
  const cap = policy.max_preset ?? 'full_access'
  return (
    <section aria-label={`${org.name}'s policy`} className="flex flex-col gap-2 rounded-md border border-line-subtle p-3">
      <div className="flex flex-wrap items-center gap-2">
        <h4 className="flex-1 font-mono text-[11px] tracking-wider text-fg-2 uppercase">Policy</h4>
        {rulesLine(org.policy) ? <span className="text-xs">{rulesLine(org.policy)}</span> : <span className="text-xs text-fg-2">No rules: any agent, up to Full access</span>}
      </div>
      <p className="text-xs text-fg-2">
        Applies to {org.name}'s projects on every member's machine. Members' own rules can add to it but never loosen it. Silt enforces it for every agent it
        starts; it can't stop an agent someone runs without Silt.
      </p>
      <fieldset disabled={!owns} className="grid grid-cols-1 gap-3 md:grid-cols-[200px_minmax(0,1fr)]">
        <span className="flex flex-col">
          <span>Agents</span>
          <span className="text-2xs text-fg-2">None checked allows any; a list allows no custom agent</span>
        </span>
        <span className="flex flex-wrap gap-x-3 gap-y-1">
          {KNOWN_AGENTS.map((agent) => (
            <label key={agent.key} className="inline-flex items-center gap-1 text-xs">
              <input
                type="checkbox"
                aria-label={`${org.name} allows ${agent.name}`}
                checked={policy.agents?.includes(agent.key) ?? false}
                onChange={(event) => {
                  const kept = (policy.agents ?? []).filter((key) => key !== agent.key)
                  const agents = event.target.checked ? [...kept, agent.key] : kept
                  set({ agents: agents.length > 0 ? agents : null })
                }}
              />
              {agent.name}
            </label>
          ))}
          {!policy.agents && (
          <label className="inline-flex items-center gap-1 text-xs">
            <input
              type="checkbox"
              aria-label={`${org.name} allows custom agents`}
              checked={policy.custom_agents !== false}
              onChange={(event) => set({ custom_agents: event.target.checked ? null : false })}
            />
            Custom agents from a machine's <code className="font-mono">agents.json</code>
          </label>
          )}
        </span>
        <span className="flex flex-col">
          <span>Most an agent may do without asking</span>
          <span className="text-2xs text-fg-2">Caps the preset, and the agent's own modes</span>
        </span>
        <span role="radiogroup" aria-label={`${org.name}'s most permissive preset`} className="inline-flex self-start overflow-hidden rounded-sm border border-line text-xs">
          {(['ask_for_everything', 'ask_before_commands', 'full_access'] as Preset[]).map((preset) => (
            <button
              key={preset}
              type="button"
              role="radio"
              aria-checked={cap === preset}
              onClick={() => set({ max_preset: preset === 'full_access' ? null : preset })}
              className={cx('px-3 py-1 max-md:min-h-11', cap === preset ? 'bg-accent-tint text-accent-fg' : 'text-fg-2 hover:bg-surface-2')}
            >
              {PRESET_LABELS[preset]}
            </button>
          ))}
        </span>
        <label htmlFor={`${org.name}-connectors`} className="flex flex-col">
          <span>Page connectors</span>
          <span className="text-2xs text-fg-2">The connectors pages may call, by name, comma-separated; empty allows any</span>
        </label>
        <input
          id={`${org.name}-connectors`}
          aria-label={`${org.name}'s page connectors`}
          defaultValue={(policy.connectors ?? []).join(', ')}
          onBlur={(event) => {
            const named = event.target.value.split(',').map((name) => name.trim()).filter(Boolean)
            set({ connectors: named.length > 0 ? named : null })
          }}
          placeholder="any"
          className="h-8 self-start rounded-sm border border-line bg-surface-1 px-2 text-xs text-fg-1"
        />
        {(
          [
            ['silt_sandbox', "Silt's sandbox", 'Agents run in it, and don’t start on a machine that can’t make one'],
            ['sandbox', "Agent's own sandbox", 'Agents that can’t sandbox are refused'],
            ['isolated', "Hide other workspaces' files", "The sandbox also keeps agents out of Silt's data and other workspaces' folders"],
            ['home_locked', 'Where threads sync', `Only to this server; never this machine only, never another`],
          ] as const
        ).map(([key, name, says]) => (
          <label key={key} className="contents">
            <span className="flex flex-col">
              <span>{name}</span>
              <span className="text-2xs text-fg-2">{says}</span>
            </span>
            <span className="inline-flex items-center gap-2 text-xs">
              <input type="checkbox" aria-label={`${org.name}: ${name}`} checked={Boolean(policy[key])} onChange={(event) => set({ [key]: event.target.checked })} />
              {policy[key] ? 'Required' : 'Off'}
            </span>
          </label>
        ))}
      </fieldset>
      {owns && draft && (
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDraft(undefined)}>
            Discard
          </Button>
          <Button variant="primary" onClick={() => void change({ change: 'policy', org: org.name, policy: draft }, 'Policy saved').then(() => setDraft(undefined))}>
            Save policy
          </Button>
        </div>
      )}
    </section>
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

      <OrgPolicyForm org={org} />

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
