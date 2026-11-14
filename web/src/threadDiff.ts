import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useMemo, useRef } from 'react'
import { api, type DiffSpec } from './api'
import { parseFiles, readWhole } from './diffParse'
import { commentBaseOf } from './diffSpec'
import { keys } from './query'
import { type Draft, type Lines, type ReviewState, linesOf, quoteOf, shownAt, sideOf } from './review'
import type { Commenting } from './components/DiffView'

/** Files in a page of a thread's diff: a change touching thousands comes a page at a time. */
export const FILES_PAGE = 50

/** How many files a page of a diff holds. */
function filesIn(diff: string): number {
  return diff.split('\n').filter((line) => line.startsWith('diff --git ')).length
}

/** A thread's diff against `base`, with what commenting on it takes: its files first, then its text, read whole or a
 * file at a time when too large; the comments on it; and `review`, as this diff shows it. */
export function useThreadDiff(thread: string, spec: DiffSpec, review: ReviewState, onCommented: () => Promise<unknown>, agent?: string) {
  const queries = useQueryClient()
  // A range of turns starts at a prompt, which isn't a version comments on deleted lines are numbered in.
  const commentBase = spec.turn === undefined ? commentBaseOf(spec.base ?? '') : undefined
  const stats = useQuery({ queryKey: keys.diffStats(thread, spec), queryFn: () => api.diffStats(thread, spec) })
  const whole = stats.data === undefined ? undefined : readWhole(stats.data)
  const diff = useInfiniteQuery({
    queryKey: keys.diffPages(thread, spec),
    queryFn: ({ pageParam }) => api.diff(thread, { ...spec, offset: pageParam, limit: FILES_PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last, pages) => (filesIn(last) < FILES_PAGE ? undefined : pages.length * FILES_PAGE),
    enabled: whole === true,
  })
  const comments = useQuery({ queryKey: [...keys.comments(thread), commentBase ?? ''], queryFn: () => api.comments(thread, commentBase) })
  const changed = () => Promise.all([queries.invalidateQueries({ queryKey: keys.comments(thread) }), onCommented()])
  const text = diff.data?.pages.join('')
  const files = useMemo(() => (text === undefined ? [] : parseFiles(text)), [text])
  // Pending comments follow their text (ADR 0066): each takes an anchor when written, and shows where it is now.
  const tried = useRef(new Set<string>())
  const anchor = (id: string, at: Lines, base: string | undefined) => {
    tried.current.add(id)
    void api
      .anchorLines(thread, { on: 'lines', path: at.path, start: at.start, end: at.end, base })
      .then(({ anchor: taken }) => review.setAnchor(id, taken))
      .catch(() => undefined)
  }
  // Drafts from before anchors take one once, where their lines are now.
  const drafts = review.review.drafts
  useEffect(() => {
    for (const draft of drafts) if (!draft.anchor && !tried.current.has(draft.id)) anchor(draft.id, draft.at, draft.base)
  })
  const anchored = drafts.filter((draft) => draft.anchor !== undefined)
  const anchors = anchored.map((draft) => draft.anchor ?? '')
  const placed = useQuery({
    queryKey: [...keys.comments(thread), 'pending', commentBase ?? '', anchors],
    queryFn: () => api.placeAnchors(thread, { base: commentBase ?? null, anchors }),
    enabled: anchors.length > 0,
  })
  const shown = drafts.map((draft) => ({ draft, at: shownAt(draft, placed.data?.[anchored.indexOf(draft)]) }))
  const moved = shown.flatMap(({ draft, at }): Draft[] => {
    if (at === 'outdated') return []
    const quote = quoteOf(files, at)
    return [{ ...draft, at, quote: quote.length > 0 ? quote : draft.quote }]
  })
  // Gone from the thread and the base alike: kept beside the diff, and sent as lines since deleted.
  const outdated = shown.flatMap(({ draft, at }) => (at === 'outdated' ? [draft] : []))
  // The review as this diff shows it: a draft on deleted lines is numbered in the base it was made against.
  const here: ReviewState = {
    ...review,
    review: { ...review.review, drafts: moved.filter((draft) => sideOf(draft.at) === 'new' || draft.base === commentBase || draft.anchor !== undefined) },
    add: (at, text, quote) => {
      const base = sideOf(at) === 'old' ? commentBase : undefined
      const id = review.add(at, text, quote, base)
      anchor(id, at, base)
      return id
    },
  }
  // What finishing sends: every draft, where its lines are now, or as lines deleted since.
  const pending: ReviewState = {
    ...review,
    review: { ...review.review, drafts: [...moved, ...outdated.map((draft) => ({ ...draft, at: { ...draft.at, side: 'old' as const } }))] },
  }
  const commenting: Commenting = {
    thread,
    comments: comments.data ?? [],
    agent,
    deleted: commentBase !== undefined,
    review: here,
    onChanged: changed,
    onComment: async (at, comment) => {
      const on = sideOf(at) === 'old' ? commentBase : undefined
      await api.addComment(thread, { on: 'lines', path: at.path, start: at.start, end: at.end, base: on }, comment)
      await changed()
    },
  }
  const unsent = (comments.data ?? []).filter((comment) => !comment.resolved && !comment.sent && linesOf(comment) !== null)
  return { stats, whole, diff, text, files, commenting, unsent, changed, pending, outdated, error: stats.error ?? diff.error }
}
