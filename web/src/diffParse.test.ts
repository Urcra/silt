import { expect, test } from 'vitest'
import { parseDiff } from './diffParse'

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
