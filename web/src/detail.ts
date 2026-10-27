import { createContext, useState } from 'react'

/** How much each step of the agent's work shows: closed, or every fold open. */
export type Detail = 'normal' | 'verbose'

export const DETAILS: { id: Detail; label: string; says: string }[] = [
  { id: 'normal', label: 'Normal', says: 'each run of work and each step closed, failures and questions showing' },
  { id: 'verbose', label: 'Verbose', says: 'every fold open, thinking and tool output included' },
]

/** How the agent's work is grouped (ADR 0049): a finished turn folded under "Worked for", or each run between its words. */
export type Grouping = 'turn' | 'run'

export const GROUPINGS: { id: Grouping; label: string; says: string }[] = [
  { id: 'turn', label: 'By turn', says: "a finished turn's work folds under Worked for, its reply showing" },
  { id: 'run', label: 'By run', says: "each run of work between the agent's words folds on its own" },
]

export const TranscriptDetail = createContext<Detail>('normal')

export const TranscriptGrouping = createContext<Grouping>('turn')

/** Every thought open or closed at once (T), and how many times that was asked, so open folds follow it. */
export const ThoughtsOpen = createContext<{ open: boolean; times: number }>({ open: false, times: 0 })

/** What the running tool printed so far, by its call's id, for its step to show. */
export const LiveTool = createContext<{ id: string | null; text: string }>({ id: null, text: '' })

const KEY = 'silt.transcript-detail'
const GROUPING_KEY = 'silt.transcript-grouping'

/** A choice kept for this browser, or `fallback`. */
function useKept<T extends string>(key: string, choices: { id: T }[], fallback: T): [T, (chosen: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const kept = localStorage.getItem(key)
      return choices.some((choice) => choice.id === kept) ? (kept as T) : fallback
    } catch {
      return fallback
    }
  })
  const choose = (chosen: T) => {
    setValue(chosen)
    try {
      localStorage.setItem(key, chosen)
    } catch {
      // Kept for this page only.
    }
  }
  return [value, choose]
}

/** The detail the person chose, kept for this browser. Compact, which folded each turn, is now grouping by turn. */
export function useDetailChoice(): [Detail, (detail: Detail) => void] {
  return useKept(KEY, DETAILS, 'normal')
}

/** The grouping the person chose, kept for this browser; by turn unless they chose otherwise. */
export function useGroupingChoice(): [Grouping, (grouping: Grouping) => void] {
  return useKept(GROUPING_KEY, GROUPINGS, 'turn')
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
