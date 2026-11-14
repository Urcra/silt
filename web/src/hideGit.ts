import { useSyncExternalStore } from 'react'
import { store, stored } from './stored'

/** Whether diffs leave out what git itself rewrote in a tool call, as a checkout, pull or stash does (ADR 0066). Off until the person ticks it, then kept in this browser. */
const KEY = 'silt.hide-git'

const listeners = new Set<() => void>()
let hidden = stored(KEY, false)

export function setHideGit(on: boolean) {
  hidden = on
  store(KEY, on)
  for (const listener of listeners) listener()
}

/** The setting, shared by every diff on the page. */
export function useHideGit(): [boolean, (on: boolean) => void] {
  const on = useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => hidden,
  )
  return [on, setHideGit]
}
