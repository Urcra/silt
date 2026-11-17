import { expect, test } from 'vitest'
import { FILE_BYTES, FILES_BYTES, pastedName, tooLarge } from './attachments'

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

test('files over what a prompt takes are refused before they are read', () => {
  expect(tooLarge([{ name: 'book.xls', size: FILE_BYTES }], 0)).toBeNull()
  expect(tooLarge([{ name: 'book.xls', size: FILE_BYTES + 1 }], 0)).toMatch(/^book\.xls is too large/)
  expect(tooLarge([{ name: 'a.xls', size: FILE_BYTES }, { name: 'b.xls', size: FILE_BYTES }], 0)).toBeNull()
  expect(tooLarge([{ name: 'c.xls', size: FILE_BYTES }], FILES_BYTES - FILE_BYTES + 1)).toMatch(/too large together/)
})
