// The lines two texts share and where they differ, for comparing a doc's versions (ADR 0077).

export type DiffLine = { kind: 'same' | 'added' | 'removed'; text: string }

/** Past this many lines on both sides together, the texts are compared as wholes. */
const MAX_LINES = 4000

/** `before` turned into `after`, line by line, by their longest common subsequence. */
export function lineDiff(before: string, after: string): DiffLine[] | null {
  const old = before.split('\n')
  const now = after.split('\n')
  // The lines both start and end with take no table.
  let start = 0
  while (start < old.length && start < now.length && old[start] === now[start]) start++
  let end = 0
  while (end < old.length - start && end < now.length - start && old[old.length - 1 - end] === now[now.length - 1 - end]) end++
  const a = old.slice(start, old.length - end)
  const b = now.slice(start, now.length - end)
  if (a.length + b.length > MAX_LINES) return null
  const longest: Uint32Array[] = Array.from({ length: a.length + 1 }, () => new Uint32Array(b.length + 1))
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      longest[i][j] = a[i] === b[j] ? longest[i + 1][j + 1] + 1 : Math.max(longest[i + 1][j], longest[i][j + 1])
  const middle: DiffLine[] = []
  let i = 0
  let j = 0
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      middle.push({ kind: 'same', text: a[i] })
      i++
      j++
    } else if (longest[i + 1][j] >= longest[i][j + 1]) middle.push({ kind: 'removed', text: a[i++] })
    else middle.push({ kind: 'added', text: b[j++] })
  }
  while (i < a.length) middle.push({ kind: 'removed', text: a[i++] })
  while (j < b.length) middle.push({ kind: 'added', text: b[j++] })
  const same = (text: string): DiffLine => ({ kind: 'same', text })
  return [...old.slice(0, start).map(same), ...middle, ...old.slice(old.length - end).map(same)]
}
