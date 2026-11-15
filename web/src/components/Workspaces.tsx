import { useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { PRESETS, permissionModes, startSettings } from '../agentState'
import { type AgentChoice, type AgentDefaults, api, type MatchRule, type Policy, type Preset, type Workspace, type WorkspaceView, type Workspaces as WorkspacesFile } from '../api'
import { keys, message } from '../query'
import { KNOWN_AGENTS, PRESET_LABELS, rulesLine, serverLine } from '../rules'
import { serverName } from '../served'
import { SETTINGS_HEADING } from '../settingsContext'
import { Button, EmptyState, Input, Select, cx, toast } from '../ui'
import { settingLabel } from './AgentSettings'
import { WorkspaceBadge } from './WorkspaceSwitcher'

/** A workspace's id from its name, as `silt workspace new` makes it. */
function slug(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40)
}

function editable(view: { default: string; workspaces: WorkspaceView[] }): WorkspacesFile {
  return { default: view.default, workspaces: view.workspaces.map((shown) => shown.workspace) }
}

const SUB = 'text-xs text-fg-2'
const ROW = 'grid grid-cols-1 gap-2 border-b border-line-subtle px-4 py-3 last:border-b-0 md:grid-cols-[200px_minmax(0,1fr)]'

/** Settings › Workspaces (ADR 0065): each workspace's server, projects, defaults, agent accounts and rules. */
export function WorkspacesSettings() {
  const queries = useQueryClient()
  const view = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces })
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const agents = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const designSystems = useQuery({ queryKey: keys.designSystems, queryFn: api.designSystems })
  const [picked, setPicked] = useState<string>()
  const [draft, setDraft] = useState<WorkspacesFile>()
  const [error, setError] = useState<string>()
  const [newName, setNewName] = useState('')
  if (!view.data) return <section aria-label="Workspaces" className="text-sm text-fg-2">Loading workspaces…</section>
  const file = draft ?? editable(view.data)
  const chosenId = picked && file.workspaces.some((workspace) => workspace.id === picked) ? picked : file.default
  const workspace = file.workspaces.find((candidate) => candidate.id === chosenId) ?? file.workspaces[0]!
  const shown = view.data.workspaces.find((candidate) => candidate.workspace.id === workspace.id)
  const org = shown?.org_policy ?? null
  const servers = status.data?.servers ?? []
  const change = (next: Partial<Workspace>) =>
    setDraft({ ...file, workspaces: file.workspaces.map((candidate) => (candidate.id === workspace.id ? { ...candidate, ...next } : candidate)) })
  const changePolicy = (next: Partial<Policy>) => change({ policy: { ...workspace.policy, ...next } })
  const save = () => {
    if (!draft) return
    setError(undefined)
    api.setWorkspaces(draft).then(
      () => {
        setDraft(undefined)
        void queries.invalidateQueries({ queryKey: keys.workspaces })
        void queries.invalidateQueries({ queryKey: keys.threads })
        void queries.invalidateQueries({ queryKey: keys.projects })
        toast('Workspaces saved', 'ok')
      },
      (failure: unknown) => setError(message(failure)),
    )
  }
  const add = () => {
    const id = slug(newName)
    if (!id || file.workspaces.some((candidate) => candidate.id === id)) return setError(id ? `There's a workspace ${id} already.` : 'Give the workspace a name.')
    const made: Workspace = { id, name: newName.trim(), server: null, rules: [], defaults: {}, env: {}, policy: {} }
    setDraft({ ...file, workspaces: [...file.workspaces, made] })
    setPicked(id)
    setNewName('')
  }
  const locked = Boolean(org?.policy.home_locked)
  const effective = shown?.effective ?? workspace.policy
  return (
    <section aria-label="Workspaces" className="flex flex-col gap-5">
      <div className="flex flex-col gap-1">
        <h2 className={SETTINGS_HEADING}>Workspaces</h2>
        <p className="text-sm text-fg-2">
          A workspace is a set of projects with one server, defaults for new threads, and rules for the agents started in it. Agents in one never see another's
          threads.
        </p>
      </div>
      <div role="tablist" aria-label="workspaces" className="flex flex-wrap items-center gap-1.5">
        {file.workspaces.map((candidate) => (
          <button
            key={candidate.id}
            type="button"
            role="tab"
            aria-selected={candidate.id === workspace.id}
            onClick={() => setPicked(candidate.id)}
            className={cx(
              'inline-flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-sm max-md:min-h-11',
              candidate.id === workspace.id ? 'border-line-strong bg-accent-tint' : 'border-line hover:bg-surface-2',
            )}
          >
            <WorkspaceBadge name={candidate.name} size={20} />
            {candidate.name}
            {candidate.id === file.default && <span className="font-mono text-2xs text-fg-2">default</span>}
          </button>
        ))}
        <form
          className="flex items-center gap-1.5"
          onSubmit={(event) => {
            event.preventDefault()
            add()
          }}
        >
          <Input aria-label="new workspace's name" placeholder="New workspace" value={newName} onChange={(event) => setNewName(event.target.value)} className="w-40 text-xs" />
          <Button type="submit" icon="plus" disabled={!newName.trim()}>
            Add
          </Button>
        </form>
      </div>

      <div className="flex flex-wrap items-center gap-3">
        <WorkspaceBadge name={workspace.name} size={32} />
        <div className="flex min-w-0 flex-1 flex-col">
          <Input aria-label="workspace name" value={workspace.name} onChange={(event) => change({ name: event.target.value })} className="max-w-xs text-base font-medium" />
          <span className={cx(SUB, 'mt-1 font-mono')}>
            {shown ? `${shown.projects.length} ${shown.projects.length === 1 ? 'project' : 'projects'} · ${serverLine(shown)}` : 'new · not saved yet'}
          </span>
        </div>
        {workspace.id !== file.default && (
          <>
            <Button onClick={() => setDraft({ ...file, default: workspace.id })}>Make default</Button>
            <Button
              variant="ghost"
              onClick={() => {
                setDraft({ ...file, workspaces: file.workspaces.filter((candidate) => candidate.id !== workspace.id) })
                setPicked(file.default)
              }}
            >
              Remove
            </Button>
          </>
        )}
      </div>

      <Section title="Where its threads sync">
        <div className={ROW}>
          <span className="flex flex-col gap-0.5">
            <span>Server</span>
            <span className={SUB}>One server per workspace, or none</span>
          </span>
          <span className="flex flex-col gap-1.5">
            <span className="flex flex-wrap gap-1.5">
              <Select
                aria-label="server"
                disabled={locked}
                value={workspace.server?.url ?? ''}
                onChange={(event) => change({ server: event.target.value ? { url: event.target.value, org: null } : null })}
              >
                <option value="">This machine only</option>
                {servers.map((server) => (
                  <option key={server.url} value={server.url}>
                    {serverName(server.url)}
                  </option>
                ))}
              </Select>
              {workspace.server && (
                <Input
                  aria-label="org"
                  placeholder="org (optional)"
                  disabled={locked}
                  value={workspace.server.org ?? ''}
                  onChange={(event) => change({ server: { url: workspace.server!.url, org: event.target.value.trim() || null } })}
                  className="w-40 text-xs"
                />
              )}
            </span>
            <span className={SUB}>
              {locked
                ? `${org!.org} keeps its projects on ${serverName(org!.server)}: no other server, and not this machine only.`
                : workspace.server?.org
                  ? `${workspace.server.org}'s policy on that server applies here too.`
                  : 'Naming an org there applies its policy here too.'}
            </span>
          </span>
        </div>
      </Section>

      <Section title="Which projects belong here">
        {workspace.rules.length === 0 && (
          <p className="px-4 py-3 text-sm text-fg-2">
            {workspace.id === file.default
              ? 'No rules needed: as the default, it has every project no other workspace’s rules match, and those you put here yourself.'
              : 'No rules yet: only projects you put here yourself.'}
          </p>
        )}
        {workspace.rules.map((rule, index) => (
          <div key={`${rule.kind}${rule.pattern}`} className="flex items-center gap-2 border-b border-line-subtle px-4 py-2.5 last:border-b-0">
            <span className="rounded-sm border border-line px-1.5 font-mono text-2xs text-fg-2">{rule.kind}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-xs">{rule.pattern}</span>
            <span className={SUB}>{projectsBy(shown, rule)}</span>
            <Button variant="ghost" aria-label={`remove the rule ${rule.pattern}`} onClick={() => change({ rules: workspace.rules.filter((_, at) => at !== index) })}>
              Remove
            </Button>
          </div>
        ))}
        <AddRule onAdd={(rule) => change({ rules: [...workspace.rules, rule] })} />
      </Section>
      <p className={cx(SUB, '-mt-3')}>
        A remote's <code className="font-mono">*</code> is one part of its path, and a trailing <code className="font-mono">**</code> any number of them; a folder
        covers everything under it. The most specific rule wins: a remote beats a folder, a longer match beats a shorter one. A project you put in a workspace yourself ignores rules. Projects
        nothing matches go to <strong>{file.workspaces.find((candidate) => candidate.id === file.default)?.name}</strong>, the default.
      </p>

      <Section title="New threads start with">
        <div className={ROW}>
          <span>Agent</span>
          <Select aria-label="default agent" value={workspace.defaults.agent ?? ''} onChange={(event) => change({ defaults: { ...workspace.defaults, agent: event.target.value || null } })}>
            <option value="">The project's, else the first installed</option>
            {(agents.data?.agents ?? []).map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.name}
              </option>
            ))}
          </Select>
        </div>
        <div className={ROW}>
          <span>Permissions</span>
          <PresetSelect label="default permissions" value={workspace.defaults.preset ?? null} onChange={(preset) => change({ defaults: { ...workspace.defaults, preset } })} />
        </div>
        <div className={ROW}>
          <span>Chats' permissions</span>
          <PresetSelect label="default permissions for chats" value={workspace.defaults.chats_preset ?? null} onChange={(chats_preset) => change({ defaults: { ...workspace.defaults, chats_preset } })} />
        </div>
        <div className={ROW}>
          <span>Design system</span>
          <Select
            aria-label="default design system"
            value={workspace.defaults.design_system ?? ''}
            onChange={(event) => change({ defaults: { ...workspace.defaults, design_system: event.target.value || null } })}
          >
            <option value="">The project's, else none</option>
            {(designSystems.data ?? []).map((system) => (
              <option key={system.name} value={system.name}>
                {system.title}
                {system.source === 'builtin' ? ' (built in)' : ''}
              </option>
            ))}
          </Select>
        </div>
        <AgentDefaultsRows
          key={workspace.id}
          agents={agents.data?.agents ?? []}
          preferred={workspace.defaults.agent ?? undefined}
          value={workspace.defaults.agents ?? {}}
          onChange={(next) => change({ defaults: { ...workspace.defaults, agents: next } })}
        />
      </Section>

      <Section title="Agent accounts">
        {Object.entries(workspace.env).map(([name, value]) => (
          <div key={name} className="flex items-center gap-2 border-b border-line-subtle px-4 py-2.5 last:border-b-0">
            <span className="w-48 shrink-0 font-mono text-xs">{name}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-xs">{value}</span>
            <Button
              variant="ghost"
              aria-label={`remove ${name}`}
              onClick={() => change({ env: Object.fromEntries(Object.entries(workspace.env).filter(([kept]) => kept !== name)) })}
            >
              Remove
            </Button>
          </div>
        ))}
        <AddVariable onAdd={(name, value) => change({ env: { ...workspace.env, [name]: value } })} />
      </Section>
      <p className={cx(SUB, '-mt-3')}>
        Given to every agent started in {workspace.name}, from a terminal or here. Its own <code className="font-mono">CLAUDE_CONFIG_DIR</code> or{' '}
        <code className="font-mono">COPILOT_HOME</code> keeps its sign-ins and the agents' own transcripts apart.
      </p>

      <section aria-label="Rules" className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="flex-1 text-sm font-medium">Rules</h3>
          {org && <span className="rounded-sm bg-warn-tint px-2 py-0.5 text-2xs">{org.org}'s policy from {serverName(org.server)}</span>}
          {shown?.waiting_for_policy && <span className="rounded-sm bg-warn-tint px-2 py-0.5 text-2xs">waiting for the org's policy</span>}
        </div>
        <p className={SUB}>
          Rules refuse, they don't just preselect. Yours can only add to {org ? `${org.org}'s` : "an org's"}, never loosen them. Silt enforces them for every agent it
          starts; it can't stop an agent you run without Silt.
        </p>
        <div className="overflow-x-auto rounded-md border border-line bg-elevated">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-line text-left font-mono text-2xs tracking-[0.04em] text-fg-2 uppercase">
                <th scope="col" className="px-3 py-2 font-medium">Rule</th>
                <th scope="col" className="px-3 py-2 font-medium">Yours</th>
                <th scope="col" className="px-3 py-2 font-medium">{org ? `${org.org} on ${serverName(org.server)}` : 'Org'}</th>
                <th scope="col" className="px-3 py-2 font-medium">In effect</th>
              </tr>
            </thead>
            <tbody>
              <Rule
                name="Agents"
                mine={
                  <AgentsInput value={workspace.policy.agents ?? null} onChange={(agents) => changePolicy({ agents })} />
                }
                theirs={org ? agentsText(org.policy.agents) : '—'}
                effect={agentsText(effective.agents)}
              />
              <Rule
                name="Most it may do without asking"
                mine={<PresetSelect label="most it may do without asking" value={workspace.policy.max_preset ?? null} onChange={(max_preset) => changePolicy({ max_preset })} anyLabel="Any" />}
                theirs={org?.policy.max_preset ? PRESET_LABELS[org.policy.max_preset] : '—'}
                effect={effective.max_preset ? PRESET_LABELS[effective.max_preset] : 'Full access'}
              />
              <Rule
                name="Agent's own sandbox"
                mine={<Toggle label="require the agent's sandbox" on={Boolean(workspace.policy.sandbox)} onChange={(sandbox) => changePolicy({ sandbox })} />}
                theirs={org?.policy.sandbox ? 'Required' : '—'}
                effect={effective.sandbox || effective.isolated ? 'Required' : 'Not required'}
              />
              <Rule
                name="Isolated history"
                mine={<Toggle label="isolated history" on={Boolean(workspace.policy.isolated)} onChange={(isolated) => changePolicy({ isolated })} />}
                theirs={org?.policy.isolated ? 'On' : '—'}
                effect={effective.isolated ? 'On' : 'Off'}
              />
              <Rule
                name="Where threads sync"
                mine={<Toggle label="lock where threads sync" on={Boolean(workspace.policy.home_locked)} onChange={(home_locked) => changePolicy({ home_locked })} />}
                theirs={org?.policy.home_locked ? `Locked to ${serverName(org.server)}` : '—'}
                effect={effective.home_locked ? 'Locked' : 'Per project'}
              />
            </tbody>
          </table>
        </div>
        <p className={SUB}>
          <strong>Isolated</strong>: Silt's history tools answer only with {workspace.name}'s threads, and the agent's sandbox can't read Silt's data or other
          workspaces' folders. Without the sandbox, an agent could still read files directly.
        </p>
        {rulesLine(effective) && <p className="text-xs">In effect: {rulesLine(effective)}</p>}
      </section>

      {(draft || error) && (
        <div className="sticky bottom-0 flex flex-wrap items-center gap-2 border-t border-line bg-bg py-3">
          {error && <p className="flex-1 text-xs text-danger">{error}</p>}
          {draft && (
            <>
              <span className="flex-1 text-xs text-fg-2">Unsaved changes</span>
              <Button variant="ghost" onClick={() => setDraft(undefined)}>
                Discard
              </Button>
              <Button variant="primary" onClick={save}>
                Save workspaces
              </Button>
            </>
          )}
        </div>
      )}
      {!view.data.saved && !draft && (
        <EmptyState title="One workspace for everything">
          Add a workspace to give some projects their own server, defaults and rules, or run{' '}
          <code className="font-mono">silt workspace new Work --remote 'github.com/acme/*'</code>.
        </EmptyState>
      )}
    </section>
  )
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section aria-label={title} className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">{title}</h3>
      <div className="overflow-hidden rounded-md border border-line bg-elevated">{children}</div>
    </section>
  )
}

function Rule({ name, mine, theirs, effect }: { name: string; mine: ReactNode; theirs: string; effect: string }) {
  return (
    <tr className="border-b border-line-subtle align-middle last:border-b-0">
      <td className="px-3 py-2">{name}</td>
      <td className="px-3 py-2">{mine}</td>
      <td className="px-3 py-2 text-fg-2">{theirs}</td>
      <td className="bg-accent-tint px-3 py-2 font-medium">{effect}</td>
    </tr>
  )
}

function agentsText(agents: string[] | null | undefined): string {
  if (!agents) return 'Any'
  if (agents.length === 0) return 'None'
  return agents.map((key) => KNOWN_AGENTS.find((agent) => agent.key === key)?.name ?? key).join(', ')
}

function projectsBy(view: WorkspaceView | undefined, rule: MatchRule): string {
  const count = view?.projects.filter((project) => project.why.kind === 'rule' && project.why.rule.pattern === rule.pattern).length ?? 0
  return count === 0 ? 'no projects yet' : count === 1 ? '1 project' : `${count} projects`
}

/** An agent's model, effort and mode for new threads, chosen from what it listed when it last ran here. */
function AgentDefaultsRows({
  agents,
  preferred,
  value,
  onChange,
}: {
  agents: AgentChoice[]
  preferred?: string
  value: Record<string, AgentDefaults>
  onChange: (next: Record<string, AgentDefaults>) => void
}) {
  const [picked, setPicked] = useState<string>()
  const agent = agents.find((candidate) => candidate.id === (picked ?? preferred)) ?? agents[0]
  if (!agent) return null
  const modes = permissionModes(agent.modes?.available) ? agent.modes!.available : []
  const settings = startSettings(agent.settings, agent.modes?.available)
  const chosen = value[agent.id] ?? {}
  const set = (next: AgentDefaults) => {
    const rest = Object.fromEntries(Object.entries(value).filter(([id]) => id !== agent.id))
    onChange(next.mode || Object.keys(next.settings ?? {}).length > 0 ? { ...rest, [agent.id]: next } : rest)
  }
  const setSetting = (id: string, choice: string) => {
    const others = Object.fromEntries(Object.entries(chosen.settings ?? {}).filter(([kept]) => kept !== id))
    set({ ...chosen, settings: choice ? { ...others, [id]: choice } : others })
  }
  return (
    <>
      <div className={ROW}>
        <span>Model, effort and mode</span>
        <div className="flex flex-col gap-1">
          <Select aria-label="agent whose defaults to set" value={agent.id} onChange={(event) => setPicked(event.target.value)}>
            {agents.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.name}
                {value[candidate.id] ? ' · set' : ''}
              </option>
            ))}
          </Select>
          {settings.length === 0 && modes.length === 0 && <span className={SUB}>{agent.name} lists its models and modes the first time it runs here.</span>}
        </div>
      </div>
      {settings.map((setting) => (
        <div key={setting.id} className={ROW}>
          <span>{settingLabel(setting)}</span>
          <Select aria-label={`default ${settingLabel(setting).toLowerCase()} for ${agent.name}`} value={chosen.settings?.[setting.id] ?? ''} onChange={(event) => setSetting(setting.id, event.target.value)}>
            <option value="">As it last ran</option>
            {setting.choices.map((choice) => (
              <option key={choice.id} value={choice.id}>
                {choice.name}
              </option>
            ))}
          </Select>
        </div>
      ))}
      {modes.length > 0 && (
        <div className={ROW}>
          <span>Mode</span>
          <Select aria-label={`default mode for ${agent.name}`} value={chosen.mode ?? ''} onChange={(event) => set({ ...chosen, mode: event.target.value || null })}>
            <option value="">As it last ran</option>
            {modes.map((mode) => (
              <option key={mode.id} value={mode.id}>
                {mode.name}
              </option>
            ))}
          </Select>
        </div>
      )}
    </>
  )
}

function PresetSelect({
  label,
  value,
  onChange,
  anyLabel = 'No default',
}: {
  label: string
  value: Preset | null
  onChange: (preset: Preset | null) => void
  anyLabel?: string
}) {
  return (
    <Select aria-label={label} value={value ?? ''} onChange={(event) => onChange((event.target.value || null) as Preset | null)}>
      <option value="">{anyLabel}</option>
      {PRESETS.map((preset) => (
        <option key={preset.id} value={preset.id}>
          {preset.label}
        </option>
      ))}
    </Select>
  )
}

function Toggle({ label, on, onChange }: { label: string; on: boolean; onChange: (on: boolean) => void }) {
  return (
    <label className="inline-flex cursor-pointer items-center gap-2 text-xs">
      <input type="checkbox" aria-label={label} checked={on} onChange={(event) => onChange(event.target.checked)} className="accent-[var(--accent)]" />
      {on ? 'on' : 'off'}
    </label>
  )
}

function AgentsInput({ value, onChange }: { value: string[] | null; onChange: (agents: string[] | null) => void }) {
  return (
    <span className="flex flex-wrap gap-x-3 gap-y-1">
      <label className="inline-flex items-center gap-1 text-xs">
        <input type="checkbox" aria-label="any agent" checked={value === null} onChange={(event) => onChange(event.target.checked ? null : [])} />
        Any
      </label>
      {value !== null &&
        KNOWN_AGENTS.map((agent) => (
          <label key={agent.key} className="inline-flex items-center gap-1 text-xs">
            <input
              type="checkbox"
              aria-label={agent.name}
              checked={value.includes(agent.key)}
              onChange={(event) => onChange(event.target.checked ? [...value, agent.key] : value.filter((kept) => kept !== agent.key))}
            />
            {agent.name}
          </label>
        ))}
    </span>
  )
}

function AddRule({ onAdd }: { onAdd: (rule: MatchRule) => void }) {
  const [kind, setKind] = useState<MatchRule['kind']>('remote')
  const [pattern, setPattern] = useState('')
  return (
    <form
      className="flex flex-wrap items-center gap-1.5 px-4 py-2.5"
      onSubmit={(event) => {
        event.preventDefault()
        if (!pattern.trim()) return
        onAdd({ kind, pattern: pattern.trim().replace(/\/+$/, '') } as MatchRule)
        setPattern('')
      }}
    >
      <Select aria-label="kind of rule" value={kind} onChange={(event) => setKind(event.target.value as MatchRule['kind'])}>
        <option value="remote">Remote</option>
        <option value="folder">Folder</option>
      </Select>
      <Input
        aria-label="rule"
        placeholder={kind === 'remote' ? 'github.com/acme/*' : '/home/you/work'}
        value={pattern}
        onChange={(event) => setPattern(event.target.value)}
        className="min-w-0 flex-1 font-mono text-xs"
      />
      <Button type="submit" disabled={!pattern.trim()}>
        Add rule
      </Button>
    </form>
  )
}

function AddVariable({ onAdd }: { onAdd: (name: string, value: string) => void }) {
  const [name, setName] = useState('')
  const [value, setValue] = useState('')
  return (
    <form
      className="flex flex-wrap items-center gap-1.5 px-4 py-2.5"
      onSubmit={(event) => {
        event.preventDefault()
        if (!/^[A-Za-z0-9_]+$/.test(name) || !value) return
        onAdd(name, value)
        setName('')
        setValue('')
      }}
    >
      <Input aria-label="variable name" placeholder="CLAUDE_CONFIG_DIR" value={name} onChange={(event) => setName(event.target.value)} className="w-48 font-mono text-xs" />
      <Input aria-label="variable value" placeholder="/home/you/.config/silt/work/claude" value={value} onChange={(event) => setValue(event.target.value)} className="min-w-0 flex-1 font-mono text-xs" />
      <Button type="submit" disabled={!name || !value}>
        Add variable
      </Button>
    </form>
  )
}
