// The expanded review (ADR 0067): a thread's changes across the conversation's column and the panel's, its files
// to tick off on the left, a diff split by default, and keys to move through it, with nothing of the conversation.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate, useParams } from '@tanstack/react-router'
import { useEffect, useMemo, useRef, useState } from 'react'
import { api, type ThreadSummary } from '../api'
import { useTopic } from '../bus'
import { useDetailChoice, useGroupingChoice } from '../detail'
import type { DiffFile } from '../diffParse'
import { type View, VIEWS, baseOf } from '../diffSpec'
import { keys, message } from '../query'
import { type Lines, itemOf, useReview } from '../review'
import { type At, filtered, firstChange, stepFile, stepHunk } from '../reviewNav'
import { type Kept, keepView, keptView } from '../reviewView'
import { useLive } from '../session'
import { useActions } from '../shortcuts'
import { useThreadDiff } from '../threadDiff'
import { Input, Select, Skeleton, Tabs } from '../ui'
import { ChipSelect } from './ChipSelect'
import { type DiffLayout, DiffView } from './DiffView'
import { FinishReview } from './FinishReview'
import { SeenFiles } from './SeenFiles'
import { FilesByStats } from './StatDiff'
import { ThreadHeader } from './ThreadView'

const LAYOUTS: { id: DiffLayout; label: string }[] = [
  { id: 'unified', label: 'Unified' },
  { id: 'split', label: 'Split' },
]

export function ReviewRoute() {
  const { id } = useParams({ from: '/t/$id/review' })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  // A short id is the end of the full one, as the app shows it, or its start, as the CLI takes it.
  const summary =
    threads.data?.find((thread) => thread.id === id) ?? threads.data?.find((thread) => thread.id.endsWith(id) || thread.id.startsWith(id))
  const navigate = useNavigate()
  // A short id in the address becomes the full one, which the bus's events name.
  useEffect(() => {
    if (summary && summary.id !== id) void navigate({ to: '/t/$id/review', params: { id: summary.id }, replace: true })
  }, [summary, id, navigate])
  useTopic('threads')
  useTopic(summary ? `thread:${summary.id}` : null)
  if (threads.error) return <p className="p-6 text-danger">{message(threads.error)}</p>
  if (!threads.data)
    return (
      <div className="flex flex-col gap-3 p-6">
        <Skeleton className="h-6 w-1/3" />
        <Skeleton className="h-40" />
      </div>
    )
  if (!summary) return <p className="p-6 text-sm text-fg-2">No thread {id} here.</p>
  return <Review key={summary.id} thread={summary.id} summary={summary} />
}

