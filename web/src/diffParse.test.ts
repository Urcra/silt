import { expect, test } from 'vitest'
import { LARGE_DIFF, LARGE_TOTAL, MANY_FILES, bytes, diffSize, parseDiff, parseFiles, readWhole, splitRows } from './diffParse'

test('a binary file says so, with no lines to count', () => {
  const diff = [
    'diff --git a/x.pyc b/x.pyc',
    'Binary files /dev/null and b/x.pyc differ',
    'diff --git a/a.rs b/a.rs',
    '@@ -1 +1 @@',
    '-a',
    '+b',
  ].join('\n')
  expect(parseFiles(diff).map((file) => [file.path, file.binary])).toEqual([
    ['x.pyc', true],
    ['a.rs', false],
  ])
})

test('a diff reads as files with their counts, and hunks pair up side by side', () => {
  const diff = [
    'diff --git a/src/main.rs b/src/main.rs',
    '--- a/src/main.rs',
    '+++ b/src/main.rs',
    '@@ -1,4 +1,4 @@',
    ' fn main() {}',
    '-fn old() {}',
    '-fn older() {}',
    '+fn new() {}',
    ' fn last() {}',
    'diff --git a/added.rs b/added.rs',
    'new file mode 100644',
    '--- /dev/null',
    '+++ b/added.rs',
    '@@ -0,0 +1 @@',
    '+fn added() {}',
    '',
  ].join('\n')
  const files = parseFiles(diff)
  expect(files.map((file) => [file.path, file.added, file.removed, file.note])).toEqual([
    ['src/main.rs', 1, 2, null],
    ['added.rs', 1, 0, 'new'],
  ])
  const rows = splitRows(files[0].hunks[0])
  expect(rows.map((row) => [row.left?.oldLine ?? null, row.right?.newLine ?? null])).toEqual([
    [1, 1],
    [2, 2],
    [3, null],
    [4, 3],
  ])
})

test('lines know their file and their number in the new file', () => {
  const diff = [
    'diff --git a/src/main.rs b/src/main.rs',
    '--- a/src/main.rs',
    '+++ b/src/main.rs',
    '@@ -1,3 +1,3 @@',
    ' fn main() {}',
    '-fn old() {}',
    '+fn new() {}',
    ' fn last() {}',
    'diff --git a/gone.rs b/gone.rs',
    '--- a/gone.rs',
    '+++ /dev/null',
    '@@ -1 +0,0 @@',
    '-fn gone() {}',
    '',
  ].join('\n')
  const lines = parseDiff(diff)
  const numbered = lines.filter((line) => line.newLine !== null).map((line) => [line.path, line.newLine, line.text])
  expect(numbered).toEqual([
    ['src/main.rs', 1, ' fn main() {}'],
    ['src/main.rs', 2, '+fn new() {}'],
    ['src/main.rs', 3, ' fn last() {}'],
  ])
  expect(lines.find((line) => line.text === '-fn gone() {}')?.path).toBe('gone.rs')
})

test("a file's size is the characters of its lines, so one minified line counts in full", () => {
  const minified = 'x'.repeat(LARGE_DIFF + 1)
  const diff = [
    'diff --git a/a.rs b/a.rs',
    '@@ -1 +1 @@',
    '-ab',
    '+abc',
    'diff --git a/data.json b/data.json',
    'new file mode 100644',
    '@@ -0,0 +1 @@',
    `+${minified}`,
  ].join('\n')
  expect(parseFiles(diff).map((file) => [file.path, diffSize(file), diffSize(file) > LARGE_DIFF])).toEqual([
    ['a.rs', 5, false],
    ['data.json', LARGE_DIFF + 1, true],
  ])
})

test('a diff of too many files, or too many bytes in all, is read a file at a time', () => {
  const stat = (size: number) => ({ path: 'a', old_path: null, added: 1, removed: 0, binary: false, size })
  expect(readWhole([stat(100), stat(200)])).toBe(true)
  expect(readWhole(Array.from({ length: MANY_FILES + 1 }, () => stat(10)))).toBe(false)
  expect(readWhole([stat(LARGE_TOTAL / 2), stat(LARGE_TOTAL / 2 + 1)])).toBe(false)
  expect([bytes(512), bytes(12_345), bytes(448 * 1024 * 1024)]).toEqual(['512 B', '12 KB', '448 MB'])
})
