import { memo, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import type { Hidden } from '../api'
import { type View, VIEWS, baseOf } from '../diffSpec'
import { useHideGit, useHideOthers, useShared } from '../hideGit'
import { message } from '../query'
import { keptView } from '../reviewView'
import { itemOf, useReview } from '../review'
import { useActions } from '../shortcuts'
import { useThreadDiff } from '../threadDiff'
import { Button, Input, Select, Skeleton, Tabs } from '../ui'
import { ChipSelect } from './ChipSelect'
import { type DiffLayout, DiffView } from './DiffView'
import { FinishReview } from './FinishReview'
import { OutdatedDrafts } from './OutdatedDrafts'
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
  // Against main the diff is git's own; elsewhere git's rewrites (ADR 0069) and other agents' edits (ADR 0089) can be
  // left out.
  const [hideGitSetting, setHideGit] = useHideGit()
  const [hideOthersSetting, setHideOthers] = useHideOthers()
  const shared = useShared(thread)
  const hideGit = hideGitSetting && view !== 'main'
  const hideOthers = hideOthersSetting && view !== 'main' && shared
  const of = { base, hideGit, hideOthers }
  const review = useReview(thread)
  const { stats, whole, diff, text, files, commenting, unsent, changed, pending, outdated, error } = useThreadDiff(
    thread,
    of,
    review,
    onCommented,
    agent,
  )
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
      <ChipSelect
        label="compare"
        value={view}
        onChange={(chosen) => setView(chosen as View)}
        options={VIEWS.map((choice) => ({ id: choice.id, name: choice.label }))}
      />
      <HideGitBox on={hideGitSetting} onChange={setHideGit} disabled={view === 'main'} />
      {shared && <HideOthersBox on={hideOthersSetting} onChange={setHideOthers} disabled={view === 'main'} />}
      <Tabs label="layout" tabs={LAYOUTS} value={layout} onChange={setPicked} />
    </>
  )
  return (
    <section ref={section} aria-label="changes" className="flex flex-col gap-3">
      {controlsIn ? (
        createPortal(controls, controlsIn)
      ) : (
        <div className="flex flex-wrap items-center justify-end gap-2">{controls}</div>
      )}
      {view === 'other' && (
        <div className="flex flex-wrap items-center gap-2">
          <Select aria-label="diff base" value={other} onChange={(event) => setOther(event.target.value)}>
            <option value="merge-base">Since the merge base with its source</option>
            <option value="last-commit">Since the last commit</option>
            <option value="commit">Since a commit…</option>
          </Select>
          {other === 'commit' && (
            <Input
              aria-label="commit"
              placeholder="sha"
              className="w-28 font-mono"
              value={commit}
              onChange={(event) => setCommit(event.target.value)}
            />
          )}
        </div>
      )}
      {(review.review.drafts.length > 0 || unsent.length > 0) && (
        <div className="flex justify-end">
          <FinishReview
            thread={thread}
            review={pending}
            unsent={unsent}
            items={(comment) => itemOf(comment, files)}
            agent={agent}
            working={working}
            onSent={changed}
          />
        </div>
      )}
      <OutdatedDrafts drafts={outdated} review={review} />
      {error && <p className="text-sm text-danger">{message(error)}</p>}
      {(stats.isPending || (whole && diff.isPending)) && <Skeleton className="h-32" />}
      <HiddenNote hidden={of} />
      {whole === false && <FilesByStats thread={thread} stats={stats.data ?? []} of={of} commenting={commenting} />}
      {whole && text !== undefined && (
        <DiffView text={text} layout={layout} viewFiles={thread} commenting={commenting} anchors={reviewed !== undefined} />
      )}
      {whole && diff.hasNextPage && (
        <Button className="self-start" disabled={diff.isFetchingNextPage} onClick={() => void diff.fetchNextPage()}>
          More files
        </Button>
      )}
    </section>
  )
})

interface BoxProps {
  on: boolean
  onChange: (on: boolean) => void
  disabled?: boolean
}

/** Leaves out of the diffs what git itself rewrote, as a checkout, pull or stash does (ADR 0069). */
export function HideGitBox(props: BoxProps) {
  return <HideBox {...props} label="Hide git’s changes" title="A checkout, pull, merge, rebase, reset or stash the agent ran" />
}

/** Leaves out of the diffs the edits of other agents working in the same clone (ADR 0089). */
export function HideOthersBox(props: BoxProps) {
  return <HideBox {...props} label="Hide other agents’ changes" title="Edits by another agent working in the same clone" />
}

function HideBox({ on, onChange, disabled = false, label, title }: BoxProps & { label: string; title: string }) {
  return (
    <label
      className="flex min-h-8 items-center gap-1.5 text-xs text-fg-2 max-md:min-h-11"
      title={disabled ? 'Against main, the diff is git’s own' : title}
    >
      <input
        type="checkbox"
        checked={on && !disabled}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
        className="accent-[var(--accent)] max-md:h-5 max-md:w-5"
      />
      {label}
    </label>
  )
}

/** Says what a diff leaves out, as what's on disk may differ. */
export function HiddenNote({ hidden }: { hidden: Hidden }) {
  const left = [
    hidden.hideGit && 'what git rewrote, as a checkout, pull or stash does',
    hidden.hideOthers && 'other agents’ edits in this clone',
  ].filter(Boolean)
  if (left.length === 0) return null
  return (
    <p role="note" className="text-xs text-fg-2">
      Without {left.join(', or ')}; what's on disk may differ.
    </p>
  )
}

/** How wide the panel must be for a diff side by side, in pixels. */
const SPLIT_WIDTH = 760
