import { expect, test } from 'vitest'
import { pastedName } from './attachments'

test('a screenshot the clipboard names image.png is numbered', () => {
  expect(pastedName({ name: 'image.png', type: 'image/png' }, 1)).toBe('pasted-1.png')
  expect(pastedName({ name: 'image.png', type: 'image/png' }, 2)).toBe('pasted-2.png')
})

test('a pasted file without a name is named by its type', () => {
  expect(pastedName({ name: '', type: 'image/jpeg' }, 3)).toBe('pasted-3.jpg')
  expect(pastedName({ name: '', type: 'image/svg+xml' }, 1)).toBe('pasted-1.svg')
  expect(pastedName({ name: '', type: '' }, 1)).toBe('pasted-1.bin')
})

test("a copied file keeps its own name", () => {
  expect(pastedName({ name: 'diagram.png', type: 'image/png' }, 1)).toBe('diagram.png')
})
