import { useSyncExternalStore } from 'react'
import { store, stored } from './stored'

// The sidebar's folds, kept in this browser so a section folded away stays so after a reload.

const KEY = 'silt.folds'
const listeners = new Set<() => void>()
let kept: Record<string, boolean> = stored(KEY, {})

function setOpen(fold: string, open: boolean) {
  kept = { ...kept, [fold]: open }
  store(KEY, kept)
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** Whether `fold` is open: `initially` until the person toggles it, then what they chose. */
export function useFold(fold: string, initially: boolean): [boolean, (open: boolean) => void] {
  const open = useSyncExternalStore(subscribe, () => kept[fold] ?? initially)
  return [open, (next) => setOpen(fold, next)]
}

/** Forgets every fold, for tests. */
export function resetFolds() {
  kept = {}
  store(KEY, kept)
  for (const listener of listeners) listener()
}
