import { describe, expect, it } from 'vitest'
import { lineDiff } from './lineDiff'

describe('a line diff', () => {
  it('keeps what both share and marks what changed', () => {
    expect(lineDiff('# Plan\n- ship\n', '# Plan\n- test\n- ship on Friday\n')).toEqual([
      { kind: 'same', text: '# Plan' },
      { kind: 'removed', text: '- ship' },
      { kind: 'added', text: '- test' },
      { kind: 'added', text: '- ship on Friday' },
      { kind: 'same', text: '' },
    ])
  })

  it('finds nothing changed in the same text, and gives up on huge ones', () => {
    expect(lineDiff('a\nb', 'a\nb')?.every((line) => line.kind === 'same')).toBe(true)
    const many = Array.from({ length: 3000 }, (_, at) => `line ${at}`)
    expect(lineDiff(many.join('\n'), many.map((line) => `${line}!`).join('\n'))).toBeNull()
  })
})
