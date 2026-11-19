import type { AgentSetting } from './api'

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
