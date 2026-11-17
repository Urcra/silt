import type { AgentChoiceItem, AgentSetting, AgentState, Preset } from './api'
import type { Tone } from './ui'

/** Which of the agent's permission requests the daemon answers by itself (ADR 0022), and Plan,
 * which asks for a plan before every new prompt (ADR 0044). */
export const PRESETS: { id: Preset; label: string; says: string }[] = [
  { id: 'plan', label: 'Plan', says: 'reads and writes a plan for you to approve; everything else asks' },
  { id: 'ask_for_everything', label: 'Ask for everything', says: 'edits and commands ask' },
  { id: 'ask_before_commands', label: 'Ask before commands', says: 'edits apply, every command asks' },
  { id: 'full_access', label: 'Full access', says: "nothing asks; the agent's own sandbox is all that limits it" },
]

/** The presets a project can start its threads with: Plan is chosen per thread. */
export const DEFAULT_PRESETS = PRESETS.filter((preset) => preset.id !== 'plan')

/** Modes that say what the agent may do without asking, by the ids Claude's and Codex's adapters use; the daemon maps them to presets. */
const PERMISSION_MODES = new Set(['default', 'read-only', 'acceptEdits', 'auto', 'bypassPermissions', 'dontAsk', 'full-access'])

/** Whether an agent's modes are its permissions, which then stand in for Silt's preset (ADR 0048). */
export function permissionModes(modes: AgentChoiceItem[] | null | undefined): boolean {
  return modes?.some((mode) => PERMISSION_MODES.has(mode.id)) ?? false
}

/** The agent's mode that does what `preset` says, as the daemon picks it, when it has one. */
export function modeForPreset(preset: Preset, modes: AgentChoiceItem[] | null | undefined): string | undefined {
  const wanted: Record<Preset, string[]> = {
    plan: ['plan'],
    ask_for_everything: ['default', 'read-only', 'agent'],
    ask_before_commands: ['acceptEdits', 'agent'],
    full_access: ['bypassPermissions', 'full-access', 'dontAsk'],
  }
  const key = (id: string) => id.split('#').pop() ?? id
  return wanted[preset].map((id) => modes?.find((mode) => key(mode.id) === id)?.id).find(Boolean)
}

/** An agent's own switch for asking nothing, such as Copilot's Allow All, which the daemon sets from the preset (ADR 0049). */
export function presetDriven(setting: AgentSetting): boolean {
  const ids = new Set(setting.choices.map((choice) => choice.id.toLowerCase()))
  return (
    setting.category === 'permissions' &&
    [
      ['on', 'off'],
      ['true', 'false'],
      ['enabled', 'disabled'],
    ].some(([on, off]) => ids.has(on!) && ids.has(off!))
  )
}

/**
 * The settings to offer before the agent runs, and start it with: not its mode where it also lists that as a
 * setting, which its modes or the preset say, nor what the preset sets (ADR 0049).
 */
export function startSettings(settings: AgentSetting[], modes: AgentChoiceItem[] | null | undefined): AgentSetting[] {
  return settings.filter((setting) => !(modes?.length && setting.category === 'mode') && !presetDriven(setting))
}

/** What a setting starts with: the person's choice, else the workspace's default while the agent still lists it, else as it last ran. */
export function startingValue(setting: AgentSetting, chosen: string | undefined, preferred: string | undefined): string {
  return (
    chosen ?? (preferred !== undefined && setting.choices.some((choice) => choice.id === preferred) ? preferred : setting.current)
  )
}

/** How a state reads after the agent's name (ADR 0022). */
export const STATE_PHRASE: Record<AgentState, string> = {
  starting: 'is starting',
  idle: 'is ready',
  working: 'is working',
  awaiting_permission: 'is waiting for your permission',
  awaiting_input: 'wrote a plan for you to approve',
  error: 'stopped with an error',
  stopped: 'stopped',
}

/** A short label for lists, and how much attention it asks for. */
export const STATE_TAG: Record<AgentState, { label: string; tone: Tone }> = {
  starting: { label: 'starting', tone: 'neutral' },
  idle: { label: 'ready', tone: 'neutral' },
  working: { label: 'working', tone: 'info' },
  awaiting_permission: { label: 'needs you', tone: 'accent' },
  awaiting_input: { label: 'needs you', tone: 'accent' },
  error: { label: 'error', tone: 'danger' },
  stopped: { label: 'stopped', tone: 'neutral' },
}

/** What approving a plan tells the agent. */
export const APPROVED = 'The plan is approved. Go ahead.'

export function running(state: AgentState): boolean {
  return state !== 'error' && state !== 'stopped'
}
