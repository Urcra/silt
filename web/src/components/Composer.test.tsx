import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { useState } from 'react'
import { afterEach, expect, test } from 'vitest'
import { ComposerBox } from './Composer'

afterEach(cleanup)

function Draft({
  id = 'prompt',
  initial = 'Please fix the bug',
  suggestion,
  onSend = () => {},
}: {
  id?: string
  initial?: string
  suggestion?: string
  onSend?: () => void
}) {
  const [value, setValue] = useState(initial)
  const [client] = useState(() => new QueryClient())
  return (
    <QueryClientProvider client={client}>
      <ComposerBox
        id={id}
        label="prompt"
        value={value}
        onChange={setValue}
        onSend={() => {
          onSend()
          setValue('')
        }}
        placeholder="Message"
        suggestion={suggestion}
      />
    </QueryClientProvider>
  )
}

function box() {
  return screen.getByRole('textbox', { name: 'prompt' }) as HTMLTextAreaElement
}

function select(start: number, end = start) {
  box().focus()
  box().setSelectionRange(start, end)
  fireEvent.select(box())
}

function input(value: string, inputType: string, caret = value.length) {
  fireEvent.input(box(), { target: { value, selectionStart: caret, selectionEnd: caret }, inputType })
}

test('undo restores a deleted part of a prompt and its selection; redo removes it again', () => {
  render(<Draft />)
  select(7, 15)
  fireEvent.keyDown(box(), { key: 'Backspace' })
  input('Please bug', 'deleteContentBackward', 7)
  fireEvent.keyDown(box(), { key: 'z', ctrlKey: true })
  expect(box()).toHaveValue('Please fix the bug')
  expect([box().selectionStart, box().selectionEnd]).toEqual([7, 15])
  fireEvent.keyDown(box(), { key: 'Z', ctrlKey: true, shiftKey: true })
  expect(box()).toHaveValue('Please bug')
  expect(box().selectionStart).toBe(7)
})

test('typing groups into one undo step, separate from backspacing', () => {
  render(<Draft initial="" />)
  select(0)
  input('h', 'insertText')
  input('hi', 'insertText')
  fireEvent.keyDown(box(), { key: 'Backspace' })
  input('h', 'deleteContentBackward')
  fireEvent.keyDown(box(), { key: 'z', metaKey: true })
  expect(box()).toHaveValue('hi')
  fireEvent.keyDown(box(), { key: 'z', metaKey: true })
  expect(box()).toHaveValue('')
  fireEvent.keyDown(box(), { key: 'y', ctrlKey: true })
  expect(box()).toHaveValue('hi')
})

test('editing after undo drops redo and pasting is its own step', () => {
  render(<Draft initial="" />)
  select(0)
  input('first', 'insertFromPaste')
  input('first second', 'insertFromPaste')
  fireEvent.keyDown(box(), { key: 'z', ctrlKey: true })
  expect(box()).toHaveValue('first')
  input('first!', 'insertText')
  fireEvent.keyDown(box(), { key: 'y', ctrlKey: true })
  expect(box()).toHaveValue('first!')
  fireEvent.keyDown(box(), { key: 'z', ctrlKey: true })
  expect(box()).toHaveValue('first')
})

test('sending or switching to another composer clears undo history', () => {
  const view = render(<Draft />)
  select(18)
  input('Please fix the bug!', 'insertText')
  fireEvent.keyDown(box(), { key: 'Enter' })
  fireEvent.keyDown(box(), { key: 'z', ctrlKey: true })
  expect(box()).toHaveValue('')
  input('another draft', 'insertFromPaste')
  view.rerender(<Draft id="other-thread" />)
  fireEvent.keyDown(box(), { key: 'z', ctrlKey: true })
  expect(box()).toHaveValue('another draft')
})

test('clearing the whole draft is undoable until it is sent', () => {
  render(<Draft />)
  select(0, 18)
  input('', 'deleteContentBackward')
  fireEvent.keyDown(box(), { key: 'z', ctrlKey: true })
  expect(box()).toHaveValue('Please fix the bug')
})

test("Tab or → puts the agent's suggestion in the empty box without sending it, and undo takes it out", () => {
  for (const key of ['Tab', 'ArrowRight']) {
    let sent = 0
    const view = render(<Draft initial="" suggestion="Add a test for it" onSend={() => sent++} />)
    expect(document.getElementById('prompt-suggestion')).toHaveTextContent('Add a test for it')
    expect(box()).toHaveAttribute('placeholder', '')
    box().focus()
    fireEvent.keyDown(box(), { key })
    expect(box()).toHaveValue('Add a test for it')
    expect(document.getElementById('prompt-suggestion')).toBeNull()
    expect(sent).toBe(0)
    fireEvent.keyDown(box(), { key: 'z', ctrlKey: true })
    expect(box()).toHaveValue('')
    view.unmount()
  }
})

test('a draft hides the suggestion, and Tab and → keep their own meaning there', () => {
  render(<Draft initial="" suggestion="Add a test for it" />)
  select(0)
  input('Also', 'insertText')
  expect(document.getElementById('prompt-suggestion')).toBeNull()
  expect(box()).toHaveAttribute('placeholder', 'Message')
  expect(fireEvent.keyDown(box(), { key: 'Tab' })).toBe(true)
  expect(fireEvent.keyDown(box(), { key: 'ArrowRight' })).toBe(true)
  expect(box()).toHaveValue('Also')
})

test('Shift+Tab leaves a suggestion where it is', () => {
  render(<Draft initial="" suggestion="Add a test for it" />)
  fireEvent.keyDown(box(), { key: 'Tab', shiftKey: true })
  expect(box()).toHaveValue('')
  expect(document.getElementById('prompt-suggestion')).toHaveTextContent('Add a test for it')
})
