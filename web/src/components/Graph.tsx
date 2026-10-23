import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { api, short, type GraphQuery, type GraphRow, type ThreadSummary } from '../api'
import { parseFiles } from '../diffParse'
import { useTopic } from '../bus'
import { layout } from '../lanes'
import { useMedia } from '../media'
import { plain } from '../plain'
import { useNow } from '../now'
import { keys, message } from '../query'
import { hosted } from '../served'
import { threadTone } from '../threadLook'
import { ago } from '../time'
import { Button, Counts, Dot, EmptyState, Icon, Input, KeyValue, Skeleton, Tabs, buttonClass, cx, toast } from '../ui'
import { CopyLink } from './CopyLink'
import { Markdown } from './Markdown'
import { Mock } from './Mock'
import { Page } from './Page'

const LANE = 16

/** Each lane's colour, in turn, as the board tells threads apart. */
const LANE_COLOUR = ['text-accent', 'text-info', 'text-ok']

/**
 * The lanes beside the rows, as the board encodes them: a line per thread in
 * its lane's colour (dashed when abandoned), a ring per turn, a filled dot
 * per commit, and the curves between threads.
 */
function Lanes({ rows, height }: { rows: GraphRow[]; height: number }) {
  const { lanes, threads, edges, width } = layout(rows)
  const x = (lane: number) => lane * LANE + LANE / 2
  const y = (index: number) => index * height + height / 2
  const colour = (lane: number) => LANE_COLOUR[lane % LANE_COLOUR.length]
  return (
    <svg aria-hidden width={Math.max(width, 1) * LANE + 8} height={rows.length * height} className="shrink-0">
      {[...threads.values()].map((thread, index) => (
        <line
          key={index}
          x1={x(thread.lane)}
          x2={x(thread.lane)}
          y1={y(thread.top)}
          y2={y(thread.bottom)}
          stroke="currentColor"
          strokeWidth={2}
          strokeDasharray={thread.abandoned ? '3 3' : undefined}
          className={thread.abandoned ? 'text-fg-3' : colour(thread.lane)}
        />
      ))}
      {edges.map((edge, index) => (
        <path
          key={index}
          d={`M ${x(edge.from)} ${y(edge.row) + height / 2} C ${x(edge.from)} ${y(edge.row)}, ${x(edge.to)} ${y(edge.row) + height / 4}, ${x(edge.to)} ${y(edge.row)}`}
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
          strokeDasharray={edge.dashed ? '4 3' : undefined}
          className={edge.kind === 'merge' ? 'text-ok' : 'text-accent'}
        />
      ))}
      {rows.map((row, index) =>
        row.kind === 'commit' ? (
          <circle key={row.id} cx={x(lanes[index])} cy={y(index)} r={5} fill="currentColor" className={colour(lanes[index])} />
        ) : (
          <circle
            key={row.id}
            cx={x(lanes[index])}
            cy={y(index)}
            r={4}
            fill="var(--bg)"
            stroke="currentColor"
            strokeWidth={2}
            className={row.kind === 'turn' ? colour(lanes[index]) : 'text-accent'}
          />
        ),
      )}
    </svg>
  )
}

/** A row's tag, before its description, for what isn't a plain turn. */
function KindTag({ row }: { row: GraphRow }) {
  const tag = (text: string, tone: string) => <span className={cx('rounded-sm px-1.5 text-2xs whitespace-nowrap', tone)}>{text}</span>
  if (row.kind === 'commit' && row.commit)
    return (
      <span className="rounded-sm border border-line bg-surface-1 px-1 font-mono text-2xs text-fg-1" data-volatile>
        {row.commit.slice(0, 7)}
      </span>
    )
  if (row.kind === 'request') return tag(row.status === 'open' ? 'merge request' : `merge request · ${row.status}`, 'bg-accent-tint text-accent-fg')
  if (row.kind === 'merge') return tag('merged', 'bg-ok-tint text-ok')
  if (row.kind === 'fork') return tag('forked', 'bg-surface-2 text-fg-2')
  return null
}

