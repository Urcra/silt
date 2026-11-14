import { expect, test } from 'vitest'
import { parseFiles } from './diffParse'
import { filtered, firstChange, stepFile, stepHunk } from './reviewNav'

// Three files: two hunks, a binary file with none, and one hunk.
const HUNKS = [2, 0, 1]

test('j and k move a file at a time, stopping at either end', () => {
  expect(stepFile(HUNKS, { file: 0, hunk: 1 }, 1)).toEqual({ file: 1, hunk: 0 })
  expect(stepFile(HUNKS, { file: 2, hunk: 0 }, 1)).toBeNull()
  expect(stepFile(HUNKS, { file: 0, hunk: 0 }, -1)).toBeNull()
})

test('n and p move a hunk at a time across files, past a file with none', () => {
  expect(stepHunk(HUNKS, { file: 0, hunk: 0 }, 1)).toEqual({ file: 0, hunk: 1 })
  expect(stepHunk(HUNKS, { file: 0, hunk: 1 }, 1)).toEqual({ file: 2, hunk: 0 })
  expect(stepHunk(HUNKS, { file: 2, hunk: 0 }, -1)).toEqual({ file: 0, hunk: 1 })
  expect(stepHunk(HUNKS, { file: 2, hunk: 0 }, 1)).toBeNull()
  expect(stepHunk(HUNKS, { file: 0, hunk: 0 }, -1)).toBeNull()
})

test('the filter keeps paths holding it, whatever their case', () => {
  const files = [{ path: 'src/Upload.rs' }, { path: 'src/chunk.rs' }]
  expect(filtered(files, 'upload')).toEqual([{ path: 'src/Upload.rs' }])
  expect(filtered(files, '  ')).toEqual(files)
})

test('c comments on the first line a hunk changed, a deleted one only where those take comments', () => {
  const [file] = parseFiles('diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@\n fn a() {}\n-old()\n+new()\n fn b() {}\n')
  expect(firstChange(file, 0, true)).toEqual({ path: 'src/a.rs', start: 2, end: 2, side: 'old' })
  expect(firstChange(file, 0, false)).toEqual({ path: 'src/a.rs', start: 2, end: 2, side: 'new' })
  expect(firstChange(file, 1, true)).toBeNull()
})
