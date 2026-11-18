// Where a whiteboard's arrows meet the items they join.

export type Box = { x: number; y: number; w: number; h: number }

/** Whether `inner` lies wholly within `outer`, as a note in a section does. */
export function within(inner: Box, outer: Box): boolean {
  return (
    inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h
  )
}

/** The middle of `box`. */
export function centreOf(box: Box): [number, number] {
  return [box.x + box.w / 2, box.y + box.h / 2]
}

/**
 * Where a line from `box`'s middle toward `toward` leaves the box, `gap` past its edge, so an arrow's head shows
 * beside the item rather than under it. The middle itself when `toward` is inside the box.
 */
export function edgeOf(box: Box, toward: [number, number], gap = 6): [number, number] {
  const [cx, cy] = centreOf(box)
  const [dx, dy] = [toward[0] - cx, toward[1] - cy]
  const [hw, hh] = [box.w / 2 + gap, box.h / 2 + gap]
  if (Math.abs(dx) <= hw && Math.abs(dy) <= hh) return [cx, cy]
  const scale = Math.min(dx === 0 ? Infinity : hw / Math.abs(dx), dy === 0 ? Infinity : hh / Math.abs(dy))
  return [cx + dx * scale, cy + dy * scale]
}

type Point = [number, number]

/** Whether the segment from `a` to `b` passes through `box`'s inside (Liang–Barsky). */
export function crosses(a: Point, b: Point, box: Box): boolean {
  const [dx, dy] = [b[0] - a[0], b[1] - a[1]]
  let [low, high] = [0, 1]
  const edges: [number, number][] = [
    [-dx, a[0] - box.x],
    [dx, box.x + box.w - a[0]],
    [-dy, a[1] - box.y],
    [dy, box.y + box.h - a[1]],
  ]
  for (const [p, q] of edges) {
    if (p === 0) {
      if (q <= 0) return false
      continue
    }
    const t = q / p
    if (p < 0) low = Math.max(low, t)
    else high = Math.min(high, t)
    if (low >= high) return false
  }
  return true
}

function length(points: Point[]): number {
  let total = 0
  for (let at = 1; at < points.length; at++)
    total += Math.hypot(points[at][0] - points[at - 1][0], points[at][1] - points[at - 1][1])
  return total
}

/**
 * An arrow from `from` to `to` as the points it passes: straight where nothing lies between, else around what does,
 * by the corners of the items in the way, `margin` clear of them, taking the shortest way that's clear. Its ends sit at
 * the edges of the items it joins, facing where it goes; straight when no way round is clear.
 */
export function route(from: Box, to: Box, others: Box[], margin = 16): Point[] {
  const [start, end] = [centreOf(from), centreOf(to)]
  const grown = others.map((box) => ({ x: box.x - margin, y: box.y - margin, w: box.w + 2 * margin, h: box.h + 2 * margin }))
  // Only the line between the two items' edges counts, so it may start and end inside them.
  const clear = (a: Point, b: Point) => !others.some((box) => crosses(a, b, box))
  const path = (points: Point[]): Point[] => {
    const first = edgeOf(from, points.length > 2 ? points[1] : end, 2)
    const last = edgeOf(to, points.length > 2 ? points[points.length - 2] : start)
    return [first, ...points.slice(1, -1), last]
  }
  const straight = path([start, end])
  if (clear(straight[0], straight[1])) return straight
  const inTheWay = grown.filter((_, at) => crosses(straight[0], straight[1], others[at]))
  const corners = (box: Box): Point[] => [
    [box.x, box.y],
    [box.x + box.w, box.y],
    [box.x + box.w, box.y + box.h],
    [box.x, box.y + box.h],
  ]
  const waypoints = inTheWay
    .flatMap(corners)
    .filter(([x, y]) => !others.some((box) => x > box.x && x < box.x + box.w && y > box.y && y < box.y + box.h))
  let best: Point[] | null = null
  const consider = (points: Point[]) => {
    const drawn = path(points)
    for (let at = 1; at < drawn.length; at++) if (!clear(drawn[at - 1], drawn[at])) return
    if (!best || length(drawn) < length(best)) best = drawn
  }
  for (const one of waypoints) consider([start, one, end])
  if (!best) for (const one of waypoints) for (const two of waypoints) if (one !== two) consider([start, one, two, end])
  return best ?? straight
}
