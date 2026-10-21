// One keydown listener for the whole app, and the actions views offer the
// command palette while they're shown.

import { useEffect, useId, useRef, useSyncExternalStore } from 'react'

export interface Shortcut {
  /** Keys joined by `+`, such as `mod+k` or `n`; `mod` is ⌘ on a Mac and Ctrl elsewhere. */
  keys: string
  label: string
  run: () => void
}

export interface Action {
  id: string
  label: string
  /** Its shortcut, shown next to it. */
  keys?: string
  run: () => void
}

const shortcuts = new Map<string, Shortcut>()
const offered = new Map<string, Action[]>()
const heard = new Set<() => void>()
let snapshot: Action[] = []

function changed() {
  snapshot = [...offered.values()].flat()
  for (const listener of heard) listener()
}

const MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)

export function formatKeys(keys: string): string {
  return keys
    .split('+')
    .map((key) => {
      if (key === 'mod') return MAC ? '⌘' : 'Ctrl'
      if (key === 'shift') return MAC ? '⇧' : 'Shift'
      if (key === 'alt') return MAC ? '⌥' : 'Alt'
      return key.length === 1 ? key.toUpperCase() : key
    })
    .join(MAC ? '' : '+')
}

export function matches(event: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'shiftKey' | 'altKey'>, keys: string, mac = MAC) {
  const parts = keys.split('+')
  const key = parts[parts.length - 1]
  const mod = parts.includes('mod')
  return (
    event.key.toLowerCase() === key &&
    (mac ? event.metaKey : event.ctrlKey) === mod &&
    (mac ? !event.ctrlKey : !event.metaKey) &&
    event.shiftKey === parts.includes('shift') &&
    event.altKey === parts.includes('alt')
  )
}

/** Whether the person is writing, where a key without a modifier is theirs. */
function typing(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)
}

function onKey(event: KeyboardEvent) {
  for (const shortcut of shortcuts.values()) {
    if (!matches(event, shortcut.keys)) continue
    if (typing(event.target) && !/mod|alt/.test(shortcut.keys)) return
    event.preventDefault()
    shortcut.run()
    return
  }
}

function register(id: string, shortcut: Shortcut): () => void {
  if (shortcuts.size === 0) document.addEventListener('keydown', onKey)
  shortcuts.set(id, shortcut)
  return () => {
    shortcuts.delete(id)
    if (shortcuts.size === 0) document.removeEventListener('keydown', onKey)
  }
}

/** Runs `run` on `keys` while the calling component is mounted. */
export function useShortcut(keys: string, label: string, run: () => void) {
  const id = useId()
  const latest = useRef(run)
  useEffect(() => {
    latest.current = run
  })
  useEffect(() => register(id, { keys, label, run: () => latest.current() }), [id, keys, label])
}

/** Offers `actions` in the palette, with their shortcuts, while the calling component is mounted. */
export function useActions(actions: Action[]) {
  const id = useId()
  const latest = useRef(actions)
  useEffect(() => {
    latest.current = actions
  })
  const signature = actions.map((action) => `${action.id}:${action.label}:${action.keys ?? ''}`).join('|')
  useEffect(() => {
    offered.set(
      id,
      latest.current.map((action) => ({
        ...action,
        run: () => latest.current.find((current) => current.id === action.id)?.run(),
      })),
    )
    const unregister = latest.current
      .filter((action) => action.keys)
      .map((action) => register(`${id}:${action.id}`, { keys: action.keys!, label: action.label, run: () => latest.current.find((current) => current.id === action.id)?.run() }))
    changed()
    return () => {
      offered.delete(id)
      for (const off of unregister) off()
      changed()
    }
  }, [id, signature])
}

/** Every action offered now, as views come and go. */
export function useOfferedActions(): Action[] {
  return useSyncExternalStore(
    (listener) => {
      heard.add(listener)
      return () => heard.delete(listener)
    },
    () => snapshot,
  )
}
