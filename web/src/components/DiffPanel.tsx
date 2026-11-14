import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query'
import { memo, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { api, type CommentView } from '../api'
import { parseFiles, readWhole } from '../diffParse'
import { useHideGit } from '../hideGit'
import { keys, message } from '../query'
import { type ReviewItem, type ReviewState, itemOf, linesOf, reviewPrompt, sideOf, spanOf, useReview } from '../review'
import { useActions } from '../shortcuts'
import { Button, Input, Select, Skeleton, Tabs, Textarea, toast } from '../ui'
import { ChipSelect } from './ChipSelect'
import { type Commenting, type DiffLayout, DiffView } from './DiffView'
import { FilesByStats } from './StatDiff'

type View = 'last-turn' | 'thread' | 'main' | 'other'

const VIEWS: { id: View; label: string }[] = [
  { id: 'last-turn', label: 'Last turn' },
  { id: 'thread', label: 'This thread' },
  { id: 'main', label: 'Against main' },
  { id: 'other', label: 'Other…' },
]

const LAYOUTS: { id: DiffLayout; label: string }[] = [
  { id: 'unified', label: 'Unified' },
  { id: 'split', label: 'Split' },
]

/** What the thread changed, by turn, overall or against main, reviewed from the gutter (ADR 0055).
 * Memoized, as the thread's page renders again with each key typed in its composer, and a large diff takes long to draw. */
export const DiffPanel = memo(function DiffPanel({
  thread,
  onCommented,
  agent,
  wide = false,
  controlsIn,
}: {
  thread: string
  /** After a comment is made or changes, for the thread's other views of them. */
  onCommented: () => Promise<unknown>
  /** The agent a review goes to. */
  agent?: string
  /** Room for the diff side by side. */
  wide?: boolean
  /** Where what it compares and how go, beside the inspector's tabs; above the diff without it. */
  controlsIn?: HTMLElement | null
}) {
  const queries = useQueryClient()
  const [view, setView] = useState<View>('thread')
  // Side by side only where both sides fit, until the person picks.
  const [picked, setPicked] = useState<DiffLayout>()
  const [width, setWidth] = useState(0)
  const section = useRef<HTMLElement>(null)
  useEffect(() => {
    const element = section.current
    if (!element) return
    const observer = new ResizeObserver(([entry]) => setWidth(entry?.contentRect.width ?? 0))
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  const layout: DiffLayout = picked ?? (wide && width >= SPLIT_WIDTH ? 'split' : 'unified')
  const [other, setOther] = useState('merge-base')
  const [commit, setCommit] = useState('')
  const base =
    view === 'thread' ? '' : view === 'other' ? (other === 'commit' ? (commit ? `commit:${commit}` : '') : other) : view
  // Lines the change deleted are a version of the thread's, which a commit of main isn't.
  const commentBase = base === '' ? 'start' : base === 'main' ? undefined : base
  // Against main the diff is git's own; elsewhere git's rewrites can be left out (ADR 0066).
  const [hideGitSetting, setHideGit] = useHideGit()
  const hideGit = hideGitSetting && view !== 'main'
  // Its files first: one too large to read whole is read a file at a time.
  const of = { base, hideGit }
  const stats = useQuery({ queryKey: keys.diffStats(thread, of), queryFn: () => api.diffStats(thread, of) })
  const whole = stats.data === undefined ? undefined : readWhole(stats.data)
  const diff = useInfiniteQuery({
    queryKey: keys.diffPages(thread, base, hideGit),
    queryFn: ({ pageParam }) => api.diff(thread, { ...of, offset: pageParam, limit: FILES_PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last, pages) => (filesIn(last) < FILES_PAGE ? undefined : pages.length * FILES_PAGE),
    enabled: whole === true,
  })
  const comments = useQuery({ queryKey: [...keys.comments(thread), commentBase ?? ''], queryFn: () => api.comments(thread, commentBase) })
  const changed = () => Promise.all([queries.invalidateQueries({ queryKey: keys.comments(thread) }), onCommented()])
  const review = useReview(thread)
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
      const base = sideOf(at) === 'old' ? commentBase : undefined
      await api.addComment(thread, { on: 'lines', path: at.path, start: at.start, end: at.end, base }, comment)
      await changed()
    },
  }
  const unsent = (comments.data ?? []).filter((comment) => !comment.resolved && !comment.sent && linesOf(comment) !== null)
  useActions([
    { id: 'last-turn', label: 'Changes in the last turn', run: () => setView('last-turn') },
    { id: 'against-main', label: 'Changes against main', run: () => setView('main') },
  ])

  // What it compares is a menu, as it's picked once in a while; the layout stays a toggle.
  const controls = (
    <>
      <ChipSelect label="compare" value={view} onChange={(chosen) => setView(chosen as View)} options={VIEWS.map((choice) => ({ id: choice.id, name: choice.label }))} />
      <HideGitBox on={hideGitSetting} onChange={setHideGit} disabled={view === 'main'} />
      <Tabs label="layout" tabs={LAYOUTS} value={layout} onChange={setPicked} />
    </>
  )
  return (
    <section ref={section} aria-label="changes" className="flex flex-col gap-3">
      {controlsIn ? createPortal(controls, controlsIn) : <div className="flex flex-wrap items-center justify-end gap-2">{controls}</div>}
      {view === 'other' && (
        <div className="flex flex-wrap items-center gap-2">
          <Select aria-label="diff base" value={other} onChange={(event) => setOther(event.target.value)}>
            <option value="merge-base">Since the merge base with its source</option>
            <option value="last-commit">Since the last commit</option>
            <option value="commit">Since a commit…</option>
          </Select>
          {other === 'commit' && (
            <Input aria-label="commit" placeholder="sha" className="w-28 font-mono" value={commit} onChange={(event) => setCommit(event.target.value)} />
          )}
        </div>
      )}
      {(review.review.drafts.length > 0 || unsent.length > 0) && (
        <ReviewBar thread={thread} review={review} unsent={unsent} items={(comment) => itemOf(comment, files)} agent={agent} onSent={changed} />
      )}
      {(stats.error ?? diff.error) && <p className="text-sm text-danger">{message(stats.error ?? diff.error)}</p>}
      {(stats.isPending || (whole && diff.isPending)) && <Skeleton className="h-32" />}
      {hideGit && (
        <p role="note" className="text-xs text-fg-2">
          Without what git rewrote in the agent's tool calls, as a checkout, pull or stash does; what's on disk may differ. Show git's changes to comment.
        </p>
      )}
      {whole === false && <FilesByStats thread={thread} stats={stats.data ?? []} of={of} commenting={hideGit ? undefined : commenting} />}
      {whole && text !== undefined && <DiffView text={text} layout={layout} viewFiles={thread} commenting={hideGit ? undefined : commenting} />}
      {whole && diff.hasNextPage && (
        <Button className="self-start" disabled={diff.isFetchingNextPage} onClick={() => void diff.fetchNextPage()}>
          More files
        </Button>
      )}
    </section>
  )
})

/** Leaves out of the diffs what git itself rewrote in a tool call: their line numbers then aren't the file's, so comments wait until it's off. */
export function HideGitBox({ on, onChange, disabled = false }: { on: boolean; onChange: (on: boolean) => void; disabled?: boolean }) {
  return (
    <label
      className="flex min-h-8 items-center gap-1.5 text-xs text-fg-2 max-md:min-h-11"
      title={disabled ? 'Against main, the diff is git’s own' : 'A checkout, pull, merge, rebase, reset or stash the agent ran'}
    >
      <input
        type="checkbox"
        checked={on && !disabled}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
        className="accent-[var(--accent)] max-md:h-5 max-md:w-5"
      />
      Hide git’s changes
    </label>
  )
}

/** The review in progress: what it holds, and finishing it with a note, which sends it all to the agent as one prompt. */
/** How a review leaves: as a prompt to the agent running here, copied for one in a terminal, or kept as comments for later. */
type Finish = 'send' | 'copy' | 'save'

function ReviewBar({
  thread,
  review,
  unsent,
  items,
  agent,
  onSent,
}: {
  thread: string
  review: ReviewState
  unsent: CommentView[]
  items: (comment: CommentView) => ReviewItem | null
  /** The agent running on the thread, when one is. */
  agent?: string
  onSent: () => Promise<unknown>
}) {
  const [finishing, setFinishing] = useState(false)
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string>()
  const { drafts, note } = review.review
  const count = drafts.length + unsent.length
  const who = agent ?? 'the agent'
  const comments = `${count} ${count === 1 ? 'comment' : 'comments'}`
  const finish = async (how: Finish) => {
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
        const made = await api.addComment(thread, { on: 'lines', path: at.path, start: at.start, end: at.end, base: draft.base }, draft.text)
        review.remove(draft.id)
        ids.push(made.id)
      }
      if (how === 'save') {
        setFinishing(false)
        toast('Kept as comments; the review sends them once an agent runs', 'ok')
        return
      }
      const sent = how === 'send' ? await api.prompt(thread, prompt) : 'copied'
      await api.commentsSent(thread, ids)
      review.clear()
      setFinishing(false)
      toast(
        sent === 'copied' ? "Review copied: paste it into the agent's terminal" : sent === 'queued' ? `Review queued for ${who}` : `Review sent to ${who}`,
        'ok',
      )
    } catch (failure: unknown) {
      setError(message(failure))
    } finally {
      setSending(false)
      await onSent()
    }
  }
  return (
    <div role="region" aria-label="review" className="flex flex-col gap-2 rounded-md border border-accent/60 bg-accent-tint px-3 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-xs text-fg-1">
          <span className="font-medium">Review</span>{' '}
          <span className="text-fg-2">
            {drafts.length > 0 && `${drafts.length} pending`}
            {drafts.length > 0 && unsent.length > 0 && ' · '}
            {unsent.length > 0 && `${unsent.length} not sent yet`}
          </span>
        </span>
        <span className="ml-auto flex gap-1.5">
          {drafts.length > 0 && !finishing && (
            <Button variant="ghost" size="sm" onClick={() => review.clear()} title="Drops the pending comments">
              Discard
            </Button>
          )}
          {!finishing && (
            <Button variant="primary" size="sm" icon="handoff" onClick={() => setFinishing(true)}>
              Finish review
            </Button>
          )}
        </span>
      </div>
      {finishing && (
        <form
          aria-label="finish review"
          className="flex flex-col gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            void finish(agent ? 'send' : 'copy')
          }}
        >
          <Textarea
            aria-label="review note"
            autoFocus
            rows={3}
            className="text-xs"
            placeholder={`A note for ${who} with the comments (optional)`}
            value={note}
            onChange={(event) => review.setNote(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
                event.preventDefault()
                void finish(agent ? 'send' : 'copy')
              }
            }}
          />
          <ul aria-label="comments in the review" className="flex flex-col gap-0.5 font-mono text-2xs text-fg-2">
            {drafts.map((draft) => (
              <li key={draft.id} className="truncate">
                {spanOf(draft.at)}
                {sideOf(draft.at) === 'old' && ' (deleted)'}: <span className="font-sans text-fg-1">{draft.text}</span>
              </li>
            ))}
            {unsent.map((comment) => (
              <li key={comment.id} className="truncate">
                {spanOf(linesOf(comment)!)} ({comment.by}): <span className="font-sans text-fg-1">{comment.text}</span>
              </li>
            ))}
          </ul>
          {!agent && (
            <p className="text-2xs leading-normal text-fg-2">
              No agent runs here from Silt. Copy the review into the agent’s terminal, or keep the comments and send them once you start one.
            </p>
          )}
          <span className="flex flex-wrap justify-end gap-1.5">
            <Button variant="ghost" size="sm" onClick={() => setFinishing(false)}>
              Cancel
            </Button>
            {!agent && drafts.length > 0 && (
              <Button size="sm" disabled={sending} onClick={() => void finish('save')}>
                Keep as comments
              </Button>
            )}
            <Button type="submit" variant="primary" size="sm" disabled={sending || count === 0}>
              {agent ? `Send ${comments} to ${who}` : `Copy ${comments} for the agent`}
            </Button>
          </span>
          {error && <p className="text-xs text-danger">{error}</p>}
        </form>
      )}
    </div>
  )
}

/** Files shown in a page of the changes panel: a change touching thousands
 * comes a page at a time. */
const FILES_PAGE = 50
/** How wide the panel must be for a diff side by side, in pixels. */
const SPLIT_WIDTH = 760

/** How many files a page of a diff holds. */
function filesIn(diff: string): number {
  return diff.split('\n').filter((line) => line.startsWith('diff --git ')).length
}
