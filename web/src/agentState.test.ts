import { describe, expect, it } from 'vitest'
import type { AgentChoice, AgentSetting, AgentStatus } from './api'
import { modeForPreset, nextPreset, presetUnavailable, restartChoices, threadAgent } from './agentState'

const model = (current: string): AgentSetting => ({
  id: 'model',
  name: 'Model',
  category: 'model',
  current,
  choices: ['gpt-6.2-sol', 'claude-opus-5.5', 'claude-sonnet-5.5'].map((id) => ({ id, name: id, description: null })),
})
const choice = (id: string, name: string, current: string) =>
  ({ id, name, description: '', modes: null, settings: [model(current)] }) as unknown as AgentChoice
const stopped = (agent_id: string, current: string) =>
  ({ agent: agent_id, agent_id, state: 'stopped', modes: null, settings: [model(current)] }) as unknown as AgentStatus

const agents = [
  choice('antigravity-acp', 'Google Antigravity', 'x'),
  choice('github-copilot-cli', 'GitHub Copilot', 'gpt-6.2-sol'),
]
const none = () => undefined

describe('threadAgent', () => {
  it('starts the agent the thread ran, not the first one listed', () => {
    expect(threadAgent(agents, stopped('github-copilot-cli', 'claude-opus-5.5'), null, none)).toBe('github-copilot-cli')
    // Once the daemon restarted, by the name the thread keeps.
    expect(threadAgent(agents, null, 'GitHub Copilot', none)).toBe('github-copilot-cli')
  })

  it('falls back to the first agent that may run when the thread ran none here, or one now refused', () => {
    expect(threadAgent(agents, null, null, none)).toBe('antigravity-acp')
    const onlyAntigravity = (id: string) => (id === 'antigravity-acp' ? undefined : 'refused')
    expect(threadAgent(agents, null, 'GitHub Copilot', onlyAntigravity)).toBe('antigravity-acp')
    expect(threadAgent(agents, stopped('gone', 'x'), null, none)).toBe('antigravity-acp')
  })
})

describe('restartChoices', () => {
  const copilot = agents[1]

  it("keeps the thread's own model over the one the agent last ran with anywhere, and over the workspace's", () => {
    const restart = restartChoices('github-copilot-cli', stopped('github-copilot-cli', 'claude-opus-5.5'), copilot, {
      model: 'claude-sonnet-5.5',
    })
    const [setting] = restart.settings
    expect(restart.value(setting!, undefined)).toBe('claude-opus-5.5')
    expect(restart.value(setting!, 'gpt-6.2-sol')).toBe('gpt-6.2-sol')
  })

  it("starts an agent new to the thread on the workspace's default, else as it last ran", () => {
    const restart = restartChoices('github-copilot-cli', null, copilot, { model: 'claude-opus-5.5' })
    expect(restart.value(restart.settings[0]!, undefined)).toBe('claude-opus-5.5')
    const plain = restartChoices('github-copilot-cli', null, copilot, undefined)
    expect(plain.value(plain.settings[0]!, undefined)).toBe('gpt-6.2-sol')
    const other = restartChoices('github-copilot-cli', stopped('antigravity-acp', 'x'), copilot, undefined)
    expect(other.value(other.settings[0]!, undefined)).toBe('gpt-6.2-sol')
  })
})

describe('presets', () => {
  const modes = (...ids: string[]) => ids.map((id) => ({ id, name: id, description: null }))

  it('step to the next one allowed, past what is refused', () => {
    const noFull = (preset: string) => (preset === 'full_access' ? 'capped' : undefined)
    expect(nextPreset('ask_before_commands', noFull)).toBe('auto')
    expect(nextPreset('auto', noFull)).toBe('plan')
    expect(nextPreset('plan', () => 'all refused')).toBeUndefined()
  })

  it('offer Auto only to an agent with an auto mode, once its modes are known', () => {
    expect(presetUnavailable('auto', modes('default', 'acceptEdits', 'auto'), 'Claude Code')).toBeUndefined()
    expect(presetUnavailable('auto', modes('read-only', 'auto', 'full-access'), 'Codex')).toBeUndefined()
    const copilot = modes('#agent', '#plan', '#autopilot').map((mode) => ({ ...mode, id: `https://x/session-modes${mode.id}` }))
    expect(presetUnavailable('auto', copilot, 'GitHub Copilot')).toBe('GitHub Copilot has no auto mode')
    expect(presetUnavailable('auto', undefined, 'GitHub Copilot')).toBeUndefined()
    expect(presetUnavailable('full_access', copilot, 'GitHub Copilot')).toBeUndefined()
  })

  it('move an agent without an auto mode to the one for Ask before commands', () => {
    expect(modeForPreset('auto', modes('default', 'acceptEdits', 'auto'))).toBe('auto')
    expect(modeForPreset('auto', modes('default', 'acceptEdits', 'bypassPermissions'))).toBe('acceptEdits')
  })
})
