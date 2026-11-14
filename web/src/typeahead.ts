import { type KeyboardEvent, useRef } from 'react'

/**
 * The option typing `typed` moves to from `current`, as a listbox's type-ahead does: one letter, typed
 * again and again, steps through the names it starts; several letters find the first name they start.
 */
export function typeahead(names: string[], current: number, typed: string): number | undefined {
  const wanted = typed.toLowerCase()
  if (!wanted || names.length === 0) return undefined
  const repeated = [...wanted].every((char) => char === wanted[0])
  const prefix = repeated ? wanted[0]! : wanted
  const from = repeated ? current + 1 : Math.max(current, 0)
  for (let step = 0; step < names.length; step++) {
    const at = (from + step) % names.length
    if (names[at]!.toLowerCase().startsWith(prefix)) return at
  }
  return undefined
}

/** Letters typed within this of each other make one search. */
const TYPED_TOGETHER_MS = 700

/**
 * A keydown handler for a box that moves focus among its `options` by type-ahead and the arrow keys.
 * It keeps the keys it takes from the app's own shortcuts, such as `n` for a new thread.
 */
export function useTypeahead(options: (box: HTMLElement) => HTMLElement[], names: string[]) {
  const typed = useRef({ text: '', at: 0 })
  return (event: KeyboardEvent<HTMLElement>) => {
    if (event.ctrlKey || event.metaKey || event.altKey) return
    const target = event.target as HTMLElement
    if (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)) return
    const all = options(event.currentTarget)
    const current = all.findIndex((option) => option === target)
    const step = event.key === 'ArrowDown' ? 1 : event.key === 'ArrowUp' ? -1 : 0
    if (step !== 0) {
      if (current < 0 || all.length === 0) return
      event.preventDefault()
      all[(current + step + all.length) % all.length]!.focus()
      return
    }
    if (event.key.length !== 1 || event.key === ' ') return
    const now = Date.now()
    typed.current = { text: (now - typed.current.at < TYPED_TOGETHER_MS ? typed.current.text : '') + event.key, at: now }
    event.preventDefault()
    event.stopPropagation()
    const next = typeahead(names, current, typed.current.text)
    if (next !== undefined) all[next]?.focus()
  }
}
