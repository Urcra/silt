import { useCallback, useState } from 'react'
import type { CommentView } from './api'
import type { DiffFile } from './diffParse'
import { store, stored } from './stored'

/** Which side of a diff lines are on: the thread's (`new`) or the base's, for lines it deleted (`old`). */
export type Side = 'old' | 'new'

/** Where a line comment sits, as the daemon anchors it. */
export interface Lines {
  path: string
  start: number
  end: number
  side?: Side
}

export function linesOf(comment: CommentView): Lines | null {
  if (comment.location.at !== 'lines') return null
  const { path, start, end, outdated, before } = comment.location
  // Deleted lines sit where they were in the base, when it has them.
  return outdated && before ? { path, start: before.start, end: before.end, side: 'old' } : { path, start, end, side: 'new' }
}

export function sideOf(at: Lines): Side {
  return at.side ?? 'new'
}

/** `path:3` or `path:3-5`. */
export function spanOf(at: Lines): string {
  return `${at.path}:${at.start}${at.end !== at.start ? `-${at.end}` : ''}`
}

/** A comment waiting in a review until it's sent, kept in this browser (ADR 0055). */
export interface Draft {
  id: string
  at: Lines
  text: string
  /** The commented lines as the diff showed them, each led by its `+`, `-` or space. */
  quote: string[]
  /** The diff base deleted lines are numbered in. */
  base?: string
}

export interface Review {
  note: string
  drafts: Draft[]
}

const EMPTY: Review = { note: '', drafts: [] }

/** The review in progress on `thread`, kept in this browser until it's sent. */
export function useReview(thread: string) {
  const key = `silt-review-${thread}`
  const [kept, setKept] = useState(() => ({ key, review: stored<Review>(key, EMPTY) }))
  // Another thread's review is read afresh rather than shown for this one.
  const review = kept.key === key ? kept.review : stored<Review>(key, EMPTY)
  const update = useCallback(
    (change: (review: Review) => Review) =>
      setKept((before) => {
        const next = change(before.key === key ? before.review : stored<Review>(key, EMPTY))
        store(key, next)
        return { key, review: next }
      }),
    [key],
  )
  return {
    review,
    add: (at: Lines, text: string, quote: string[], base?: string) =>
      update((before) => ({
        ...before,
        drafts: [...before.drafts, { id: `${Date.now()}-${before.drafts.length}`, at, text, quote, base }],
      })),
    edit: (id: string, text: string) =>
      update((before) => ({ ...before, drafts: before.drafts.map((draft) => (draft.id === id ? { ...draft, text } : draft)) })),
    remove: (id: string) => update((before) => ({ ...before, drafts: before.drafts.filter((draft) => draft.id !== id) })),
    setNote: (note: string) => update((before) => ({ ...before, note })),
    clear: () => update(() => EMPTY),
  }
}

export type ReviewState = ReturnType<typeof useReview>

/** The lines `at` names in a diff, each led by its sign, as a review quotes them. */
export function quoteOf(files: DiffFile[], at: Lines): string[] {
  const side = sideOf(at)
  const file = files.find((candidate) => candidate.path === at.path)
  if (!file) return []
  return file.hunks.flatMap((hunk) =>
    hunk.lines
      .filter((line) => {
        const number = side === 'old' ? (line.kind === 'add' ? null : line.oldLine) : line.newLine
        return number !== null && number >= at.start && number <= at.end
      })
      .map((line) => `${line.kind === 'add' ? '+' : line.kind === 'del' ? '-' : ' '}${line.text}`),
  )
}

/** One comment as a review sends it. */
export interface ReviewItem {
  at: Lines
  text: string
  by?: string
  quote?: string[]
}

export function itemOf(comment: CommentView, files: DiffFile[] = []): ReviewItem | null {
  const at = linesOf(comment)
  return at && { at, text: comment.text, by: comment.by, quote: quoteOf(files, at) }
}

/** Lines of code a review quotes at most per comment. */
const QUOTED = 30

/** The prompt asking the agent to address a review: its note, then each comment under the lines it's on. */
export function reviewPrompt(items: ReviewItem[], note = ''): string {
  const comments = items.map((item) => {
    const by = item.by ? ` (${item.by})` : ''
    const deleted = sideOf(item.at) === 'old' ? ', lines the change deleted' : ''
    const head = `${spanOf(item.at)}${deleted}${by}`
    const quote = item.quote ?? []
    if (quote.length === 0) return `- ${head}: ${item.text}`
    const shown = quote.length > QUOTED ? [...quote.slice(0, QUOTED), ' …'] : quote
    const longest = Math.max(2, ...shown.flatMap((line) => (line.match(/`+/g) ?? []).map((run) => run.length)))
    const fence = '`'.repeat(longest + 1)
    return `- ${head}: ${item.text}\n${fence}diff\n${shown.join('\n')}\n${fence}`
  })
  const intro = note.trim() ? `${note.trim()}\n\n` : ''
  return `${intro}Please address these review comments:\n${comments.join('\n')}`
}
