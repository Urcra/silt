import { expect, test } from 'vitest'
import { ago } from './time'

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
