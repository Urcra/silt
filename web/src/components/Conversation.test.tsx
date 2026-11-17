import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, expect, test } from 'vitest'
import type { ShowLine } from '../api'
import { Conversation } from './Conversation'

afterEach(cleanup)

test("another agent's edits in the clone are labelled as theirs, not this thread's", () => {
  const lines: ShowLine[] = [
    { line: 'tool_call', id: 't1', name: 'Edit', input: '{}' },
    { line: 'edits', tool_use_id: 't1', files: [{ path: 'src/mine.rs', inserted: 1, deleted: 0 }] },
    { line: 'edits', tool_use_id: null, files: [{ path: 'src/theirs.rs', inserted: 2, deleted: 1 }], shared: { others: true, agents: ['claude-code 1a2b3c4d'] } },
    { line: 'edits', tool_use_id: null, files: [{ path: 'src/either.rs', inserted: 1, deleted: 1 }], shared: { others: false, agents: ['claude-code 1a2b3c4d', 'codex 5e6f7a8b'] } },
  ]
  render(<Conversation lines={lines} />)
  expect(screen.getByText(/^Ran 1 tool/)).toHaveTextContent('Ran 1 tool · edited mine.rs, either.rs · another agent edited theirs.rs')
  expect(screen.getByText('src/theirs.rs').closest('p')).toHaveTextContent('✎ by another agent (claude-code 1a2b3c4d)')
  expect(screen.getByText('src/either.rs').closest('p')).toHaveTextContent('✎ by one of these agents (claude-code 1a2b3c4d, codex 5e6f7a8b)')
  expect(screen.getByText('src/mine.rs').closest('p')).not.toHaveTextContent('✎')
})
