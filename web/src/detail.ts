import { createContext, useState } from 'react'

/** How much of each turn's work the transcript shows: the replies alone, the work folded, or everything open. */
export type Detail = 'compact' | 'normal' | 'verbose'

export const DETAILS: { id: Detail; label: string; says: string }[] = [
  { id: 'compact', label: 'Compact', says: "each turn's work folded under Worked for, its reply showing" },
  { id: 'normal', label: 'Normal', says: 'commands, questions and plans showing; the rest folded' },
  { id: 'verbose', label: 'Verbose', says: 'every fold open, thinking and tool output included' },
]

export const TranscriptDetail = createContext<Detail>('normal')

const KEY = 'silt.transcript-detail'

/** The detail the person chose, kept for this browser. */
export function useDetailChoice(): [Detail, (detail: Detail) => void] {
  const [detail, setDetail] = useState<Detail>(() => {
    try {
      const kept = localStorage.getItem(KEY)
      return DETAILS.some((choice) => choice.id === kept) ? (kept as Detail) : 'normal'
    } catch {
      return 'normal'
    }
  })
  const choose = (chosen: Detail) => {
    setDetail(chosen)
    try {
      localStorage.setItem(KEY, chosen)
    } catch {
      // Kept for this page only.
    }
  }
  return [detail, choose]
}

/** Drafts for a thread about to open, such as a prompt to edit in a fork made for it. */
const pending = new Map<string, string>()

export function leaveDraft(thread: string, draft: string) {
  pending.set(thread, draft)
}

export function takeDraft(thread: string): string | undefined {
  const draft = pending.get(thread)
  pending.delete(thread)
  return draft
}
