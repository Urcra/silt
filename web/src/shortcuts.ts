// One keydown listener for the whole app, and the actions views offer the
// command palette while they're shown.

import { useEffect, useId, useRef, useSyncExternalStore } from 'react'

export interface Shortcut {
  /** Keys joined by `+`, such as `mod+k` or `n`; `mod` is ⌘ on a Mac and Ctrl elsewhere. */
  keys: string
  label: string
  run: () => void
  /** Wins over shortcuts without it while registered, as a view's own keys win over the app's. */
  over?: boolean
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

export function matches(
  event: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'shiftKey' | 'altKey'>,
  keys: string,
  mac = MAC,
) {
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

function inDialog(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest('dialog[open], [role="dialog"]') !== null
}

type Pressed = Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'shiftKey' | 'altKey'>

/** The shortcut a key press runs: one registered `over` first, then the rest in the order they came; none while the
 * person types or is in a dialog, unless it takes a modifier. */
export function pick(list: Iterable<Shortcut>, event: Pressed, writing: boolean, mac = MAC): Shortcut | undefined {
  const all = [...list]
  const found = [...all.filter((shortcut) => shortcut.over), ...all.filter((shortcut) => !shortcut.over)].find((shortcut) =>
    matches(event, shortcut.keys, mac),
  )
  if (!found || (writing && !/mod|alt/.test(found.keys))) return undefined
  return found
}

function onKey(event: KeyboardEvent) {
  // An overlay that took the key, such as a menu closing on Escape, keeps it.
  if (event.defaultPrevented) return
  const found = pick(shortcuts.values(), event, typing(event.target) || inDialog(event.target))
  if (!found) return
  event.preventDefault()
  found.run()
}

function register(id: string, shortcut: Shortcut): () => void {
  if (shortcuts.size === 0) document.addEventListener('keydown', onKey)
  shortcuts.set(id, shortcut)
  return () => {
    shortcuts.delete(id)
    if (shortcuts.size === 0) document.removeEventListener('keydown', onKey)
  }
}

/** How a view's keys stand against the app's. */
export interface Scope {
  /** Over the app's own keys while mounted, as the review's j, k and n are. */
  over?: boolean
}

/** Runs `run` on `keys` while the calling component is mounted. */
export function useShortcut(keys: string, label: string, run: () => void, { over = false }: Scope = {}) {
  const id = useId()
  const latest = useRef(run)
  useEffect(() => {
    latest.current = run
  })
  useEffect(() => register(id, { keys, label, over, run: () => latest.current() }), [id, keys, label, over])
}

/** Offers `actions` in the palette, with their shortcuts, while the calling component is mounted. */
export function useActions(actions: Action[], { over = false }: Scope = {}) {
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
      .map((action) =>
        register(`${id}:${action.id}`, {
          keys: action.keys!,
          label: action.label,
          over,
          run: () => latest.current.find((current) => current.id === action.id)?.run(),
        }),
      )
    changed()
    return () => {
      offered.delete(id)
      for (const off of unregister) off()
      changed()
    }
  }, [id, signature, over])
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
