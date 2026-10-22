import { expect, test } from 'vitest'
import { parseDiff, parseFiles, splitRows } from './diffParse'

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
