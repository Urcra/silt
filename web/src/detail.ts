import { createContext } from 'react'
import { message } from './query'
import { useTextSettings } from './text'
import { toast } from './ui'

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

/** What the agent is thinking now, not recorded yet, for the running turn's card to show where it will land. */
export const LiveThought = createContext('')

/** The detail the person chose, kept for this machine with text and layout (ADR 0107). */
export function useDetailChoice(): [Detail, (detail: Detail) => void] {
  const { shown, set } = useTextSettings()
  return [shown.detail, (detail) => void set('detail', detail).catch((failure: unknown) => toast(message(failure), 'danger'))]
}

/** The grouping the person chose, kept for this machine; by turn unless they chose otherwise. */
export function useGroupingChoice(): [Grouping, (grouping: Grouping) => void] {
  const { shown, set } = useTextSettings()
  return [
    shown.grouping,
    (grouping) => void set('grouping', grouping).catch((failure: unknown) => toast(message(failure), 'danger')),
  ]
}
