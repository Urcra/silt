import { describe, expect, it } from 'vitest'
import type { CommentView } from './api'
import { parseFiles } from './diffParse'
import { itemOf, linesOf, quoteOf, reviewPrompt } from './review'

const DIFF = `diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {}
-fn old() {}
+fn new() {}
 fn end() {}
`

function comment(location: CommentView['location']): CommentView {
  return { id: 'c1', by: 'alice', time_ms: 0, text: 'why?', resolved: false, sent: false, location, replies: [] }
}

describe('review', () => {
  it('places a comment on deleted lines where the base has them', () => {
    const lines = { at: 'lines', path: 'src/main.rs', start: 2, end: 2, outdated: true, before: { start: 2, end: 2 } } as const
    expect(linesOf(comment(lines))).toEqual({ path: 'src/main.rs', start: 2, end: 2, side: 'old' })
    expect(linesOf(comment({ ...lines, outdated: false }))).toEqual({ path: 'src/main.rs', start: 2, end: 2, side: 'new' })
  })

  it('quotes the lines on the side commented', () => {
    const files = parseFiles(DIFF)
    expect(quoteOf(files, { path: 'src/main.rs', start: 2, end: 3 })).toEqual(['+fn new() {}', ' fn end() {}'])
    expect(quoteOf(files, { path: 'src/main.rs', start: 2, end: 2, side: 'old' })).toEqual(['-fn old() {}'])
    expect(quoteOf(files, { path: 'other.rs', start: 1, end: 1 })).toEqual([])
  })

  it('sends the note, then each comment under its lines', () => {
    const files = parseFiles(DIFF)
    const deleted = itemOf(comment({ at: 'lines', path: 'src/main.rs', start: 2, end: 2, outdated: true, before: { start: 2, end: 2 } }), files)!
    const prompt = reviewPrompt([deleted, { at: { path: 'src/main.rs', start: 2, end: 2 }, text: 'rename it' }], 'Close, a few things.')
    expect(prompt).toBe(
      [
        'Close, a few things.',
        '',
        'Please address these review comments:',
        '- src/main.rs:2, lines the change deleted (alice): why?',
        '```diff',
        '-fn old() {}',
        '```',
        '- src/main.rs:2: rename it',
      ].join('\n'),
    )
  })

  it('fences quotes longer than the backticks they hold', () => {
    const prompt = reviewPrompt([{ at: { path: 'README.md', start: 1, end: 1 }, text: 'fix', quote: ['+```rust'] }])
    expect(prompt).toContain('````diff\n+```rust\n````')
  })
})
