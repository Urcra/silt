/** Tailwind classes for one line of a unified diff. */
export function lineClass(line: string): string {
  if (line.startsWith('diff --git') || line.startsWith('rename ')) return 'font-semibold text-fg-1 bg-surface-1'
  if (line.startsWith('+++') || line.startsWith('---')) return 'text-fg-3'
  if (line.startsWith('@@')) return 'text-info'
  if (line.startsWith('+')) return 'bg-ok-tint text-fg-1'
  if (line.startsWith('-')) return 'bg-danger-tint text-fg-1'
  return 'text-fg-2'
}