function Review({ thread, summary }: { thread: string; summary: ThreadSummary }) {
  const navigate = useNavigate()
  const queries = useQueryClient()
  const [kept, setKept] = useState(() => keptView(thread))
  const change = (next: Partial<Kept>) =>
    setKept((before) => {
      const after = { ...before, ...next }
      keepView(thread, after)
      return after
    })
  const base = baseOf(kept.view, kept.other, kept.commit)
  const review = useReview(thread)
  const { agent: status } = useLive(thread)
  const agent = status?.agent
  const working = status?.state === 'working' || status?.state === 'awaiting_permission' || status?.state === 'starting'
  const reload = () => queries.invalidateQueries({ queryKey: keys.comments(thread) })
  const { stats, whole, diff, files, commenting, unsent, changed, error } = useThreadDiff(thread, base, review, reload, agent)
  // The review reads every page of the diff, which `readWhole` keeps to what a page can draw.
  useEffect(() => {
    if (diff.hasNextPage && !diff.isFetchingNextPage) void diff.fetchNextPage()
  }, [diff])
  const [detail, setDetail] = useDetailChoice()
  const [grouping, setGrouping] = useGroupingChoice()
  const [filter, setFilter] = useState('')
  const filterBox = useRef<HTMLInputElement>(null)
  const pane = useRef<HTMLElement>(null)
  const [at, setAt] = useState<At | null>(null)
  const [ask, setAsk] = useState<{ at: Lines; key: number }>()
  const opener = useRef<(() => void) | null>(null)
  const shown = filtered(files, filter)
  const hunks = shown.map((file) => file.hunks.length)
  const totals = useMemo(
    () => stats.data && { added: stats.data.reduce((sum, file) => sum + file.added, 0), removed: stats.data.reduce((sum, file) => sum + file.removed, 0) },
    [stats.data],
  )
  const back = () => void navigate({ to: '/t/$id', params: { id: thread } })

  const show = (file: DiffFile, hunk?: number) => {
    change({ file: file.path })
    const target = hunk === undefined ? document.getElementById(`file-${file.path}`) : pane.current?.querySelector(`[data-hunk="${CSS.escape(`${file.path}#${hunk}`)}"]`)
    target?.scrollIntoView({ block: 'start' })
  }
  const go = (next: At | null) => {
    if (!next) return
    const file = shown[next.file]
    if (!file) return
    setAt(next)
    show(file, next.hunk)
  }
  const current = at ?? (kept.file ? { file: Math.max(0, shown.findIndex((file) => file.path === kept.file)), hunk: 0 } : { file: 0, hunk: 0 })
  useActions(
    [
      { id: 'review:back', label: 'Back to the conversation', keys: 'escape', run: back },
      { id: 'review:next-file', label: 'Next file', keys: 'j', run: () => go(at ? stepFile(hunks, current, 1) : current) },
      { id: 'review:previous-file', label: 'Previous file', keys: 'k', run: () => go(stepFile(hunks, current, -1)) },
      { id: 'review:next-hunk', label: 'Next hunk', keys: 'n', run: () => go(at ? stepHunk(hunks, current, 1) : current) },
      { id: 'review:previous-hunk', label: 'Previous hunk', keys: 'p', run: () => go(stepHunk(hunks, current, -1)) },
      {
        id: 'review:comment',
        label: 'Comment on this hunk',
        keys: 'c',
        run: () => {
          const file = shown[current.file]
          const lines = file && firstChange(file, current.hunk, commenting.deleted ?? false)
          if (lines) setAsk({ at: lines, key: Date.now() })
        },
      },
      { id: 'review:layout', label: kept.layout === 'split' ? 'Unified diff' : 'Split diff', keys: 's', run: () => change({ layout: kept.layout === 'split' ? 'unified' : 'split' }) },
      { id: 'review:filter', label: 'Filter files', keys: '/', run: () => filterBox.current?.focus() },
      { id: 'review:finish', label: 'Finish the review', keys: 'mod+enter', run: () => opener.current?.() },
    ],
    { over: true },
  )

  const cursor = at && shown[at.file] ? { path: shown[at.file].path, hunk: at.hunk } : undefined
  return (
    <div className="flex h-full min-h-0 flex-col" data-page="review">
      <ThreadHeader
        thread={thread}
        summary={summary}
        people={[]}
        changes={totals ?? null}
        panelOpen={false}
        waiting={status?.state === 'awaiting_input' || status?.state === 'awaiting_permission'}
        onChanges={back}
        onDetails={back}
        detail={detail}
        onDetail={setDetail}
        grouping={grouping}
        onGrouping={setGrouping}
        onConversation={back}
      />
      <div className="flex min-h-12 shrink-0 flex-wrap items-center gap-2 border-b border-line-subtle px-4 py-1.5 md:pl-5">
        <h2 className="text-xs font-medium">Changes</h2>
        <span className="ml-auto flex flex-wrap items-center gap-2">
          <ChipSelect label="compare" value={kept.view} onChange={(chosen) => change({ view: chosen as View })} options={VIEWS.map((choice) => ({ id: choice.id, name: choice.label }))} />
          <Tabs label="layout" tabs={LAYOUTS} value={kept.layout} onChange={(layout) => change({ layout })} />
          <FinishReview
            thread={thread}
            review={review}
            unsent={unsent}
            items={(comment) => itemOf(comment, files)}
            agent={agent}
            working={working}
            onSent={changed}
            opener={(open) => {
              opener.current = open
            }}
          />
        </span>
      </div>
      {kept.view === 'other' && (
        <div className="flex flex-wrap items-center gap-2 border-b border-line-subtle px-5 py-2">
          <Select aria-label="diff base" value={kept.other} onChange={(event) => change({ other: event.target.value })}>
            <option value="merge-base">Since the merge base with its source</option>
            <option value="last-commit">Since the last commit</option>
            <option value="commit">Since a commit…</option>
          </Select>
          {kept.other === 'commit' && (
            <Input aria-label="commit" placeholder="sha" className="w-28 font-mono" value={kept.commit} onChange={(event) => change({ commit: event.target.value })} />
          )}
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <div className="flex w-64 shrink-0 flex-col gap-2 overflow-y-auto border-r border-line-subtle p-3">
          <Input ref={filterBox} aria-label="filter files" placeholder="Filter files ( / )" value={filter} onChange={(event) => setFilter(event.target.value)} className="text-xs" />
          <SeenFiles
            files={filtered(stats.data ?? [], filter).map((file) => ({ path: file.path, added: file.added, removed: file.removed, binary: file.binary, seen: 'no' as const }))}
            picked={cursor?.path ?? kept.file}
            link={(path) => (
              <button
                type="button"
                className="min-w-0 flex-1 truncate text-left text-inherit hover:text-fg-1"
                title={path}
                onClick={() => {
                  const index = shown.findIndex((file) => file.path === path)
                  if (index >= 0) go({ file: index, hunk: 0 })
                  else change({ file: path })
                }}
              >
                {path}
              </button>
            )}
          />
        </div>
        <section ref={pane} aria-label="changes" className="flex min-w-0 flex-1 flex-col gap-3 overflow-y-auto px-5 py-4">
          {error && <p className="text-sm text-danger">{message(error)}</p>}
          {(stats.isPending || (whole && diff.isPending)) && <Skeleton className="h-32" />}
          {whole === false && <FilesByStats thread={thread} stats={filtered(stats.data ?? [], filter)} of={{ base }} commenting={commenting} />}
          {whole && files.length === 0 && !diff.isPending && <p className="text-sm text-fg-2">No changes.</p>}
          {whole && shown.length > 0 && (
            <DiffView files={shown} layout={kept.layout} viewFiles={thread} commenting={commenting} anchors picked={cursor?.path} cursor={cursor} ask={ask} />
          )}
        </section>
      </div>
    </div>
  )
}
