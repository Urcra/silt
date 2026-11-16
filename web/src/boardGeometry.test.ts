import { describe, expect, it } from 'vitest'
import { edgeOf, within } from './boardGeometry'

describe("a whiteboard arrow's ends", () => {
  const box = { x: 0, y: 0, w: 100, h: 50 }
  it('leave the item at its edge, past a gap, toward the other end', () => {
    expect(edgeOf(box, [300, 25], 0)).toEqual([100, 25])
    expect(edgeOf(box, [50, 200], 0)).toEqual([50, 50])
    expect(edgeOf(box, [-100, 25], 10)).toEqual([-10, 25])
    expect(edgeOf(box, [100, 125], 0)).toEqual([62.5, 50])
  })
  it('stay in the middle when the other end is inside the item', () => {
    expect(edgeOf(box, [60, 30])).toEqual([50, 25])
  })
})

describe('a section', () => {
  it('holds what lies wholly within it', () => {
    const section = { x: 0, y: 0, w: 400, h: 300 }
    expect(within({ x: 10, y: 10, w: 160, h: 100 }, section)).toBe(true)
    expect(within({ x: 300, y: 10, w: 160, h: 100 }, section)).toBe(false)
  })
})
