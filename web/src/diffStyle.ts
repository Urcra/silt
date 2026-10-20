/** Tailwind classes for one line of a unified diff. */
export function lineClass(line: string): string {
  if (line.startsWith('diff --git') || line.startsWith('rename ')) return 'font-semibold text-neutral-800 bg-neutral-100'
  if (line.startsWith('+++') || line.startsWith('---')) return 'text-neutral-500'
  if (line.startsWith('@@')) return 'text-sky-700'
  if (line.startsWith('+')) return 'bg-emerald-50 text-emerald-900'
  if (line.startsWith('-')) return 'bg-rose-50 text-rose-900'
  return 'text-neutral-700'
}
