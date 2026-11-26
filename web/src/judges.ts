import type { AgentChoice, Judge } from './api'
import { isModel } from './agentSettings'

/** Whether two judges of Auto's check are the same agent on the same model (ADR 0132). */
export function sameJudge(a: Judge, b: Judge): boolean {
  if (a.kind === 'thread' || b.kind === 'thread') return a.kind === b.kind
  return a.agent === b.agent && (a.model ?? null) === (b.model ?? null)
}

/** A judge as the person knows it: `GitHub Copilot on Claude Haiku 5.5`. */
export function judgeName(judge: Judge | null | undefined, agents: AgentChoice[]): string {
  if (!judge || judge.kind === 'thread') return 'Same as the thread’s agent and model'
  const agent = agents.find((each) => each.id === judge.agent)
  const name = agent?.name ?? judge.agent
  if (!judge.model) return name
  const model = agent?.settings.find(isModel)?.choices.find((choice) => choice.id === judge.model)?.name ?? judge.model
  return `${name} on ${model}`
}