/** The chosen row, in full, with what can be done from it (the Graph board's panel). */
function Details({ row, thread }: { row: GraphRow; thread: ThreadSummary | undefined }) {
  const now = useNow()
  const navigate = useNavigate()
  const queries = useQueryClient()
  const turnDiff = useQuery({
    queryKey: keys.diff(row.thread, { turn: row.turn ?? 0 }),
    queryFn: () => api.diff(row.thread, { turn: row.turn! }),
    enabled: row.turn !== null && !hosted(),
    staleTime: Infinity,
  })
  const files = turnDiff.data === undefined ? null : parseFiles(turnDiff.data)
  const failed = (failure: unknown) => toast(message(failure), 'danger')
  return (
    <aside aria-label="details" className="flex flex-col gap-4 px-5 py-5">
      <div className="flex flex-col gap-1.5">
        <span className="flex items-center gap-2">
          <KindTag row={row} />
          {row.turn !== null && <span className="font-mono text-2xs text-fg-2">turn {row.turn}</span>}
        </span>
        <h2 className="text-lg font-medium break-words">{plain(row.text)}</h2>
      </div>
      <KeyValue
        rows={[
          ['Thread', thread?.title ?? short(row.thread)],
          ['By', [row.owner || 'you', row.agent && `with ${row.agent}`].filter(Boolean).join(' ')],
          [
            'On',
            <span key="on">
              <Mock id="graph.device">workstation · </Mock>
              <span data-volatile>{ago(Number(row.time_ms), now)}</span>
            </span>,
          ],
          ...(row.kind === 'request' && row.status ? ([['Status', row.status]] as [string, string][]) : []),
        ]}
      />
      {row.kind === 'turn' && (
        <section className="flex flex-col gap-1.5">
          <h3 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">Prompt</h3>
          <Markdown text={row.text} className="text-xs" />
        </section>
      )}
      {row.turn !== null && (
        <section className="flex flex-col gap-1.5">
          <h3 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">Files</h3>
          {files === null && !hosted() && <Skeleton className="h-8" />}
          {files?.length === 0 && <p className="text-xs text-fg-2">No files changed.</p>}
          <ul className="flex flex-col gap-0.5 font-mono text-2xs">
            {files?.map((file) => (
              <li key={file.path} className="flex gap-2">
                <span className="min-w-0 flex-1 truncate">{file.path}</span>
                <Counts added={file.added} removed={file.removed} />
              </li>
            ))}
          </ul>
        </section>
      )}
      <div className="flex flex-wrap gap-1.5">
        <Link
          to="/t/$id"
          params={{ id: row.thread }}
          hash={row.turn !== null ? `turn-${row.turn}` : undefined}
          className={buttonClass('primary')}
          aria-label={row.turn !== null ? `Turn ${row.turn} of thread ${short(row.thread)}` : `Thread ${short(row.thread)}`}
        >
          {row.turn !== null ? 'Open at this turn' : 'Open the thread'}
        </Link>
        {row.kind === 'request' && (
          <Link to="/mr/$id" params={{ id: row.id }} className={buttonClass('secondary')}>
            The merge request
          </Link>
        )}
        {row.other && row.kind !== 'request' && (
          <Link to="/t/$id" params={{ id: row.other }} className={buttonClass('secondary')}>
            {row.kind === 'fork' ? 'Its source' : 'The fork merged'}
          </Link>
        )}
        {row.turn !== null && !hosted() && (
          <>
            <Button
              icon="fork"
              onClick={() => api.fork(row.thread, row.turn!).then((made) => void navigate({ to: '/t/$id', params: { id: made.thread } }), failed)}
            >
              Fork from here
            </Button>
            <Button
              icon="revert"
              onClick={() =>
                api.revert(row.thread, row.turn!).then((reverted) => {
                  toast(`Reverted to turn ${reverted.turn}: ${reverted.files.length} files written back`)
                  void queries.invalidateQueries()
                }, failed)
              }
            >
              Revert to here
            </Button>
          </>
        )}
        {row.turn !== null && (
          <CopyLink path={`/t/${row.thread}#turn-${row.turn}`} thread={row.thread} label="Copy link" className={buttonClass('ghost')} />
        )}
      </div>
    </aside>
  )
}

type Which = 'all' | 'mine' | 'family'

