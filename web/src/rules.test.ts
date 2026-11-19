import { expect, test } from 'vitest'
import type { WorkspaceView } from './api'
import { agentKey, agentRefused, presetRefused, rulesLine, within } from './rules'

function work(org: boolean): WorkspaceView {
  const policy = { agents: ['copilot'], max_preset: 'ask_before_commands' as const }
  return {
    workspace: {
      id: 'work',
      name: 'Work',
      server: { url: 'https://silt.acme.dev', org: 'acme' },
      rules: [],
      defaults: {},
      env: {},
      policy: org ? {} : policy,
    },
    org_policy: org ? { org: 'acme', server: 'https://silt.acme.dev', policy } : null,
    waiting_for_policy: false,
    effective: policy,
    projects: [],
  }
}

test('presets are capped by what goes through without asking', () => {
  expect(within('plan', 'ask_before_commands')).toBe(true)
  expect(within('full_access', 'ask_before_commands')).toBe(false)
  expect(within('full_access', null)).toBe(true)
})

test('an agent the rules refuse says whose rule it is', () => {
  expect(agentKey('claude-acp')).toBe('claude')
  expect(agentRefused(work(true), 'github-copilot-cli')).toBeUndefined()
  expect(agentRefused(work(true), 'claude-acp')).toBe("Not allowed in Work · acme's policy")
  expect(agentRefused(work(false), 'codex-acp')).toBe('Not allowed in Work · your rules')
  expect(presetRefused(work(true), 'full_access')).toBe('acme allows at most Ask before commands')
  expect(presetRefused(work(false), 'full_access')).toBe('Your rules allow at most Ask before commands')
  expect(presetRefused(work(true), 'plan')).toBeUndefined()
  expect(rulesLine(work(true).effective)).toBe('GitHub Copilot only · at most Ask before commands')
})
