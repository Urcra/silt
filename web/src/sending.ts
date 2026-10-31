import { useSyncExternalStore } from 'react'

/** A prompt the person sent that the thread's history doesn't have yet. */
export type Sending = {
  id: number
  text: string
  files: string[]
  /** How many entries the history had when it was sent: only a prompt after them is this one. */
  after: number
}

const NONE: Sending[] = []
// Outside React's state, as New thread sends a prompt to a thread whose page isn't drawn yet.
const sending = new Map<string, Sending[]>()
const listeners = new Set<() => void>()
let next = 0

function changed(thread: string, list: Sending[]) {
  if (list.length === 0) sending.delete(thread)
  else sending.set(thread, list)
  for (const listener of listeners) listener()
}

export function addSending(thread: string, text: string, files: string[], after: number): number {
  const id = ++next
  changed(thread, [...(sending.get(thread) ?? NONE), { id, text, files, after }])
  return id
}

export function dropSending(thread: string, ids: number[]) {
  const before = sending.get(thread) ?? NONE
  const after = before.filter((item) => !ids.includes(item.id))
  if (after.length !== before.length) changed(thread, after)
}

export function useSending(thread: string): Sending[] {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => sending.get(thread) ?? NONE,
  )
}

/**
 * Which of `sent` the history has taken in. Each takes a prompt of its own after
 * the entries it followed: its own words first, else any, as the agent may have
 * been told them otherwise (with the others' drafts, or a command expanded).
 */
export function arrived(sent: Sending[], entries: { kind: string; text: string }[]): number[] {
  const taken = new Set<number>()
  const ids = new Set<number>()
  const claim = (item: Sending, fits: (text: string) => boolean) => {
    const at = entries.findIndex((entry, index) => index >= item.after && !taken.has(index) && entry.kind === 'user' && fits(entry.text))
    if (at === -1) return
    taken.add(at)
    ids.add(item.id)
  }
  for (const item of sent) claim(item, (text) => text.includes(item.text.trim()))
  for (const item of sent) if (!ids.has(item.id)) claim(item, () => true)
  return sent.filter((item) => ids.has(item.id)).map((item) => item.id)
}
