import { expect, test } from 'vitest'
import { fitScale, fitted, MAX_SCALE, pannable, panned, pastPreview, percent, placed, visible, zoomedAt } from './zoom'

const stage = { width: 1000, height: 600 }
const chart = { width: 3200, height: 1800 }

test('an image fits its stage with a margin and is never blown up to fit', () => {
  expect(fitScale(stage, chart)).toBeCloseTo(952 / 3200)
  expect(fitScale(stage, { width: 200, height: 100 })).toBe(1)
  expect(fitScale(stage, { width: 0, height: 0 })).toBe(1)
})

test('a fitted image is centred', () => {
  const view = fitted(stage, chart)
  expect(view.x).toBeCloseTo(24)
  expect(view.y).toBeCloseTo((600 - 1800 * view.scale) / 2)
})

test('zooming keeps the point under the pointer over the same part of the image', () => {
  const at = fitted(stage, chart)
  const px = 700
  const py = 300
  const before = { x: (px - at.x) / at.scale, y: (py - at.y) / at.scale }
  const zoomed = zoomedAt(at, 1, px, py, stage, chart)
  expect(zoomed.scale).toBe(1)
  expect((px - zoomed.x) / zoomed.scale).toBeCloseTo(before.x)
  expect((py - zoomed.y) / zoomed.scale).toBeCloseTo(before.y)
})

test('zoom stops at 800% and at half the fit', () => {
  const at = fitted(stage, chart)
  expect(zoomedAt(at, 100, 0, 0, stage, chart).scale).toBe(MAX_SCALE)
  expect(zoomedAt(at, 0.001, 0, 0, stage, chart).scale).toBeCloseTo(at.scale / 2)
})

test('a zoomed image pans only as far as its edges', () => {
  const at = zoomedAt(fitted(stage, chart), 1, 500, 300, stage, chart)
  expect(pannable(at, stage, chart)).toBe(true)
  expect(panned(at, 100_000, 100_000, stage, chart)).toMatchObject({ x: 0, y: 0 })
  expect(panned(at, -100_000, -100_000, stage, chart)).toMatchObject({ x: 1000 - 3200, y: 600 - 1800 })
  expect(pannable(fitted(stage, chart), stage, chart)).toBe(false)
  expect(placed({ scale: 1, x: 50, y: 0 }, stage, { width: 200, height: 100 })).toEqual({ scale: 1, x: 400, y: 250 })
})

test('the navigator shows the part on screen', () => {
  expect(visible({ scale: 1, x: -1600, y: -900 }, stage, chart)).toEqual({
    left: 0.5,
    top: 0.5,
    width: 1000 / 3200,
    height: 600 / 1800,
  })
})

test('a preview zoomed past its pixels says so; an original never does', () => {
  expect(pastPreview({ scale: 0.3, x: 0, y: 0 }, chart, 1280)).toBe(false)
  expect(pastPreview({ scale: 0.5, x: 0, y: 0 }, chart, 1280)).toBe(true)
  expect(pastPreview({ scale: 4, x: 0, y: 0 }, chart, undefined)).toBe(false)
  expect(pastPreview({ scale: 4, x: 0, y: 0 }, chart, 3200)).toBe(false)
  expect(percent(0.3456)).toBe('35%')
})
