import { expect, test } from 'vitest'
import { diffFile, filePath, linesHash, parseLines, permalink } from './permalink'

test('a line hash reads back as it was written, in order', () => {
  expect(parseLines('#L4')).toEqual({ start: 4, end: 4 })
  expect(parseLines('L9-L4')).toEqual({ start: 4, end: 9 })
  expect(parseLines('#turn-3')).toBeNull()
  expect(parseLines('#L0')).toBeNull()
  expect(linesHash({ start: 4, end: 9 })).toBe('L4-L9')
  expect(linesHash({ start: 4, end: 4 })).toBe('L4')
})

test('a file path keeps its directories and pins the version', () => {
  expect(filePath('abc', 'src/a b.rs', 'f.1,e.2', { start: 2, end: 3 })).toBe('/t/abc/files/src/a%20b.rs?at=f.1%2Ce.2#L2-L3')
  expect(filePath('abc', 'main.rs')).toBe('/t/abc/files/main.rs')
})

test("a link from a machine on a server opens in the server's app", () => {
  expect(permalink('/t/abc#turn-2', 'https://silt.example.com')).toBe('https://silt.example.com/app/t/abc#turn-2')
  expect(permalink('/t/abc', null)).toBe(new URL('t/abc', document.baseURI).toString())
})

test('a diff names the file each header starts', () => {
  expect(diffFile('diff --git a/src/old.rs b/src/new.rs')).toBe('src/new.rs')
  expect(diffFile('+diff --git')).toBeNull()
})
