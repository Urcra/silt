import type { AgentChoiceItem, AgentSetting } from './api'

/** By its tag, or by its id where the agent tags none. */
export function isModel(setting: AgentSetting): boolean {
  return setting.category === 'model' || (setting.category === null && setting.id === 'model')
}

/** How hard the agent thinks, by its tag or a name agents give it. */
export function isEffort(setting: AgentSetting): boolean {
  return setting.category === 'thought_level' || /^(effort|reasoning|thinking)/i.test(setting.id)
}

/** Its name where it's listed on its own, the same for every agent's model and effort. */
export function settingLabel(setting: AgentSetting): string {
  return isModel(setting) ? 'Model' : isEffort(setting) ? 'Effort' : setting.name
}

/**
 * Where an effort choice sits among the agent's levels, for one bar per level. The agent's own default, which leaves
 * the level to it, isn't a level: it's `null` and isn't counted.
 */
export function effortLevel(choices: AgentChoiceItem[], id: string): { at: number | null; of: number } {
  const levels = choices.filter((choice) => !isDefaultEffort(choice))
  const at = levels.findIndex((choice) => choice.id === id)
  return { at: at < 0 ? null : at, of: levels.length }
}

function isDefaultEffort(choice: AgentChoiceItem): boolean {
  return /^(default|auto)$/i.test(choice.id) || /^(default|auto)$/i.test(choice.name)
}
