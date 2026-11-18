import { describe, expect, it } from 'vitest'
import { markdownBlocks } from './markdownBlocks'

describe('markdownBlocks', () => {
  it('cuts at blank lines between paragraphs and headings', () => {
    expect(markdownBlocks('# Title\n\nFirst.\n\nSecond')).toEqual(['# Title\n', 'First.\n', 'Second'])
  })

  it('keeps a fenced block whole, blank lines and all', () => {
    const text = 'Look:\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\nAfter'
    expect(markdownBlocks(text)).toEqual(['Look:\n', '```rust\nfn a() {}\n\nfn b() {}\n```\n', 'After'])
  })

  it("keeps a loose list's items and what's indented under them together", () => {
    const text = '- one\n\n- two\n\n  more of two\n\nThe end'
    expect(markdownBlocks(text)).toEqual(['- one\n\n- two\n\n  more of two\n', 'The end'])
  })

  it('leaves an unfinished fence open to the end', () => {
    expect(markdownBlocks('```\nstill\n\ngoing')).toEqual(['```\nstill\n\ngoing'])
  })

  it('gives back the text whole when joined', () => {
    const text = 'a\n\n\nb\n\n```\nx\n\n```\n\n1. c\n\n2. d'
    expect(markdownBlocks(text).join('\n')).toBe(text)
  })
})
