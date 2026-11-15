// Moving through a review's diff by key: a file at a time, or a hunk at a time across files.

import type { FileSeen, TurnRun } from './api'
import type { DiffFile, Hunk } from './diffParse'
import type { Lines } from './review'

/** Where the review's cursor is: a file of the diff, and a hunk of it. */
export interface At {
  file: number
  hunk: number
}

/** The next file's first hunk (`by` 1) or the previous file's, or none past either end. */
export function stepFile(hunks: number[], at: At, by: 1 | -1): At | null {
  const file = at.file + by
  return file < 0 || file >= hunks.length ? null : { file, hunk: 0 }
}

/** The next hunk or the previous one, across files, skipping files without any; none past either end. */
export function stepHunk(hunks: number[], at: At, by: 1 | -1): At | null {
  const hunk = at.hunk + by
  if (hunk >= 0 && hunk < (hunks[at.file] ?? 0)) return { file: at.file, hunk }
  for (let file = at.file + by; file >= 0 && file < hunks.length; file += by) {
    if (hunks[file] > 0) return { file, hunk: by === 1 ? 0 : hunks[file] - 1 }
  }
  return null
}

/** Files whose path holds `filter`, ignoring case; all of them for an empty one. */
export function filtered<T extends { path: string }>(files: T[], filter: string): T[] {
  const wanted = filter.trim().toLowerCase()
  return wanted ? files.filter((file) => file.path.toLowerCase().includes(wanted)) : files
}

/** The first line a hunk changed, to comment on by key: an added line, or a deleted one where those take comments. */
export function firstChange(file: DiffFile, hunk: number, deleted: boolean): Lines | null {
  for (const line of file.hunks[hunk]?.lines ?? []) {
    if (line.kind === 'add' && line.newLine !== null) return { path: file.path, start: line.newLine, end: line.newLine, side: 'new' }
    if (deleted && line.kind === 'del' && line.oldLine !== null) return { path: file.path, start: line.oldLine, end: line.oldLine, side: 'old' }
  }
  return null
}

/** The turns that wrote what a hunk added, in order; a hunk that only deleted is the range's when that's one turn. */
export function turnsOfHunk(hunk: Hunk, runs: TurnRun[], only?: number): number[] {
  const turns = new Set<number>()
  for (const line of hunk.lines) {
    if (line.kind !== 'add' || line.newLine === null) continue
    const number = line.newLine
    const run = runs.find((candidate) => candidate.start <= number && number <= candidate.end)
    if (run) turns.add(run.turn)
  }
  if (turns.size === 0 && only !== undefined && hunk.lines.some((line) => line.kind === 'del')) turns.add(only)
  return [...turns].sort((a, b) => a - b)
}

/** The turns picked, from a click on `turn`: just it, or with shift held, the range from the first picked to it. */
export function pickTurns(picked: [number, number] | undefined, turn: number, extend: boolean): [number, number] {
  if (!extend || !picked) return [turn, turn]
  return [Math.min(picked[0], turn), Math.max(picked[0], turn)]
}

/** A file's seen state from the marks the person made: as seen, changed since, or not marked. */
export function seenOf(path: string, marks: ReadonlyMap<string, FileSeen>): 'yes' | 'no' | 'changed' {
  const mark = marks.get(path)
  return mark === undefined ? 'no' : mark.seen === 'seen' ? 'yes' : 'changed'
}

/** How many lines are new since the person saw a file. */
export function linesNew(mark: FileSeen | undefined): number {
  return (mark?.new_lines ?? []).reduce((sum, span) => sum + span.end - span.start + 1, 0)
}

/** The next file after `at` not seen as it is now, wrapping round; none when every one is. */
export function nextUnseen(paths: string[], marks: ReadonlyMap<string, FileSeen>, at: number): number | null {
  for (let step = 1; step <= paths.length; step += 1) {
    const index = (at + step) % paths.length
    if (seenOf(paths[index], marks) !== 'yes') return index
  }
  return null
}

/** The turns a review starts on: those since the one the person reviewed through, when there are any. */
export function defaultTurns(count: number, reviewed: number | null): [number, number] | undefined {
  return reviewed !== null && reviewed < count ? [reviewed + 1, count] : undefined
}
