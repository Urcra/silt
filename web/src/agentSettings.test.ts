import { describe, expect, it } from 'vitest'
import type { AgentChoiceItem } from './api'
import { effortLevel } from './agentSettings'

const choices = (...ids: string[]): AgentChoiceItem[] =>
  ids.map((id) => ({ id, name: id[0].toUpperCase() + id.slice(1), description: null }))

describe('effortLevel', () => {
  const claude = choices('default', 'low', 'medium', 'high', 'xhigh', 'max')

  it('gives each level its own place and leaves the default out', () => {
    expect(claude.map((choice) => effortLevel(claude, choice.id))).toEqual([
      { at: null, of: 5 },
      { at: 0, of: 5 },
      { at: 1, of: 5 },
      { at: 2, of: 5 },
      { at: 3, of: 5 },
      { at: 4, of: 5 },
    ])
  })

  it('counts every choice where none is the default', () => {
    expect(effortLevel(choices('low', 'medium', 'high'), 'high')).toEqual({ at: 2, of: 3 })
  })

  it('has no level for a choice it no longer lists', () => {
    expect(effortLevel(claude, 'ultra')).toEqual({ at: null, of: 5 })
  })
})
