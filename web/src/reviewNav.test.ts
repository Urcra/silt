import { expect, test } from 'vitest'
import { parseFiles } from './diffParse'
import { defaultTurns, filtered, firstChange, linesNew, nextUnseen, pickTurns, seenOf, stepFile, stepHunk, turnsOfHunk } from './reviewNav'

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

test("a hunk names the turns that wrote what it added, or the turn shown when it only deleted", () => {
  const [file] = parseFiles('diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n fn a() {}\n-old()\n+new()\n+more()\n fn b() {}\n@@ -9,2 +10,1 @@\n fn c() {}\n-gone()\n')
  const runs = [
    { start: 1, end: 2, turn: 1 },
    { start: 3, end: 3, turn: 3 },
  ]
  expect(turnsOfHunk(file.hunks[0], runs)).toEqual([1, 3])
  expect(turnsOfHunk(file.hunks[1], runs)).toEqual([])
  expect(turnsOfHunk(file.hunks[1], runs, 2)).toEqual([2])
})

test('a click picks a turn, and a shift-click the range from the first picked', () => {
  expect(pickTurns(undefined, 3, true)).toEqual([3, 3])
  expect(pickTurns([3, 3], 5, true)).toEqual([3, 5])
  expect(pickTurns([3, 5], 1, true)).toEqual([1, 3])
  expect(pickTurns([3, 5], 4, false)).toEqual([4, 4])
})

test('a file reads as seen, changed since with how many lines are new, or unmarked', () => {
  const marks = new Map([
    ['a.rs', { path: 'a.rs', seen: 'seen' as const, new_lines: [] }],
    ['b.rs', { path: 'b.rs', seen: 'changed' as const, new_lines: [{ start: 3, end: 4 }, { start: 9, end: 9 }] }],
  ])
  expect(['a.rs', 'b.rs', 'c.rs'].map((path) => seenOf(path, marks))).toEqual(['yes', 'changed', 'no'])
  expect(linesNew(marks.get('b.rs'))).toBe(3)
  // ] goes on to the next file not seen as it is now, round to the start.
  expect(nextUnseen(['a.rs', 'b.rs', 'c.rs'], marks, 1)).toBe(2)
  expect(nextUnseen(['a.rs', 'b.rs', 'c.rs'], marks, 2)).toBe(1)
  expect(nextUnseen(['a.rs'], marks, 0)).toBeNull()
})

test('a review starts after the turn the person reviewed through, while there are turns since', () => {
  expect(defaultTurns(5, 3)).toEqual([4, 5])
  expect(defaultTurns(3, 3)).toBeUndefined()
  expect(defaultTurns(3, null)).toBeUndefined()
})
