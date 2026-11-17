import { describe, expect, it } from 'vitest'
import { docBlocks } from './docBlocks'

describe("a doc's blocks", () => {
  it('are where they are in the source, prose editable and the rest kept as written', () => {
    const text = '# Plan\n\nShip it on <?silt date 2026-11-16?>.\n\n```js\nx\n```\n\n<?silt chart {"type":"bar"} ?>\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n- one\n- two\n'
    const blocks = docBlocks(text)
    expect(blocks.map((block) => text.slice(block.start, block.end))).toEqual([
      '# Plan',
      'Ship it on <?silt date 2026-11-16?>.',
      '```js\nx\n```',
      '<?silt chart {"type":"bar"} ?>',
      '| a | b |\n|---|---|\n| 1 | 2 |',
      '- one\n- two',
    ])
    expect(blocks.map((block) => block.editable)).toEqual([true, true, false, false, false, true])
    expect(blocks[0].depth).toBe(1)
  })
})
