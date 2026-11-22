import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, expect, test } from 'vitest'
import { AgentMark } from './AgentMark'

afterEach(cleanup)

test("an agent Silt knows shows its logo, named where it's labelled", () => {
  render(<AgentMark agent="claude-acp" labelled />)
  const mark = screen.getByRole('img', { name: 'Claude Code' })
  expect(mark.tagName.toLowerCase()).toBe('svg')
  expect(mark.querySelectorAll('path').length).toBeGreaterThan(0)
})

test('an agent without a logo shows its initial', () => {
  render(<AgentMark agent="junie-acp" name="Junie" labelled />)
  expect(screen.getByRole('img', { name: 'Junie' }).textContent).toBe('J')
})

test('unlabelled, it is hidden from a screen reader', () => {
  const { container } = render(<AgentMark agent="codex" />)
  expect(container.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true')
  expect(screen.queryByRole('img')).toBeNull()
})
