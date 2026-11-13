import { type Dispatch, type SetStateAction, useCallback, useSyncExternalStore } from 'react'

/**
 * What's typed in each composer, by where it is, so leaving a screen and coming back finds the text as it was.
 * Kept for this tab, through a reload, but not shared with other tabs, which would each tell the thread's
 * viewers a different draft.
 */
const KEY = 'silt-drafts'

let drafts: Record<string, string> = read()
const listeners = new Set<() => void>()

function read(): Record<string, string> {
  try {
    return JSON.parse(sessionStorage.getItem(KEY) ?? '{}') as Record<string, string>
  } catch {
    return {}
  }
}

function write() {
  try {
    sessionStorage.setItem(KEY, JSON.stringify(drafts))
  } catch {
    // Kept for this page only, then.
  }
}

export const threadDraft = (thread: string) => `thread:${thread}`
export const NEW_THREAD_DRAFT = 'new-thread'
export const NEW_CHAT_DRAFT = 'new-chat'

export function draftOf(key: string): string {
  return drafts[key] ?? ''
}

export function keepDraft(key: string, text: string) {
  if (draftOf(key) === text) return
  drafts = { ...drafts, [key]: text }
  if (text === '') delete drafts[key]
  write()
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** `useState` for the draft at `key`, kept when the screen goes and given back when it comes again. */
export function useDraft(key: string): [string, Dispatch<SetStateAction<string>>] {
  const draft = useSyncExternalStore(subscribe, () => draftOf(key))
  const setDraft = useCallback<Dispatch<SetStateAction<string>>>(
    (next) => keepDraft(key, typeof next === 'function' ? next(draftOf(key)) : next),
    [key],
  )
  return [draft, setDraft]
}
