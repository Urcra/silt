import { describe, expect, it } from 'vitest'
import { findAcross, renderedExcerpt } from './textFind'

describe("a doc comment's text in the read view", () => {
  it('reads as it renders', () => {
    expect(renderedExcerpt('## The **plan**\n- ship [it](https://x.y) `now`')).toBe('The plan ship it now')
  })
  it('is found across nodes, whitespace matching whitespace', () => {
    const texts = ['The ', 'plan', '\n\n', 'ship it now']
    expect(findAcross(texts, 'plan ship')).toEqual({ start: [1, 0], end: [3, 4] })
    expect(findAcross(texts, 'nothing')).toBeNull()
    expect(findAcross(texts, '  ')).toBeNull()
  })
})
