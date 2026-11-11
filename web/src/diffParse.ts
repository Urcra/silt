import type { DiffStat } from './api'

/** A line of code in a hunk, without its `+`, `-` or space. */
export interface CodeLine {
  kind: 'context' | 'add' | 'del'
  text: string
  oldLine: number | null
  newLine: number | null
}

export interface Hunk {
  header: string
  lines: CodeLine[]
}

/** One file of a diff, by its new path (its old one when deleted). */
export interface DiffFile {
  path: string
  added: number
  removed: number
  hunks: Hunk[]
  /** `new`, `deleted` or `renamed from …`, when git says so. */
  note: string | null
  /** Git says the file is binary, so it has no lines to count. */
  binary: boolean
}

/** The most of a file's diff drawn unasked, in characters. A generated or
 * minified file can be megabytes on one line, and laying that out stalls the
 * page for minutes. */
export const LARGE_DIFF = 200_000

/** Past either, a diff is read a file at a time rather than whole: tens of
 * thousands of files or hundreds of megabytes, as a dependency directory
 * recorded before `.gitignore` named it, would stall the page for good. */
export const LARGE_TOTAL = 2_000_000
export const MANY_FILES = 300

/** Whether a diff of these files can be read whole. */
export function readWhole(stats: readonly DiffStat[]): boolean {
  return stats.length <= MANY_FILES && stats.reduce((total, stat) => total + stat.size, 0) <= LARGE_TOTAL
}

/** A size in bytes, as `12 KB` or `448 MB`. */
export function bytes(size: number): string {
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${Math.round(size / 1024).toLocaleString()} KB`
  return `${(size / (1024 * 1024)).toLocaleString(undefined, { maximumFractionDigits: size < 10 * 1024 * 1024 ? 1 : 0 })} MB`
}

/** How many characters a file's diff holds. */
export function diffSize(file: DiffFile): number {
  return file.hunks.reduce((total, hunk) => hunk.lines.reduce((sum, line) => sum + line.text.length, total), 0)
}

/** A git-style unified diff, file by file and hunk by hunk. */
export function parseFiles(text: string): DiffFile[] {
  const files: DiffFile[] = []
  let file: DiffFile | null = null
  let hunk: Hunk | null = null
  let old = 0
  let next = 0
  for (const line of text.replace(/\n$/, '').split('\n')) {
    if (line.startsWith('diff --git ')) {
      file = { path: line.slice(line.lastIndexOf(' b/') + 3), added: 0, removed: 0, hunks: [], note: null, binary: false }
      files.push(file)
      hunk = null
      continue
    }
    if (!file) continue
    if (line.startsWith('new file')) file.note = 'new'
    else if (line.startsWith('deleted file')) file.note = 'deleted'
    else if (line.startsWith('rename from ')) file.note = `renamed from ${line.slice('rename from '.length)}`
    else if (line.startsWith('Binary files ')) file.binary = true
    const header = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line)
    if (header) {
      old = Number(header[1])
      next = Number(header[2])
      hunk = { header: line, lines: [] }
      file.hunks.push(hunk)
      continue
    }
    if (!hunk || line.startsWith('\\')) continue
    if (line.startsWith('+')) {
      hunk.lines.push({ kind: 'add', text: line.slice(1), oldLine: null, newLine: next++ })
      file.added += 1
    } else if (line.startsWith('-')) {
      hunk.lines.push({ kind: 'del', text: line.slice(1), oldLine: old++, newLine: null })
      file.removed += 1
    } else {
      hunk.lines.push({ kind: 'context', text: line.slice(1), oldLine: old++, newLine: next++ })
    }
  }
  return files
}

/** A hunk side by side: removed lines on the left facing the lines that replaced them. */
export function splitRows(hunk: Hunk): { left: CodeLine | null; right: CodeLine | null }[] {
  const rows: { left: CodeLine | null; right: CodeLine | null }[] = []
  let removed: CodeLine[] = []
  let added: CodeLine[] = []
  const flush = () => {
    for (let index = 0; index < Math.max(removed.length, added.length); index++) {
      rows.push({ left: removed[index] ?? null, right: added[index] ?? null })
    }
    removed = []
    added = []
  }
  for (const line of hunk.lines) {
    if (line.kind === 'del') removed.push(line)
    else if (line.kind === 'add') added.push(line)
    else {
      flush()
      rows.push({ left: line, right: line })
    }
  }
  flush()
  return rows
}

/** One line of a unified diff, with its line number in the new file when it has one. */
export interface DiffLine {
  text: string
  /** The file it belongs to, by its new path (its old one when deleted). */
  path: string | null
  newLine: number | null
}

/** Reads a git-style unified diff into lines that know their file and new line number. */
export function parseDiff(text: string): DiffLine[] {
  const lines: DiffLine[] = []
  let path: string | null = null
  let next = 0
  for (const line of text.replace(/\n$/, '').split('\n')) {
    if (line.startsWith('diff --git ')) {
      path = line.slice(line.lastIndexOf(' b/') + 3)
      next = 0
      lines.push({ text: line, path, newLine: null })
      continue
    }
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line)
    if (hunk) {
      next = Number(hunk[1])
      lines.push({ text: line, path, newLine: null })
      continue
    }
    const inHunk = next > 0 && !line.startsWith('+++') && !line.startsWith('---')
    if (inHunk && (line.startsWith('+') || line.startsWith(' ') || line === '')) {
      lines.push({ text: line, path, newLine: next })
      next += 1
      continue
    }
    lines.push({ text: line, path, newLine: null })
  }
  return lines
}
