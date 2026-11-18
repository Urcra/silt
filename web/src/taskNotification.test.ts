import { expect, test } from 'vitest'
import { taskNotification } from './taskNotification'

test("a background task's report reads as its summary and result", () => {
  const text = [
    '<task-notification>',
    '<task-id>a0eb0a4730fa314a7</task-id>',
    '<status>completed</status>',
    '<summary>Agent "placeholder wait" finished</summary>',
    '<result>The tool has a **Linux** build.</result>',
    '<usage><tool_uses>3</tool_uses></usage>',
    '</task-notification>',
  ].join('\n')
  expect(taskNotification(text)).toEqual({
    summary: 'Agent "placeholder wait" finished',
    status: 'completed',
    result: 'The tool has a **Linux** build.',
  })
})

test('a prompt that only mentions a notification is a prompt', () => {
  expect(taskNotification('What does <task-notification> mean?')).toBeNull()
  expect(taskNotification('Fix the lexer')).toBeNull()
  expect(taskNotification('<task-notification>\n<status>failed</status>')).toEqual({
    summary: 'A background task reported back',
    status: 'failed',
    result: '',
  })
})