/** Every thread's turns, commits, forks and merges, newest first, in lanes (the History graph board). */
export function Graph() {
  useTopic('threads')
  useTopic('mrs')
  const now = useNow()
  const [which, setWhich] = useState<Which>('all')
  const [turns, setTurns] = useState(true)
  const [commits, setCommits] = useState(true)
  const [settled, setSettled] = useState(false)
  const [text, setText] = useState('')
  const [before, setBefore] = useState<number[]>([])
  const [chosen, setChosen] = useState<GraphRow>()
  const kinds = [turns && 'turn', commits && 'commit', 'fork', 'request', 'merge'].filter(Boolean).join(',')
  const query: GraphQuery = {
    project: null,
    family: which === 'family' && chosen ? chosen.family : null,
    mine: which === 'mine',
    kinds,
    settled,
    before: before.at(-1) ?? null,
    limit: null,
  }
  const graph = useQuery({ queryKey: keys.graph(query), queryFn: () => api.graph(query) })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const threadOf = (id: string) => threads.data?.find((thread) => thread.id === id)
  const wanted = text.trim().toLowerCase()
  const rows = (graph.data ?? []).filter(
    (row) =>
      !wanted ||
      row.text.toLowerCase().includes(wanted) ||
      row.owner.toLowerCase().includes(wanted) ||
      (threadOf(row.thread)?.title ?? '').toLowerCase().includes(wanted),
  )
  const picked = chosen && rows.find((row) => row.id === chosen.id) ? chosen : rows[0]
  const grid = 'grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 md:grid-cols-[minmax(0,1fr)_200px_140px_56px]'
  // A phone's rows are tall enough to tap; the lanes follow them.
  const rowHeight = useMedia('(max-width: 767px)') ? 44 : 36
  return (
    <Page
      flush
      label="history graph"
      title="History graph"
      inspector={picked ? <Details row={picked} thread={threadOf(picked.thread)} /> : undefined}
      inspectorLabel="selected"
    >
      <div className="flex min-h-10 flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-line px-4 py-1.5 md:px-6">
        <Tabs
          label="which threads"
          value={which}
          onChange={setWhich}
          tabs={[
            { id: 'all', label: 'All threads' },
            { id: 'mine', label: 'Mine' },
            { id: 'family', label: 'This thread’s family' },
          ]}
        />
        <span className="flex flex-wrap items-center gap-x-3">
          {[
            ['Turns', turns, setTurns],
            ['Commits', commits, setCommits],
            ['Settled threads', settled, setSettled],
          ].map(([label, on, set]) => (
            <label key={label as string} className="flex items-center gap-1.5 text-xs text-fg-2 max-md:min-h-11">
              <input
                type="checkbox"
                checked={on as boolean}
                onChange={(event) => (set as (value: boolean) => void)(event.target.checked)}
                className="accent-[var(--accent)] max-md:h-5 max-md:w-5"
              />
              {label as string}
            </label>
          ))}
        </span>
        <Input
          aria-label="filter history"
          placeholder="Filter by text or author"
          value={text}
          onChange={(event) => setText(event.target.value)}
          className="text-xs max-md:w-full md:ml-auto md:w-56"
        />
      </div>
      <div className="px-4 pt-2 pb-6 md:px-6">
        {graph.isPending && <Skeleton className="h-40" />}
        {graph.error && <p className="text-danger">{message(graph.error)}</p>}
        {graph.isSuccess && rows.length === 0 && (wanted ? (
          <EmptyState title={`No history matches “${text.trim()}”`}>
            <Button variant="ghost" onClick={() => setText('')}>
              Clear filter
            </Button>
          </EmptyState>
        ) : (
          <EmptyState title="Nothing here yet">Turns, commits and forks show up as they happen.</EmptyState>
        ))}
        {rows.length > 0 && (
          <div className="flex min-w-0 overflow-x-auto">
            <div className="shrink-0">
              <div className="flex h-7 items-center border-b border-line pl-1 font-mono text-2xs tracking-[0.04em] text-fg-2 uppercase">Graph</div>
              <Lanes rows={rows} height={rowHeight} />
            </div>
            <div className="min-w-0 flex-1">
              <div className={cx(grid, 'h-7 border-b border-line px-2 font-mono text-2xs tracking-[0.04em] text-fg-2 uppercase')}>
                <span>Description</span>
                <span className="max-md:hidden">Thread</span>
                <span className="max-md:hidden">Who</span>
                <span className="text-right">When</span>
              </div>
              <ul aria-label="graph">
                {rows.map((row) => {
                  const thread = threadOf(row.thread)
                  return (
                    <li key={row.id} style={{ height: rowHeight }}>
                      <button
                        type="button"
                        aria-label={`${row.kind} ${row.text}`}
                        aria-current={picked?.id === row.id || undefined}
                        className={cx(grid, 'h-full w-full rounded-sm px-2 text-left text-sm hover:bg-surface-2', picked?.id === row.id && 'bg-accent-tint')}
                        onClick={() => setChosen(row)}
                      >
                        <span className="flex min-w-0 items-center gap-2">
                          <KindTag row={row} />
                          <span className={cx('min-w-0 truncate', row.kind === 'fork' && 'font-mono text-xs')}>{plain(row.text)}</span>
                          {row.turn !== null && <span className="shrink-0 font-mono text-2xs text-fg-2">turn {row.turn}</span>}
                          {thread && ['awaiting_permission', 'awaiting_input'].includes(thread.agent_state ?? '') && row.kind === 'turn' && (
                            <span className="shrink-0 rounded-sm bg-warn-tint px-1.5 text-2xs">waiting on you</span>
                          )}
                        </span>
                        <span className="flex min-w-0 items-center gap-1.5 text-xs max-md:hidden">
                          {thread && <Dot tone={threadTone(thread)} />}
                          <span className="truncate">{thread?.title ?? short(row.thread)}</span>
                        </span>
                        <span className="truncate font-mono text-2xs text-fg-2 max-md:hidden">
                          {[row.owner || 'you', row.agent].filter(Boolean).join(' · ')}
                        </span>
                        <span className="text-right font-mono text-2xs text-fg-2" data-volatile>
                          {ago(Number(row.time_ms), now)}
                        </span>
                      </button>
                    </li>
                  )
                })}
              </ul>
            </div>
          </div>
        )}
        <div className="mt-3 flex gap-2">
          {before.length > 0 && (
            <Button variant="ghost" onClick={() => setBefore(before.slice(0, -1))}>
              <Icon name="chevronDown" className="rotate-180" />
              Newer history
            </Button>
          )}
          {(graph.data?.length ?? 0) >= 200 && (
            <Button variant="ghost" onClick={() => setBefore([...before, Number(rows[rows.length - 1].time_ms)])}>
              <Icon name="chevronDown" />
              Load older history
            </Button>
          )}
        </div>
      </div>
    </Page>
  )
}
