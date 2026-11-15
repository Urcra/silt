// A workspace's rules as the web UI shows them (ADR 0065). The daemon
// enforces them; this only greys out what it would refuse, and says why.

import type { Policy, Preset, WorkspaceView, WorkspacesView } from './api'

/** The agents rules name, with their CLI's and ACP adapter's ids, as `silt_workspace::KNOWN_AGENTS` lists them. */
export const KNOWN_AGENTS: { key: string; ids: string[]; name: string }[] = [
  { key: 'claude', ids: ['claude-code', 'claude-acp'], name: 'Claude Code' },
  { key: 'codex', ids: ['codex', 'codex-acp'], name: 'Codex' },
  { key: 'copilot', ids: ['copilot-cli', 'github-copilot-cli'], name: 'GitHub Copilot' },
  { key: 'gemini', ids: ['gemini-cli', 'gemini'], name: 'Gemini CLI' },
  { key: 'agy', ids: ['antigravity-cli', 'antigravity-acp'], name: 'Antigravity' },
  { key: 'opencode', ids: ['opencode', 'opencode-v2'], name: 'OpenCode' },
  { key: 'local', ids: ['local'], name: 'Silt on a local model' },
]

/** The agents that run on local models, as `silt_workspace::LOCAL_AGENTS` lists them. */
export const LOCAL_AGENTS = ['local', 'claude']

export function agentKey(id: string): string {
  return KNOWN_AGENTS.find((agent) => agent.ids.includes(id))?.key ?? id.toLowerCase()
}

const LATITUDE: Record<Preset, number> = { ask_for_everything: 0, plan: 1, ask_before_commands: 2, full_access: 3 }

export const PRESET_LABELS: Record<Preset, string> = {
  ask_for_everything: 'Ask for everything',
  plan: 'Plan',
  ask_before_commands: 'Ask before commands',
  full_access: 'Full access',
}

export function within(preset: Preset, cap: Preset | null | undefined): boolean {
  return !cap || LATITUDE[preset] <= LATITUDE[cap]
}

/** Whose rules a workspace's effective rule comes from, for the reason under what it refuses. */
function whose(view: WorkspaceView, rule: (policy: Policy) => boolean): string {
  const org = view.org_policy
  return org && rule(org.policy) ? `${org.org}'s policy` : 'your rules'
}

/** Why `agent` can't start in the workspace, or nothing when it can. */
export function agentRefused(view: WorkspaceView | undefined, agent: string): string | undefined {
  // A Local only workspace runs Silt's own agent, and Claude Code on a local model (ADR 0066).
  if (view?.effective.local_only && !LOCAL_AGENTS.includes(agentKey(agent))) return `Local only · ${view.workspace.name} runs local models only`
  const allowed = view?.effective.agents
  if (!view || !allowed || allowed.includes(agentKey(agent))) return undefined
  return `Not allowed in ${view.workspace.name} · ${whose(view, (policy) => Boolean(policy.agents && !policy.agents.includes(agentKey(agent))))}`
}

/** Why `preset` isn't allowed in the workspace, or nothing when it is. */
export function presetRefused(view: WorkspaceView | undefined, preset: Preset): string | undefined {
  const cap = view?.effective.max_preset
  if (!view || within(preset, cap)) return undefined
  const source = whose(view, (policy) => !within(preset, policy.max_preset))
  return `${source === 'your rules' ? 'Your rules allow' : `${source.replace("'s policy", '')} allows`} at most ${PRESET_LABELS[cap!]}`
}

/** The workspace a project is in, by its repository, else the default. */
export function workspaceOf(views: WorkspacesView | undefined, repo: string, workspace?: string | null): WorkspaceView | undefined {
  if (!views?.workspaces) return undefined
  const id = workspace ?? views.workspaces.find((view) => view.projects.some((project) => project.repo === repo))?.workspace.id ?? views.default
  return views.workspaces.find((view) => view.workspace.id === id)
}

/** The rules in effect, in a line: `Copilot only · at most Ask before commands · sandboxed`. */
export function rulesLine(policy: Policy): string {
  const parts: string[] = []
  if (policy.agents) {
    const names = policy.agents.map((key) => KNOWN_AGENTS.find((agent) => agent.key === key)?.name ?? key)
    parts.push(names.length === 0 ? 'no agent' : `${names.join(', ')} only`)
  }
  if (policy.max_preset && policy.max_preset !== 'full_access') parts.push(`at most ${PRESET_LABELS[policy.max_preset]}`)
  if (policy.custom_agents === false) parts.push('no custom agents')
  if (policy.isolated) parts.push('isolated history')
  else if (policy.sandbox) parts.push("agent's sandbox required")
  if (policy.home_locked) parts.push('home locked')
  if (policy.local_only) parts.push('local only')
  return parts.join(' · ')
}

export function serverLine(view: WorkspaceView): string {
  const server = view.workspace.server
  if (!server) return 'this machine only'
  const host = server.url.replace(/^\w+:\/\//, '').replace(/\/.*$/, '')
  return server.org ? `${server.org} on ${host}` : host
}

/** The preset a mode answers as, as the daemon maps it (ADR 0048): `undefined` for one it doesn't know. */
function modePreset(mode: string): Preset | undefined {
  const key = mode.split('#').pop() ?? mode
  if (key === 'plan') return 'plan'
  if (key === 'default' || key === 'read-only') return 'ask_for_everything'
  if (key === 'acceptEdits') return 'ask_before_commands'
  if (['bypassPermissions', 'auto', 'dontAsk', 'full-access', 'autopilot'].includes(key)) return 'full_access'
  return undefined
}

/** Why the agent's `mode` isn't allowed in the workspace, or nothing when it is. Copilot's modes say how it works, not what it asks (ADR 0049). */
export function modeRefused(view: WorkspaceView | undefined, agent: string, mode: string): string | undefined {
  const cap = view?.effective.max_preset
  if (!view || !cap || cap === 'full_access') return undefined
  const preset = modePreset(mode)
  if (preset) return presetRefused(view, preset)
  return agentKey(agent) === 'copilot' ? undefined : `Not known to Silt; ${presetRefused(view, 'full_access')?.toLowerCase() ?? 'refused'}`
}
