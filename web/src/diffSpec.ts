/** What a thread's changes are compared with, as the compare menu names it. */
export type View = 'last-turn' | 'thread' | 'main' | 'other'

export const VIEWS: { id: View; label: string }[] = [
  { id: 'last-turn', label: 'Last turn' },
  { id: 'thread', label: 'This thread' },
  { id: 'main', label: 'Against main' },
  { id: 'other', label: 'Other…' },
]

/** The diff base the daemon takes for a view: '' for the whole thread; `other` is a base from Other…, `commit` its sha. */
export function baseOf(view: View, other: string, commit: string): string {
  if (view === 'thread') return ''
  if (view === 'other') return other === 'commit' ? (commit ? `commit:${commit}` : '') : other
  return view
}

/** The version a diff's deleted lines are numbered in, for comments on them; none for a git base, which isn't a
 * version of the thread. */
export function commentBaseOf(base: string): string | undefined {
  return base === '' ? 'start' : base === 'main' ? undefined : base
}
