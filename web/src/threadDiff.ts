import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query'
import { useMemo } from 'react'
import { api } from './api'
import { parseFiles, readWhole } from './diffParse'
import { commentBaseOf } from './diffSpec'
import { keys } from './query'
import { type ReviewState, linesOf, sideOf } from './review'
import type { Commenting } from './components/DiffView'

/** Files in a page of a thread's diff: a change touching thousands comes a page at a time. */
export const FILES_PAGE = 50

/** How many files a page of a diff holds. */
function filesIn(diff: string): number {
  return diff.split('\n').filter((line) => line.startsWith('diff --git ')).length
}

/** A thread's diff against `base`, with what commenting on it takes: its files first, then its text, read whole or a
 * file at a time when too large; the comments on it; and `review`, as this diff shows it. */
export function useThreadDiff(thread: string, base: string, review: ReviewState, onCommented: () => Promise<unknown>, agent?: string) {
  const queries = useQueryClient()
  const commentBase = commentBaseOf(base)
  const stats = useQuery({ queryKey: keys.diffStats(thread, { base }), queryFn: () => api.diffStats(thread, { base }) })
  const whole = stats.data === undefined ? undefined : readWhole(stats.data)
  const diff = useInfiniteQuery({
    queryKey: keys.diffPages(thread, base),
    queryFn: ({ pageParam }) => api.diff(thread, { base, offset: pageParam, limit: FILES_PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last, pages) => (filesIn(last) < FILES_PAGE ? undefined : pages.length * FILES_PAGE),
    enabled: whole === true,
  })
  const comments = useQuery({ queryKey: [...keys.comments(thread), commentBase ?? ''], queryFn: () => api.comments(thread, commentBase) })
  const changed = () => Promise.all([queries.invalidateQueries({ queryKey: keys.comments(thread) }), onCommented()])
  // The review as this diff shows it: a draft on deleted lines is numbered in the base it was made against.
  const here: ReviewState = {
    ...review,
    review: { ...review.review, drafts: review.review.drafts.filter((draft) => sideOf(draft.at) === 'new' || draft.base === commentBase) },
    add: (at, text, quote) => review.add(at, text, quote, sideOf(at) === 'old' ? commentBase : undefined),
  }
  const text = diff.data?.pages.join('')
  const files = useMemo(() => (text === undefined ? [] : parseFiles(text)), [text])
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
  return { stats, whole, diff, text, files, commenting, unsent, changed, error: stats.error ?? diff.error }
}
