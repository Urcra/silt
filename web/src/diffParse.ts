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
