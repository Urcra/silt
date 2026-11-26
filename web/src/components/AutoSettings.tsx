import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, type AgentChoice, type Judge, type WorkspaceView, type WorkspacesView } from '../api'
import { checkedBySilt } from '../agentState'
import { isModel } from '../agentSettings'
import { judgeName } from '../judges'
import { keys, message } from '../query'
import { agentRefused } from '../rules'
import { SETTINGS_HEADING } from '../settingsContext'
import { Icon, toast } from '../ui'
import { AgentMark } from './AgentMark'
import { ChipMenu, type ChipChoice } from './ChipMenu'
import { WorkspaceBadge } from './WorkspaceSwitcher'

const ROW = 'grid grid-cols-1 gap-2 border-b border-line-subtle px-4 py-3 last:border-b-0 md:grid-cols-[200px_minmax(0,1fr)]'
const THREAD = 'thread'

/** A judge as the chooser's option ids name it. */
function idOf(judge: Judge | null | undefined): string {
  if (!judge || judge.kind === 'thread') return THREAD
  return `${judge.agent}/${judge.model ?? ''}`
}

function judgeOf(id: string): Judge {
  if (id === THREAD) return { kind: 'thread' }
  const at = id.indexOf('/')
  const model = id.slice(at + 1)
  return { kind: 'agent', agent: id.slice(0, at), model: model || null }
}

/**
 * Settings › Auto (ADR 0132): for each workspace, who checks the commands of an agent without an auto mode of its own,
 * from the agents that workspace allows, and what Auto does for each of them.
 */
export function AutoSettings() {
  const queries = useQueryClient()
  const view = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces })
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const agents = choices.data?.agents ?? []
  const choose = (all: WorkspacesView, id: string, judge: Judge) => {
    const workspaces = all.workspaces.map(({ workspace }) =>
      workspace.id === id
        ? { ...workspace, defaults: { ...workspace.defaults, auto_judge: judge.kind === 'thread' ? null : judge } }
        : workspace,
    )
    api.setWorkspaces({ default: all.default, workspaces }).then(
      () => {
        void queries.invalidateQueries({ queryKey: keys.workspaces })
        toast('Saved who checks for Auto', 'ok')
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  }
  return (
    <section aria-label="Auto" className="flex flex-col gap-5">
      <div className="flex flex-col gap-1">
        <h2 className={SETTINGS_HEADING}>Auto</h2>
        <p className="text-sm text-fg-2">
          Auto lets what&apos;s safe through and asks about the rest. An agent with an auto mode of its own uses that. For an
          agent without one, Silt checks each command first, with an agent the workspace allows: what it refuses goes back to the
          agent with why, and the agent can ask you to allow it. A check that fails, or takes more than 60 s, leaves the command
          to you.
        </p>
      </div>
      {!view.data ? (
        <p className="text-sm text-fg-2">Loading workspaces…</p>
      ) : (
        view.data.workspaces.map((shown) => (
          <WorkspaceAuto
            key={shown.workspace.id}
            shown={shown}
            agents={agents}
            onChoose={(judge) => view.data && choose(view.data, shown.workspace.id, judge)}
          />
        ))
      )}
      <p className="text-xs text-fg-2">
        Each check is a session of its own, on that agent&apos;s plan. A thread can choose another of its workspace&apos;s agents
        under Auto in its permissions menu.
      </p>
    </section>
  )
}

function WorkspaceAuto({
  shown,
  agents,
  onChoose,
}: {
  shown: WorkspaceView
  agents: AgentChoice[]
  onChoose: (judge: Judge) => void
}) {
  const { workspace, org_policy: org, effective } = shown
  const fixed = org?.policy.auto_judge
    ? { judge: org.policy.auto_judge, by: org.org }
    : workspace.policy.auto_judge
      ? { judge: workspace.policy.auto_judge, by: 'your rules' }
      : null
  const judge = fixed?.judge ?? workspace.defaults.auto_judge ?? null
  const allowed = agents.filter((agent) => !agentRefused(shown, agent.id))
  const options: ChipChoice[] = [
    { id: THREAD, name: 'Same as the thread’s agent and model', description: 'Each thread’s own agent checks for it' },
    ...agents.flatMap((agent) => {
      const group = {
        key: agent.id,
        heading: (
          <>
            <AgentMark agent={agent.id} name={agent.name} className="h-3 w-3" />
            {agent.name}
          </>
        ),
      }
      const refused = agentRefused(shown, agent.id)
      const models = agent.settings.find(isModel)?.choices ?? []
      if (refused || models.length === 0) return [{ id: `${agent.id}/`, name: agent.name, refused, group }]
      return models.map((model) => ({ id: `${agent.id}/${model.id}`, name: model.name, group }))
    }),
  ]
  const named = effective.agents ? allowed.map((agent) => agent.name) : null
  return (
    <section aria-label={`Auto in ${workspace.name}`} className="flex flex-col gap-2">
      <h3 className="flex items-center gap-2 text-sm font-medium">
        <WorkspaceBadge name={workspace.name} size={20} />
        {workspace.name}
        <span className="font-mono text-2xs text-fg-2">{org ? `org ${org.org}` : workspace.id}</span>
      </h3>
      <div className="overflow-hidden rounded-md border border-line bg-elevated text-sm">
        <div className={ROW}>
          <span>Who checks</span>
          <div className="flex min-w-0 flex-col gap-1.5">
            {fixed ? (
              <span className="inline-flex h-7 w-fit items-center gap-1.5 rounded-sm border border-line bg-elevated px-2 text-xs text-fg-2 max-md:min-h-11">
                <Icon name="lock" className="h-3 w-3 shrink-0" />
                {judgeName(fixed.judge, agents)}
              </span>
            ) : (
              <span>
                <ChipMenu
                  label={`who checks for Auto in ${workspace.name}`}
                  value={idOf(judge)}
                  ghost={false}
                  heading={`Who checks in ${workspace.name}`}
                  trigger={judgeName(judge, agents)}
                  options={options}
                  onChange={(id) => onChoose(judgeOf(id))}
                />
              </span>
            )}
            <span className="text-xs text-fg-3">
              {fixed
                ? `Set by ${fixed.by} for ${workspace.name}.`
                : named
                  ? `Only agents ${workspace.name} allows: ${named.join(', ') || 'none here'}.`
                  : `Any agent ${workspace.name} allows.`}
            </span>
          </div>
        </div>
        <div className={ROW}>
          <span>Its agents</span>
          {allowed.length === 0 ? (
            <span className="text-xs text-fg-2">No agent here that {workspace.name} allows.</span>
          ) : (
            <ul className="flex flex-col gap-1">
              {allowed.map((agent) => {
                const silt = checkedBySilt(agent.modes?.available)
                return (
                  <li key={agent.id} className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
                    <AgentMark agent={agent.id} name={agent.name} className="h-3.5 w-3.5 text-fg-1" />
                    <span className="text-fg-1">{agent.name}</span>
                    <span className="text-fg-3">·</span>
                    <span className={silt ? 'text-xs text-accent-fg' : 'text-xs text-fg-2'}>
                      {!silt
                        ? 'its own auto mode'
                        : judge && judge.kind === 'agent'
                          ? `checked by ${judgeName(judge, agents)}`
                          : 'checked by Silt'}
                    </span>
                  </li>
                )
              })}
            </ul>
          )}
        </div>
      </div>
    </section>
  )
}
