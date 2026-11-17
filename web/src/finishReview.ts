import { useState } from 'react'
import { api, type CommentView, hiddenLines } from './api'
import { message } from './query'
import { type ReviewItem, type ReviewState, reviewPrompt } from './review'
import { toast } from './ui'

/** How a review leaves: as a prompt to the agent running here, in place of its running turn, copied for one in a
 * terminal, kept as comments for later, or with nothing to change, as reviewed through a turn (ADR 0067). */
export type Finish = 'send' | 'steer' | 'copy' | 'save' | 'reviewed'

export interface FinishProps {
  thread: string
  review: ReviewState
  /** Open line comments not sent to the agent yet, which go with the review. */
  unsent: CommentView[]
  items: (comment: CommentView) => ReviewItem | null
  /** The agent running on the thread, when one is. */
  agent?: string
  /** Whether that agent is in a turn, which a review then waits for or stops. */
  working?: boolean
  onSent: () => Promise<unknown>
  /** The turn a review marked reviewed is through, where it can be marked so. */
  reviewedTurn?: number
}

/** Finishing a review: each pending comment becomes a comment, then the lot goes to the agent as one prompt. */
export function useFinishReview({ thread, review, unsent, items, agent, onSent, reviewedTurn }: FinishProps) {
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string>()
  const { drafts, note } = review.review
  const who = agent ?? 'the agent'
  const finish = async (how: Finish): Promise<boolean> => {
    setError(undefined)
    setSending(true)
    const prompt = reviewPrompt(
      [...drafts.map((draft) => ({ at: draft.at, text: draft.text, quote: draft.quote })), ...unsent.flatMap((comment) => items(comment) ?? [])],
      note,
    )
    // Copied at once, while the click still lets the page write the clipboard.
    if (how === 'copy') void navigator.clipboard?.writeText(prompt).catch(() => undefined)
    try {
      const ids = unsent.map((comment) => comment.id)
      // Each draft becomes a comment first, and leaves the review once it is one, so a failure halfway repeats none.
      for (const draft of drafts) {
        const { at } = draft
        const on = draft.anchor ? { on: 'anchored' as const, anchor: draft.anchor } : { on: 'lines' as const, path: at.path, start: at.start, end: at.end, base: draft.base, ...hiddenLines(draft) }
        const made = await api.addComment(thread, on, draft.text)
        review.remove(draft.id)
        ids.push(made.id)
      }
      if (how === 'reviewed' && reviewedTurn !== undefined) {
        await api.setThreadState(thread, { pinned: null, snooze: null, settled: null, seen: false, reviewed_turn: reviewedTurn })
        review.setNote('')
        toast(drafts.length > 0 ? `Reviewed through turn ${reviewedTurn}; the comments are kept` : `Reviewed through turn ${reviewedTurn}`, 'ok')
        return true
      }
      if (how === 'save') {
        review.setNote('')
        toast('Kept as comments; the review sends them once an agent runs', 'ok')
        return true
      }
      const sent = how === 'send' ? await api.prompt(thread, prompt) : how === 'steer' ? await api.steer(thread, prompt) : 'copied'
      await api.commentsSent(thread, ids)
      review.clear()
      toast(
        sent === 'copied'
          ? "Review copied: paste it into the agent's terminal"
          : sent === 'queued'
            ? `Review queued for ${who}, after its turn`
            : sent === 'steering'
              ? `Review sent to ${who}, in place of its turn`
              : `Review sent to ${who}`,
        'ok',
      )
      return true
    } catch (failure: unknown) {
      setError(message(failure))
      return false
    } finally {
      setSending(false)
      await onSent()
    }
  }
  return { count: drafts.length + unsent.length, sending, error, finish }
}

