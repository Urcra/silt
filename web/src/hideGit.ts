import { useSyncExternalStore } from 'react'
import { store, stored } from './stored'

/** A diff filter: off until the person ticks it, then kept in this browser and shared by every diff on the page. */
function filter(key: string) {
  const listeners = new Set<() => void>()
  let on = stored(key, false)
  return {
    set: (next: boolean) => {
      on = next
      store(key, next)
      for (const listener of listeners) listener()
    },
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    get: () => on,
  }
}

function useFilter(kept: ReturnType<typeof filter>): [boolean, (on: boolean) => void] {
  return [useSyncExternalStore(kept.subscribe, kept.get), kept.set]
}

/** Whether diffs leave out what git itself rewrote in a tool call, as a checkout, pull or stash does (ADR 0069). */
const git = filter('silt.hide-git')
/** Whether diffs leave out the edits of other agents working in the same clone (ADR 0089). */
const others = filter('silt.hide-others')

export const setHideGit = git.set
export const setHideOthers = others.set
export const useHideGit = () => useFilter(git)
export const useHideOthers = () => useFilter(others)
