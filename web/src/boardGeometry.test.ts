import { describe, expect, it } from 'vitest'
import { crosses, edgeOf, route, within } from './boardGeometry'

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

describe("a whiteboard arrow's way", () => {
  const [a, b] = [
    { x: 0, y: 0, w: 100, h: 50 },
    { x: 400, y: 0, w: 100, h: 50 },
  ]
  it('goes straight where nothing lies between', () => {
    expect(route(a, b, [{ x: 150, y: 200, w: 100, h: 100 }])).toEqual([
      [102, 25],
      [394, 25],
    ])
  })
  it('goes round what does, clear of it', () => {
    const between = { x: 200, y: -40, w: 80, h: 130 }
    const way = route(a, b, [between])
    expect(way.length).toBeGreaterThan(2)
    for (let at = 1; at < way.length; at++) expect(crosses(way[at - 1], way[at], between)).toBe(false)
    const inside = ([x, y]: [number, number], box: typeof a) => x > box.x && x < box.x + box.w && y > box.y && y < box.y + box.h
    expect(inside(way[0], a) || inside(way[way.length - 1], b)).toBe(false)
  })
  it('stays straight when no way round is clear', () => {
    const around = [{ x: 350, y: -100, w: 200, h: 250 }]
    expect(route(a, b, around)).toHaveLength(2)
  })
})
