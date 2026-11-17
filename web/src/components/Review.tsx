// The expanded review (ADR 0067): a thread's changes across the conversation's column and the panel's, its files
// to tick off on the left, a diff split by default, and keys to move through it, with nothing of the conversation.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useParams } from '@tanstack/react-router'
import { useEffect, useMemo, useRef, useState } from 'react'
import { api, type DiffSpec, type ThreadSummary, type TurnInfo } from '../api'
import { useTopic } from '../bus'
import { useDetailChoice, useGroupingChoice } from '../detail'
import type { DiffFile } from '../diffParse'
import { type View, VIEWS, baseOf } from '../diffSpec'
import { useHideGit, useHideOthers } from '../hideGit'
import { keys, message } from '../query'
import { type Lines, itemOf, useReview } from '../review'
import { type At, defaultTurns, filtered, firstChange, linesNew, nextUnseen, pickTurns, seenOf, stepFile, stepHunk, turnsOfHunk } from '../reviewNav'
import { type Kept, keepView, keptView } from '../reviewView'
import { useLive } from '../session'
import { useActions } from '../shortcuts'
import { useThreadDiff } from '../threadDiff'
import { Button, Icon, Input, Select, Skeleton, Tabs, cx, toast } from '../ui'
import { ChipSelect } from './ChipSelect'
import { HiddenNote, HideGitBox, HideOthersBox } from './DiffPanel'
import { type DiffLayout, DiffView } from './DiffView'
import { FinishReview } from './FinishReview'
import { OutdatedDrafts } from './OutdatedDrafts'
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
  const turns = useQuery({ queryKey: keys.turns(thread), queryFn: () => api.turns(thread) })
  const reviewed = summary.mine.reviewed_turn
  // Until the person picks, a review starts after the turn they reviewed through.
  const picked = kept.chosen ? kept.turns : (kept.turns ?? defaultTurns(turns.data?.length ?? 0, reviewed))
  // Against main the diff is git's own; elsewhere git's rewrites (ADR 0069) and other agents' edits (ADR 0089) can be
  // left out.
  const againstMain = !picked && base === 'main'
  const [hideGitSetting, setHideGit] = useHideGit()
  const [hideOthersSetting, setHideOthers] = useHideOthers()
  const hidden = { hideGit: hideGitSetting && !againstMain, hideOthers: hideOthersSetting && !againstMain && summary.shared }
  const spec: DiffSpec = picked ? { turn: picked[0], toTurn: picked[1], ...hidden } : { base, ...hidden }
  const review = useReview(thread)
  const { agent: status } = useLive(thread)
  const agent = status?.agent
  const working = status?.state === 'working' || status?.state === 'awaiting_permission' || status?.state === 'starting'
  // What Mark reviewed records: the end of the turns picked, else the last turn that ended.
  const ended = (turns.data?.length ?? 0) - (working ? 1 : 0)
  const reviewedTurn = picked ? picked[1] : ended > 0 ? ended : undefined
  const reload = () => queries.invalidateQueries({ queryKey: keys.comments(thread) })
  const { stats, whole, diff, files, commenting, unsent, changed, pending, outdated, error } = useThreadDiff(thread, spec, review, reload, agent)
  const wrote = useQuery({ queryKey: keys.diffTurns(thread, spec), queryFn: () => api.diffTurns(thread, spec), enabled: whole === true })
  const runs = useMemo(() => new Map((wrote.data ?? []).map((file) => [file.path, file.runs])), [wrote.data])
  const only = picked && picked[0] === picked[1] ? picked[0] : undefined
  const seenView = useQuery({ queryKey: keys.threadSeen(thread, spec), queryFn: () => api.threadSeen(thread, spec) })
  const marks = useMemo(() => new Map((seenView.data?.files ?? []).map((file) => [file.path, file])), [seenView.data])
  // A tick shows at once, until the daemon's answer has it.
  const [ticked, setTicked] = useState<Record<string, boolean>>({})
  const seenState = (path: string) => (ticked[path] === undefined ? seenOf(path, marks) : ticked[path] ? 'yes' : 'no')
  const head = seenView.data?.head
  const markSeen = (path: string, seen: boolean) => {
    if (!head) return
    setTicked((before) => ({ ...before, [path]: seen }))
    void api
      .markThreadSeen(thread, { path, seen, at: head })
      .catch((failure: unknown) => toast(message(failure), 'danger'))
      .then(() => queries.invalidateQueries({ queryKey: keys.threadSeen(thread, spec) }))
      .then(() =>
        setTicked((before) => {
          const { [path]: _, ...rest } = before
          return rest
        }),
      )
  }
  const fresh = useMemo(() => {
    const lines = new Map<string, Set<number>>()
    for (const file of seenView.data?.files ?? []) {
      if (file.seen !== 'changed') continue
      lines.set(file.path, new Set(file.new_lines.flatMap((span) => Array.from({ length: span.end - span.start + 1 }, (_, index) => span.start + index))))
    }
    return lines
  }, [seenView.data])
  const turnsOf = (file: DiffFile, hunk: number) => {
    const shown = file.hunks[hunk]
    return shown ? turnsOfHunk(shown, runs.get(file.path) ?? [], only) : []
  }
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
      {
        id: 'review:turn',
        label: "Open this hunk's turn in the conversation",
        keys: 't',
        run: () => {
          const file = shown[current.file]
          const [turn] = file ? turnsOf(file, current.hunk) : []
          if (turn !== undefined) void navigate({ to: '/t/$id', params: { id: thread }, hash: `turn-${turn}` })
        },
      },
      {
        id: 'review:seen',
        label: 'Mark this file seen, or not',
        keys: 'v',
        run: () => {
          const file = shown[current.file]
          if (file) markSeen(file.path, seenState(file.path) !== 'yes')
        },
      },
      {
        id: 'review:next-unseen',
        label: "Next file you haven't seen, or that changed since",
        keys: ']',
        run: () => {
          const next = nextUnseen(
            shown.map((file) => file.path),
            marks,
            current.file,
          )
          if (next !== null) go({ file: next, hunk: 0 })
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
        <TurnPicker turns={turns.data ?? []} picked={picked} reviewed={reviewed} onPick={(chosen) => change({ turns: chosen, chosen: true })} />
        <span className="ml-auto flex flex-wrap items-center gap-2">
          {picked ? (
            <Button variant="ghost" size="sm" onClick={() => change({ turns: undefined, chosen: true })} title="Back to what the compare menu shows">
              All changes
            </Button>
          ) : (
            <ChipSelect label="compare" value={kept.view} onChange={(chosen) => change({ view: chosen as View })} options={VIEWS.map((choice) => ({ id: choice.id, name: choice.label }))} />
          )}
          <HideGitBox on={hideGitSetting} onChange={setHideGit} disabled={againstMain} />
          {summary.shared && <HideOthersBox on={hideOthersSetting} onChange={setHideOthers} disabled={againstMain} />}
          <Tabs label="layout" tabs={LAYOUTS} value={kept.layout} onChange={(layout) => change({ layout })} />
          <FinishReview
            thread={thread}
            review={pending}
            unsent={unsent}
            items={(comment) => itemOf(comment, files)}
            agent={agent}
            working={working}
            onSent={changed}
            reviewedTurn={reviewedTurn}
            opener={(open) => {
              opener.current = open
            }}
          />
        </span>
      </div>
      {kept.view === 'other' && !picked && (
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
            files={filtered(stats.data ?? [], filter).map((file) => {
              const seen = seenState(file.path)
              const count = linesNew(marks.get(file.path))
              const note =
                seen === 'changed' ? (
                  <span className="text-info">{count > 0 ? `${count} ${count === 1 ? 'line' : 'lines'} new since you saw it` : 'changed since you saw it'}</span>
                ) : undefined
              return { path: file.path, added: file.added, removed: file.removed, binary: file.binary, seen, note }
            })}
            onSeen={head ? markSeen : undefined}
            hint="A file you marked seen comes back, half-ticked, when the agent changes it again."
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
          <OutdatedDrafts drafts={outdated} review={review} />
          {error && <p className="text-sm text-danger">{message(error)}</p>}
          {(stats.isPending || (whole && diff.isPending)) && <Skeleton className="h-32" />}
          <HiddenNote hidden={spec} />
          {whole === false && <FilesByStats thread={thread} stats={filtered(stats.data ?? [], filter)} of={spec} commenting={commenting} />}
          {whole && files.length === 0 && !diff.isPending && <p className="text-sm text-fg-2">No changes.</p>}
          {whole && shown.length > 0 && (
            <DiffView
              files={shown}
              layout={kept.layout}
              viewFiles={thread}
              commenting={commenting}
              anchors
              picked={cursor?.path}
              cursor={cursor}
              ask={ask}
              fresh={(path) => fresh.get(path)}
              hunkNote={(file, _, index) => <TurnNote thread={thread} turns={turnsOf(file, index)} said={turns.data ?? []} />}
            />
          )}
        </section>
      </div>
    </div>
  )
}

/** The turns a hunk came from, as a link to the first in the conversation. */
function TurnNote({ thread, turns, said }: { thread: string; turns: number[]; said: TurnInfo[] }) {
  const [first] = turns
  if (first === undefined) return null
  const prompt = said.find((turn) => turn.number === first)?.prompt?.split('\n')[0]
  return (
    <Link
      to="/t/$id"
      params={{ id: thread }}
      hash={`turn-${first}`}
      title="Open this turn in the conversation (t)"
      className="inline-flex min-w-0 items-center gap-1 rounded-sm px-1.5 text-accent-fg hover:bg-surface-2"
    >
      <Icon name="comment" className="h-3 w-3 shrink-0" />
      <span className="truncate">{turns.length === 1 ? `turn ${first}${prompt ? ` · ${prompt}` : ''}` : `turns ${turns.join(', ')}`}</span>
    </Link>
  )
}

/** The thread's turns, to see one or a range of them: a click picks a turn, a shift-click the range to it. */
function TurnPicker({
  turns,
  picked,
  reviewed,
  onPick,
}: {
  turns: TurnInfo[]
  picked?: [number, number]
  /** The turn the person reviewed through, marked. */
  reviewed: number | null
  onPick: (turns: [number, number] | undefined) => void
}) {
  if (turns.length === 0) return null
  return (
    <span role="group" aria-label="turns" className="flex min-w-0 items-center gap-1.5 text-2xs text-fg-3">
      <span>Turns</span>
      <span className="flex max-w-[50vw] gap-0.5 overflow-x-auto">
        {turns.map((turn) => {
          const on = picked !== undefined && turn.number >= picked[0] && turn.number <= picked[1]
          return (
            <button
              key={turn.number}
              type="button"
              aria-pressed={on}
              title={`${turn.prompt?.split('\n')[0] ?? `Turn ${turn.number}`}${turn.number === reviewed ? ' · reviewed through here' : ''}`}
              onClick={(event) => {
                const next = pickTurns(picked, turn.number, event.shiftKey)
                onPick(picked && !event.shiftKey && picked[0] === turn.number && picked[1] === turn.number ? undefined : next)
              }}
              className={cx(
                'inline-flex h-5 min-w-[22px] items-center justify-center rounded-sm px-1 font-mono text-2xs',
                on ? 'bg-accent-tint text-accent-fg' : 'text-fg-2 hover:bg-surface-2 hover:text-fg-1',
                turn.number === reviewed && 'underline decoration-ok decoration-2 underline-offset-2',
              )}
            >
              {turn.number}
            </button>
          )
        })}
      </span>
    </span>
  )
}
