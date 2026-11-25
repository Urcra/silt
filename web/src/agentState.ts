import type { AgentChoice, AgentChoiceItem, AgentModes, AgentSetting, AgentState, AgentStatus, Preset } from './api'
import type { Tone } from './ui'

/** Which of the agent's permission requests the daemon answers by itself (ADR 0022), and Plan, which asks for a plan
 * before every new prompt (ADR 0044); the same for every agent, by how much they let through (ADR 0114). `short` is
 * what a chip shows. */
export const PRESETS: { id: Preset; label: string; short: string; says: string }[] = [
  { id: 'plan', label: 'Plan', short: 'Plan', says: 'reads and writes a plan for you to approve; everything else asks' },
  { id: 'ask_for_everything', label: 'Ask for everything', short: 'Ask', says: 'edits and commands ask' },
  { id: 'ask_before_commands', label: 'Ask before commands', short: 'Edits', says: 'edits apply, every command asks' },
  {
    id: 'auto',
    label: 'Auto',
    short: 'Auto',
    says: "the agent's own check lets safe actions through and asks about risky ones",
  },
  {
    id: 'full_access',
    label: 'Full access',
    short: 'Full',
    says: "nothing asks; the agent's own sandbox is all that limits it",
  },
]

/** The preset as `PRESETS` describes it. */
export function presetOf(id: Preset): (typeof PRESETS)[number] {
  return PRESETS.find((preset) => preset.id === id) ?? PRESETS[2]!
}

/** The preset after `preset` that may be picked, for Shift+Tab; none where none other may. */
export function nextPreset(preset: Preset, refused: (preset: Preset) => string | undefined): Preset | undefined {
  const at = PRESETS.findIndex((choice) => choice.id === preset)
  for (let step = 1; step < PRESETS.length; step++) {
    const next = PRESETS[(at + step) % PRESETS.length]!.id
    if (!refused(next)) return next
  }
  return undefined
}

/** Why the agent can't take `preset` itself, or nothing: Auto needs an auto mode of its own, so one without asks as
 * for Ask before commands. Nothing while its modes aren't known. */
export function presetUnavailable(
  preset: Preset,
  modes: AgentChoiceItem[] | null | undefined,
  agent: string,
): string | undefined {
  if (preset !== 'auto' || !modes || modes.some((mode) => (mode.id.split('#').pop() ?? mode.id) === 'auto')) return undefined
  return `${agent || 'This agent'} has no auto mode`
}

/** The presets a project can start its threads with: Plan is chosen per thread. */
export const DEFAULT_PRESETS = PRESETS.filter((preset) => preset.id !== 'plan')

/** A mode's id as Silt knows it: Copilot's are links ending in `#agent`, `#plan` and `#autopilot`. */
const modeKey = (id: string) => id.split('#').pop() ?? id

/** Modes that say what the agent may do without asking, by the ids Claude's, Codex's, Antigravity's and Silt's own
 * adapters use; the daemon maps them to presets. */
const PERMISSION_MODES = new Set([
  'default',
  'read-only',
  'acceptEdits',
  'accept-edits',
  'auto',
  'bypassPermissions',
  'dontAsk',
  'full-access',
  'chat',
])

/** Whether an agent's modes are its permissions, which then follow Silt's preset rather than show (ADR 0114). */
export function permissionModes(modes: AgentChoiceItem[] | null | undefined): boolean {
  return modes?.some((mode) => PERMISSION_MODES.has(modeKey(mode.id))) ?? false
}

/** The agent's plan mode, found as the daemon finds it. */
function isPlanMode(mode: AgentChoiceItem, modes: AgentModes): boolean {
  return mode.id === modes.plan || mode.id.toLowerCase().includes('plan') || mode.name.toLowerCase().includes('plan')
}

/**
 * The agent's own modes, which say how it works rather than what it may do, such as Copilot's Autopilot: the
 * permissions menu lists them under its name (ADR 0118). Not those a preset moves it into, not its plan mode, which
 * the Plan preset is, and not its default: the mode Ask before commands moves it to, else the first it lists.
 */
export function ownModes(modes: AgentModes | null | undefined): AgentChoiceItem[] {
  if (!modes) return []
  const fallback = modeForPreset('ask_before_commands', modes.available) ?? modes.available[0]?.id
  return modes.available.filter(
    (mode) => !PERMISSION_MODES.has(modeKey(mode.id)) && mode.id !== fallback && !isPlanMode(mode, modes),
  )
}

