import { memo, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { type View, VIEWS, baseOf } from '../diffSpec'
import { message } from '../query'
import { keptView } from '../reviewView'
import { itemOf, useReview } from '../review'
import { useActions } from '../shortcuts'
import { useThreadDiff } from '../threadDiff'
import { Button, Input, Select, Skeleton, Tabs } from '../ui'
import { ChipSelect } from './ChipSelect'
import { type DiffLayout, DiffView } from './DiffView'
import { FinishReview } from './FinishReview'
import { FilesByStats } from './StatDiff'

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
  working = false,
  wide = false,
  controlsIn,
}: {
  thread: string
  /** After a comment is made or changes, for the thread's other views of them. */
  onCommented: () => Promise<unknown>
  /** The agent a review goes to. */
  agent?: string
  /** Whether that agent is in a turn. */
  working?: boolean
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
  const [reviewed] = useState(() => keptView(thread).file)
  const [other, setOther] = useState('merge-base')
  const [commit, setCommit] = useState('')
  const base = baseOf(view, other, commit)
  const review = useReview(thread)
  const { stats, whole, diff, text, files, commenting, unsent, changed, error } = useThreadDiff(thread, base, review, onCommented, agent)
  const shownReviewed = text !== undefined && reviewed !== undefined
  useEffect(() => {
    if (shownReviewed) document.getElementById(`file-${reviewed}`)?.scrollIntoView({ block: 'start' })
  }, [shownReviewed, reviewed])
  useActions([
    { id: 'last-turn', label: 'Changes in the last turn', run: () => setView('last-turn') },
    { id: 'against-main', label: 'Changes against main', run: () => setView('main') },
  ])

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
      {(review.review.drafts.length > 0 || unsent.length > 0) && (
        <div className="flex justify-end">
          <FinishReview thread={thread} review={review} unsent={unsent} items={(comment) => itemOf(comment, files)} agent={agent} working={working} onSent={changed} />
        </div>
      )}
      {error && <p className="text-sm text-danger">{message(error)}</p>}
      {(stats.isPending || (whole && diff.isPending)) && <Skeleton className="h-32" />}
      {whole === false && <FilesByStats thread={thread} stats={stats.data ?? []} of={{ base }} commenting={commenting} />}
      {whole && text !== undefined && <DiffView text={text} layout={layout} viewFiles={thread} commenting={commenting} anchors={reviewed !== undefined} />}
      {whole && diff.hasNextPage && (
        <Button className="self-start" disabled={diff.isFetchingNextPage} onClick={() => void diff.fetchNextPage()}>
          More files
        </Button>
      )}
    </section>
  )
})

/** How wide the panel must be for a diff side by side, in pixels. */
const SPLIT_WIDTH = 760
