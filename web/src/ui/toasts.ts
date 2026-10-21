import type { Tone } from './basics'

export interface Toast {
  id: number
  text: string
  tone: Tone
}

const TOAST_MS = 5000
let toasts: Toast[] = []
let nextToast = 0
const heard = new Set<() => void>()

export function setToasts(next: Toast[]) {
  toasts = next
  for (const listener of heard) listener()
}

export function currentToasts(): Toast[] {
  return toasts
}

export function hearToasts(listener: () => void): () => void {
  heard.add(listener)
  return () => heard.delete(listener)
}

/** Says something briefly, such as a failure nobody waits on. */
export function toast(text: string, tone: Tone = 'neutral') {
  const id = nextToast++
  setToasts([...toasts, { id, text, tone }])
  setTimeout(() => setToasts(toasts.filter((shown) => shown.id !== id)), TOAST_MS)
}
