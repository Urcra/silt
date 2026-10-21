import type { AgentChoiceItem } from './api'

// An agent's modes are only known once its session is up, so a new thread
// offers the ones it had the last time this browser saw it run.
const KEY = 'silt-agent-modes:'

export function rememberModes(agent: string, modes: AgentChoiceItem[]) {
  try {
    localStorage.setItem(KEY + agent, JSON.stringify(modes))
  } catch {
    // Without storage a new thread just doesn't offer them.
  }
}

export function rememberedModes(agent: string): AgentChoiceItem[] {
  try {
    const kept = localStorage.getItem(KEY + agent)
    return kept ? (JSON.parse(kept) as AgentChoiceItem[]) : []
  } catch {
    return []
  }
}
