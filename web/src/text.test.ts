import { expect, test } from 'vitest'
import { zoomLink } from './desktop'
import { RANGES, TEXT_DEFAULTS, changed, cleaned, effective, stepped } from './text'

test('what is left at its default is left out, so a later default applies', () => {
  expect(cleaned({ chat_size: 13, code_size: 14, density: 'regular', wrap: true, ui_font: null, detail: 'verbose' })).toEqual({
    code_size: 14,
    detail: 'verbose',
  })
  expect(changed({ chat_size: 13 }, 'chat_size')).toBe(false)
  expect(changed({ chat_size: 15 }, 'chat_size')).toBe(true)
})

test('a setting left out reads as its default', () => {
  const shown = effective({ chat_size: 16, diff_layout: null })
  expect(shown.chat_size).toBe(16)
  expect(shown.ui_size).toBe(TEXT_DEFAULTS.ui_size)
  expect(shown.diff_layout).toBeNull()
  expect(shown.motion).toBe('system')
})

test('sizes step within their range, as silt-theme clamps them', () => {
  expect(stepped(RANGES.chat_size, 13, 1)).toBe(14)
  expect(stepped(RANGES.chat_size, 22, 1)).toBe(22)
  expect(stepped(RANGES.code_size, 10, -1)).toBe(10)
  expect(stepped(RANGES.zoom, 100, 2)).toBe(120)
  expect(stepped(RANGES.zoom, 50, -1)).toBe(50)
})

test('the link that zooms the desktop app names the level alone', () => {
  expect(zoomLink(120)).toBe('silt://zoom/?percent=120')
})

const SOURCES = import.meta.glob<string>(['./**/*.ts', './**/*.tsx', '!./**/*.test.*', '!./wasm/**'], {
  query: '?raw',
  import: 'default',
  eager: true,
})

test('text sizes come from the scale, so text and layout reach them', () => {
  // A whiteboard's notes are content, drawn at the board's own zoom.
  const exempt = new Set(['./components/Whiteboard.tsx'])
  const literal = /\btext-\[\d+(\.\d+)?px\]/
  expect(Object.keys(SOURCES).length).toBeGreaterThan(100)
  const found = Object.entries(SOURCES)
    .filter(([path]) => !exempt.has(path))
    .flatMap(([path, source]) =>
      source.split('\n').flatMap((line, index) => (literal.test(line) ? [`${path}:${index + 1}`] : [])),
    )
  expect(found).toEqual([])
})
