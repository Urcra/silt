// Where a whiteboard's arrows meet the items they join.

export type Box = { x: number; y: number; w: number; h: number }

/** Whether `inner` lies wholly within `outer`, as a note in a section does. */
export function within(inner: Box, outer: Box): boolean {
  return inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h
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
