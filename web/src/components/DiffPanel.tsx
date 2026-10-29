import { useInfiniteQuery } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { api, type CommentView } from '../api'
import { keys, message } from '../query'
import { linesOf, reviewPrompt } from '../review'
import { useActions } from '../shortcuts'
import { Button, Input, Select, Skeleton, Tabs, toast } from '../ui'
import { ChipSelect } from './ChipSelect'
import { type DiffLayout, DiffView } from './DiffView'

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

/** What the thread changed, by turn, overall or against main, with comments from the gutter. */
export function DiffPanel({
  thread,
  comments,
  onCommented,
  agent,
  wide = false,
  controlsIn,
}: {
  thread: string
  comments: CommentView[]
  onCommented: () => Promise<unknown>
  /** The agent a comment can go to straight away. */
  agent?: string
  /** Room for the diff side by side. */
  wide?: boolean
  /** Where what it compares and how go, beside the inspector's tabs; above the diff without it. */
  controlsIn?: HTMLElement | null
}) {
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
  const [sent, setSent] = useState<string>()
  const [error, setError] = useState<string>()
  const base =
    view === 'thread' ? '' : view === 'other' ? (other === 'commit' ? (commit ? `commit:${commit}` : '') : other) : view
  const diff = useInfiniteQuery({
    queryKey: keys.diffPages(thread, base),
    queryFn: ({ pageParam }) => api.diff(thread, { base, offset: pageParam, limit: FILES_PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last, pages) => (filesIn(last) < FILES_PAGE ? undefined : pages.length * FILES_PAGE),
  })
  const pending = comments.filter((comment) => !comment.resolved && linesOf(comment) !== null)
  useActions([
    { id: 'last-turn', label: 'Changes in the last turn', run: () => setView('last-turn') },
    { id: 'against-main', label: 'Changes against main', run: () => setView('main') },
  ])
  const send = (prompt: string) =>
    api.prompt(thread, prompt).then((how) => setSent(how === 'queued' ? 'Queued for the agent' : 'Sent to the agent'))

  // What it compares is a menu, as it's picked once in a while; the layout stays a toggle.
  const controls = (
    <>
      <ChipSelect label="compare" value={view} onChange={(chosen) => setView(chosen as View)} options={VIEWS.map((choice) => ({ id: choice.id, name: choice.label }))} />
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
      {pending.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="primary"
            onClick={() => {
              setError(undefined)
              send(reviewPrompt(pending)).catch((failure: unknown) => setError(message(failure)))
            }}
          >
            Send {pending.length} {pending.length === 1 ? 'comment' : 'comments'} to the agent
          </Button>
          {sent && <span className="text-xs text-fg-2">{sent}</span>}
        </div>
      )}
      {error && <p className="text-sm text-danger">{error}</p>}
      {diff.error && <p className="text-sm text-danger">{message(diff.error)}</p>}
      {diff.isPending && <Skeleton className="h-32" />}
      {diff.data !== undefined && (
        <DiffView
          text={diff.data.pages.join('')}
          layout={layout}
          viewFiles={thread}
          commenting={{
            thread,
            comments,
            agent,
            onChanged: onCommented,
            onComment: async (at, text, now) => {
              const made = await api.addComment(thread, { on: 'lines', ...at }, text)
              await onCommented()
              if (now) {
                await send(reviewPrompt([made]))
                toast('Sent to the agent', 'ok')
              }
            },
          }}
        />
      )}
      {diff.hasNextPage && (
        <Button className="self-start" disabled={diff.isFetchingNextPage} onClick={() => void diff.fetchNextPage()}>
          More files
        </Button>
      )}
    </section>
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
