import { expect, test } from 'vitest'
import type { AgentCommand } from './api'
import { matching, typedCommand } from './slashCommands'

const command = (name: string): AgentCommand => ({ name, description: '', hint: null })

test('a command is being typed from the slash until a space', () => {
  expect(typedCommand('/')).toBe('')
  expect(typedCommand('/rev')).toBe('rev')
  expect(typedCommand('/review ')).toBeNull()
  expect(typedCommand('see /review')).toBeNull()
  expect(typedCommand('/review\nnext')).toBeNull()
})

test('commands that start with what was typed come before those that hold it', () => {
  const commands = ['engineering:code-review', 'review', 'compact', 'security-review'].map(command)
  expect(matching(commands, 'rev').map((found) => found.name)).toEqual(['review', 'engineering:code-review', 'security-review'])
  expect(matching(commands, 'REV').map((found) => found.name)).toEqual(['review', 'engineering:code-review', 'security-review'])
  expect(matching(commands, '')).toEqual(commands)
})
