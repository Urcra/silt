import { describe, expect, it } from 'vitest'
import { modeForPreset } from './agentState'
import { CHATS, isChat } from './chats'
import { projectName } from './projects'

describe('chats', () => {
  it('are a project of their own, named Chats', () => {
    expect(isChat(CHATS)).toBe(true)
    expect(isChat('github.com/acme/app')).toBe(false)
    expect(projectName(CHATS)).toBe('Chats')
  })

  it('pick the agent mode their preset says, as the daemon does', () => {
    const modes = [
      { id: 'default', name: 'Default', description: null },
      { id: 'acceptEdits', name: 'Accept edits', description: null },
      { id: 'bypassPermissions', name: 'Bypass', description: null },
    ]
    expect(modeForPreset('full_access', modes)).toBe('bypassPermissions')
    expect(modeForPreset('ask_before_commands', modes)).toBe('acceptEdits')
    expect(modeForPreset('full_access', [{ id: 'code', name: 'Code', description: null }])).toBeUndefined()
  })
})
