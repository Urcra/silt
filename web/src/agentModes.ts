import type { AgentSetting } from './api'
import { store, stored } from './stored'

// An agent's settings are only known once its session is up, so a new thread
// offers the ones it had the last time this browser saw it run.
const SETTINGS_KEY = 'silt-agent-settings:'

export function rememberSettings(agent: string, settings: AgentSetting[]) {
  store(SETTINGS_KEY + agent, settings)
}

export function rememberedSettings(agent: string): AgentSetting[] {
  return stored<AgentSetting[]>(SETTINGS_KEY + agent, [])
}