/** The preset the daemon answers with in one of the agent's own modes: Autopilot asks nothing; in a mode Silt can't
 * read, everything the agent asks goes to the person (ADR 0118). */
export function presetForOwnMode(mode: string): Preset {
  return modeKey(mode) === 'autopilot' ? 'full_access' : 'ask_for_everything'
}

/** What one of the agent's own modes does, as the permissions menu says it: the agent's words, and what Silt asks. */
export function ownModeSays(mode: AgentChoiceItem): string {
  if (modeKey(mode.id) === 'autopilot')
    return 'nothing asks, and it keeps going until the work is done, without coming back to you'
  return [mode.description, 'Silt asks you about every request'].filter(Boolean).join(' · ')
}

/** One step of the permissions menu: a preset, or one of the agent's own modes. */
export type Permission = { preset: Preset; mode?: undefined } | { mode: string; preset?: undefined }

/** The step after the one chosen that may be picked, for Shift+Tab: the presets, then the agent's own modes (ADR 0118). */
export function nextPermission(
  preset: Preset,
  mode: string | undefined,
  own: AgentChoiceItem[],
  refused: (preset: Preset) => string | undefined,
  refusedMode: (mode: string) => string | undefined,
): Permission | undefined {
  const steps: Permission[] = [
    ...PRESETS.map((choice): Permission => ({ preset: choice.id })),
    ...own.map((choice): Permission => ({ mode: choice.id })),
  ]
  const at = steps.findIndex((step) => (mode !== undefined ? step.mode === mode : step.preset === preset))
  for (let step = 1; step < steps.length; step++) {
    const next = steps[(at + step) % steps.length]!
    if (!(next.mode !== undefined ? refusedMode(next.mode) : refused(next.preset))) return next
  }
  return undefined
}

/** The agent's mode that does what `preset` says, as the daemon picks it, when it has one. */
export function modeForPreset(preset: Preset, modes: AgentChoiceItem[] | null | undefined): string | undefined {
  const wanted: Record<Preset, string[]> = {
    plan: ['plan'],
    ask_for_everything: ['default', 'read-only', 'agent'],
    ask_before_commands: ['acceptEdits', 'accept-edits', 'agent'],
    auto: ['auto', 'acceptEdits', 'accept-edits', 'agent'],
    full_access: ['bypassPermissions', 'full-access', 'dontAsk'],
  }
  return wanted[preset].map((id) => modes?.find((mode) => modeKey(mode.id) === id)?.id).find(Boolean)
}

/** An agent's own switch for asking nothing, such as Copilot's Allow All, which the daemon sets from the preset (ADR 0124). */
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
 * setting, which its modes or the preset say, nor what the preset sets (ADR 0124).
 */
export function startSettings(settings: AgentSetting[], modes: AgentChoiceItem[] | null | undefined): AgentSetting[] {
  return settings.filter((setting) => !(modes?.length && setting.category === 'mode') && !presetDriven(setting))
}

/**
 * The agent a thread whose agent stopped starts again: the one it ran, as the daemon still knows it or as the thread
 * names it, while it may run here; else the first that may.
 */
export function threadAgent(
  agents: AgentChoice[],
  stopped: AgentStatus | null,
  named: string | null | undefined,
  refused: (agent: string) => string | undefined,
): string {
  const may = (id: string | undefined) => (id && agents.some((choice) => choice.id === id) && !refused(id) ? id : undefined)
  const ran = stopped?.agent_id || agents.find((choice) => choice.name === named || choice.id === named)?.id
  return may(ran) ?? agents.find((choice) => !refused(choice.id))?.id ?? ''
}

/**
 * The modes and settings `agent` starts again with in a thread whose agent stopped. The thread's own, as its agent
 * left them, when it's the same agent, so the session goes on with the model it had; the agent's as it last ran
 * anywhere on this machine otherwise, under the workspace's defaults.
 */
export function restartChoices(
  agent: string,
  stopped: AgentStatus | null,
  seen: AgentChoice | undefined,
  preferred: Record<string, string> | undefined,
): { modes: AgentModes | null; settings: AgentSetting[]; value: (setting: AgentSetting, chosen: string | undefined) => string } {
  const own = stopped?.agent_id === agent ? stopped : undefined
  const modes = own ? own.modes : (seen?.modes ?? null)
  return {
    modes,
    settings: startSettings((own ?? seen)?.settings ?? [], modes?.available),
    value: (setting, chosen) => (own ? (chosen ?? setting.current) : startingValue(setting, chosen, preferred?.[setting.id])),
  }
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
