import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { api, short, type GraphQuery, type GraphRow } from '../api'
import { layout } from '../lanes'
import { keys, message } from '../query'
import { Button, EmptyState, Select, Skeleton, Tag, cx } from '../ui'

const ROW = 36
const LANE = 16

const KINDS: { value: string; label: string }[] = [
  { value: '', label: 'Everything' },
  { value: 'turn', label: 'Turns' },
  { value: 'commit', label: 'Commits' },
  { value: 'fork,request,merge', label: 'Forks and merges' },
]

const TONE: Record<string, 'neutral' | 'info' | 'accent' | 'ok'> = {
  turn: 'neutral',
  commit: 'info',
  fork: 'accent',
  request: 'accent',
  merge: 'ok',
}

/** The lanes beside the rows: a line per thread, a mark per row, and the curves between threads. */
function Lanes({ rows }: { rows: GraphRow[] }) {
  const { lanes, threads, edges, width } = layout(rows)
  const x = (lane: number) => lane * LANE + LANE / 2
  const y = (index: number) => index * ROW + ROW / 2
  return (
    <svg aria-hidden width={width * LANE} height={rows.length * ROW} className="shrink-0 text-fg-3">
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
        />
      ))}
      {edges.map((edge, index) => (
        <path
          key={index}
          d={`M ${x(edge.from)} ${y(edge.row) + ROW / 2} C ${x(edge.from)} ${y(edge.row)}, ${x(edge.to)} ${y(edge.row) + ROW / 4}, ${x(edge.to)} ${y(edge.row)}`}
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
          strokeDasharray={edge.dashed ? '4 3' : undefined}
          className={edge.kind === 'merge' ? 'text-ok' : 'text-accent'}
        />
      ))}
      {rows.map((row, index) =>
        row.kind === 'commit' ? (
          <rect key={row.id} x={x(lanes[index]) - 4} y={y(index) - 4} width={8} height={8} className="fill-info" />
        ) : (
          <circle key={row.id} cx={x(lanes[index])} cy={y(index)} r={4} className={row.kind === 'turn' ? 'fill-fg-3' : 'fill-accent'} />
        ),
      )}
    </svg>
  )
}

function Details({ row, onClose }: { row: GraphRow; onClose: () => void }) {
  return (
    <aside aria-label="details" className="space-y-2 rounded-md border border-line p-3 text-sm md:w-72 md:shrink-0">
      <div className="flex items-center gap-2">
        <Tag tone={TONE[row.kind] ?? 'neutral'}>{row.kind}</Tag>
        <button className="ml-auto text-xs text-fg-3" onClick={onClose}>
          close
        </button>
      </div>
      <p className="break-words">{row.text}</p>
      <p className="text-xs text-fg-2">
        {row.owner || 'you'}
        {row.agent && <> · {row.agent}</>} · {new Date(Number(row.time_ms)).toLocaleString()}
      </p>
      <ul className="space-y-1 text-xs">
        <li>
          <Link to="/t/$id" params={{ id: row.thread }} hash={row.turn !== null ? `turn-${row.turn}` : undefined} className="text-accent-fg">
            {row.turn !== null ? `Turn ${row.turn} of thread ${short(row.thread)}` : `Thread ${short(row.thread)}`}
          </Link>
        </li>
        {row.other && (
          <li>
            <Link to="/t/$id" params={{ id: row.other }} className="text-accent-fg">
              {row.kind === 'fork' ? 'Its source' : row.kind === 'merge' ? 'The fork merged' : 'The thread it asks to merge into'}{' '}
              {short(row.other)}
            </Link>
          </li>
        )}
        {row.kind === 'request' && (
          <li>
            <Link to="/mr/$id" params={{ id: row.id }} className="text-accent-fg">
              The merge request ({row.status})
            </Link>
          </li>
        )}
      </ul>
    </aside>
  )
}

/** Every thread's turns, commits, forks and merges, newest first, in lanes (the History graph board). */
export function Graph() {
  const [mine, setMine] = useState(false)
  const [kinds, setKinds] = useState('')
  const [settled, setSettled] = useState(false)
  const [before, setBefore] = useState<number[]>([])
  const [chosen, setChosen] = useState<GraphRow>()
  const query: GraphQuery = {
    project: null,
    family: null,
    mine,
    kinds: kinds || null,
    settled,
    before: before.at(-1) ?? null,
    limit: null,
  }
  const graph = useQuery({ queryKey: keys.graph(query), queryFn: () => api.graph(query) })
  const rows = graph.data ?? []
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-3">
        <h2 className="text-lg font-semibold">Graph</h2>
        <label className="flex items-center gap-1 text-sm">
          <input type="checkbox" checked={mine} onChange={(event) => setMine(event.target.checked)} /> Mine
        </label>
        <Select aria-label="what to show" value={kinds} onChange={(event) => setKinds(event.target.value)}>
          {KINDS.map((kind) => (
            <option key={kind.value} value={kind.value}>
              {kind.label}
            </option>
          ))}
        </Select>
        <label className="flex items-center gap-1 text-sm">
          <input type="checkbox" checked={settled} onChange={(event) => setSettled(event.target.checked)} /> Settled too
        </label>
        {before.length > 0 && <Button onClick={() => setBefore(before.slice(0, -1))}>Newer</Button>}
      </div>
      {graph.isPending && <Skeleton className="h-40" />}
      {graph.error && <p className="text-danger">{message(graph.error)}</p>}
      {graph.isSuccess && rows.length === 0 && <EmptyState title="Nothing here yet">Turns, commits and forks show up as they happen.</EmptyState>}
      <div className="flex flex-col gap-4 md:flex-row">
        <div className="flex min-w-0 flex-1 overflow-x-auto">
          <Lanes rows={rows} />
          <ul aria-label="graph" className="min-w-0 flex-1">
            {rows.map((row) => (
              <li key={row.id} style={{ height: ROW }}>
                <button
                  className={cx(
                    'flex h-full w-full items-center gap-2 rounded-sm px-2 text-left text-sm hover:bg-surface-2',
                    chosen?.id === row.id && 'bg-accent-tint',
                  )}
                  onClick={() => setChosen(row)}
                >
                  <Tag tone={TONE[row.kind] ?? 'neutral'}>{row.kind === 'turn' ? `turn ${row.turn}` : row.kind}</Tag>
                  <span className="min-w-0 flex-1 truncate">{row.text}</span>
                  <span className="shrink-0 font-mono text-[11px] text-fg-3">{short(row.thread)}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
        {chosen && <Details row={chosen} onClose={() => setChosen(undefined)} />}
      </div>
      {rows.length >= 200 && <Button onClick={() => setBefore([...before, Number(rows[rows.length - 1].time_ms)])}>Older</Button>}
    </div>
  )
}
