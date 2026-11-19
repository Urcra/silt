import { expect, test } from 'vitest'
import { ago, resetsAt } from './time'

test('times read as dense lists say them', () => {
  const now = Date.UTC(2026, 9, 22, 12)
  expect(ago(now - 20_000, now)).toBe('just now')
  expect(ago(now - 4 * 60_000, now)).toBe('4m')
  expect(ago(now - 2 * 3_600_000, now)).toBe('2h')
  expect(ago(now - 30 * 3_600_000, now)).toBe('yesterday')
  expect(ago(now - 3 * 86_400_000, now)).toBe('3d')
  expect(ago(now - 15 * 86_400_000, now)).toBe('2w')
  expect(ago(now + 5_000, now)).toBe('just now')
})

test("a limit's reset reads as a time today, a weekday this week, else a date", () => {
  // Local times, as the person reads them.
  const now = new Date(2026, 10, 17, 9).getTime()
  const time = (at: Date) => at.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
  const today = new Date(2026, 10, 17, 14)
  expect(resetsAt(today.getTime(), now)).toBe(`Resets at ${time(today)}`)
  const later = new Date(2026, 10, 18, 7)
  expect(resetsAt(later.getTime(), now)).toBe(
    `Resets ${later.toLocaleDateString(undefined, { weekday: 'short' })} ${time(later)}`,
  )
  const month = new Date(2026, 11, 1)
  expect(resetsAt(month.getTime(), now)).toBe(`Resets ${month.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}`)
})
