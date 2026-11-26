import { expect, test } from 'vitest'
import { decorated, inSeason } from './seasonal'

const day = (month: number, date: number) => new Date(2026, month - 1, date, 12)

test('the season runs from 1 December to 6 January', () => {
  expect(inSeason(day(11, 30))).toBe(false)
  expect(inSeason(day(12, 1))).toBe(true)
  expect(inSeason(day(12, 31))).toBe(true)
  expect(inSeason(day(1, 6))).toBe(true)
  expect(inSeason(day(1, 7))).toBe(false)
  expect(inSeason(day(7, 15))).toBe(false)
})

test('the decorations show only in the desktop app, on Auto only in season', () => {
  const auto = { show: 'auto', snow: true } as const
  expect(decorated(auto, day(12, 24), true)).toBe(true)
  expect(decorated(auto, day(12, 24), false)).toBe(false)
  expect(decorated(auto, day(3, 1), true)).toBe(false)
  expect(decorated({ show: 'always', snow: true }, day(3, 1), true)).toBe(true)
  expect(decorated({ show: 'always', snow: true }, day(3, 1), false)).toBe(false)
  expect(decorated({ show: 'off', snow: true }, day(12, 24), true)).toBe(false)
})
