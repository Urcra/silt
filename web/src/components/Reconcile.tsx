import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate, useParams } from '@tanstack/react-router'
import { useState } from 'react'
import { api, short, type HistoryEntry, type ThreadSummary } from '../api'
import { useNow } from '../now'
import { keys, message } from '../query'
import { ago } from '../time'
import { toolOf } from '../transcript'
import { Button, Dot, EmptyState, Icon, Input, KeyValue, Skeleton, buttonClass, cx, toast } from '../ui'
import { Mock } from './Mock'
import { Page } from './Page'

/** The last few entries of one side, as the board lists them. */
function Entries({ entries, numbered }: { entries: HistoryEntry[]; numbered: 'index' | 'added' }) {
  if (entries.length === 0) return <p className="text-xs text-fg-2">Nothing on this side.</p>
  return (
    <ol className="flex flex-col gap-1.5 text-xs">
      {entries.map((entry, at) => (
        <li key={entry.index} className="grid grid-cols-[32px_minmax(0,1fr)] gap-2">
          <span className="text-right font-mono text-2xs text-fg-2">{numbered === 'index' ? `#${entry.index}` : `+${at + 1}`}</span>
          <span className={cx('line-clamp-2', entry.kind === 'user' ? 'font-medium' : entry.kind === 'assistant' ? '' : 'font-mono text-2xs text-fg-2')}>
            {entry.kind === 'user'
              ? `› ${entry.text}`
              : entry.kind === 'tool_call'
                ? [toolOf(entry.text).name, toolOf(entry.text).file].filter(Boolean).join(' ')
                : entry.text}
          </span>
        </li>
      ))}
    </ol>
  )
}

/**
 * A thread whose copy here differed from its server's (ADR 0009, the
 * Reconcile board): the server's version, kept, beside this machine's work,
 * moved to a recovery thread, and what to do with it.
 */
export function Reconcile() {
  const { id } = useParams({ from: '/t/$id/reconcile' })
  const now = useNow()
  const navigate = useNavigate()
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const found = status.data?.recoveries?.find((recovery) => recovery.recovery.endsWith(id) || recovery.thread.endsWith(id))
  const original = useQuery({ queryKey: keys.history(found?.thread ?? ''), queryFn: () => api.history(found!.thread), enabled: found !== undefined })
  const moved = useQuery({ queryKey: keys.history(found?.recovery ?? ''), queryFn: () => api.history(found!.recovery), enabled: found !== undefined })
  const [title, setTitle] = useState('Bring back this machine’s work')
  const [offering, setOffering] = useState(false)
  const summary = (thread: string | undefined): ThreadSummary | undefined => threads.data?.find((candidate) => candidate.id === thread)
  const device = status.data?.servers?.[0]?.name ?? 'this machine'

  if (status.isPending) return <Skeleton className="m-6 h-40" />
  if (status.error) return <p className="p-6 text-danger">{message(status.error)}</p>
  if (!found)
    return (
      <Page title="Reconcile a diverged thread" width="narrow">
        <EmptyState title="No recovery of this thread is known here">
          When this machine’s copy of a thread differs from its server’s, its work moves to a recovery thread, and this page compares the two. The
          daemon remembers recoveries until it restarts; <code className="font-mono">silt auth status</code> lists them.
        </EmptyState>
      </Page>
    )

  const kept = summary(found.thread)
  const recovery = summary(found.recovery)
  // Tool output says little out of its turn, so the sides show what was said and done.
  const said = (entry: HistoryEntry) => entry.kind !== 'tool_result' && entry.kind !== 'thinking'
  const keptEntries = (original.data?.entries ?? []).filter(said).slice(-3)
  const movedEntries = (moved.data?.entries ?? []).filter((entry) => entry.origin === 'own' && said(entry)).slice(0, 3)
  const offer = () =>
    api.offerMergeRequest(found.recovery, title).then(
      (mr) => void navigate({ to: '/mr/$id', params: { id: mr.id } }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const setAside = () =>
    api.setThreadState(found.recovery, { pinned: null, snooze: null, settled: true, seen: false, reviewed_turn: null }).then(
      () => toast('The recovery thread is settled; its work stays in its history', 'ok'),
      (failure: unknown) => toast(message(failure), 'danger'),
    )

  return (
    <Page flush label="reconcile" title="Reconcile" meta="this machine’s copy differed from the server’s">
      <div className="flex max-w-[1100px] flex-col gap-5 px-4 py-6 md:px-6">
        <div className="flex flex-col gap-2">
          <p className="max-w-[760px] text-sm leading-relaxed text-fg-1">
            Silt’s data on <b className="font-medium">{device}</b> no longer matched the server, as happens after restoring from an old backup. The
            server’s version of <span className="font-mono" data-volatile>{short(found.thread)}</span> is kept. Nothing from this machine was lost: its work now
            continues in a recovery thread forked from the server’s version, made <span data-volatile>{ago(found.time_ms, now)}</span> ago.
          </p>
        </div>
        <div className="grid gap-4 md:grid-cols-2">
          <section aria-labelledby="kept" className="flex flex-col gap-3 rounded-md border border-line bg-elevated p-4 [&>dl]:border-t [&>dl]:border-line-subtle [&>dl]:pt-3">
            <div className="-mx-4 flex items-center gap-2 border-b border-line-subtle px-4 pb-3">
              <Dot tone="ok" />
              <h2 id="kept" className="text-sm font-medium">
                Server’s version, kept
              </h2>
              <Link to="/t/$id" params={{ id: found.thread }} className="ml-auto font-mono text-2xs text-accent-fg">
                <span data-volatile>{short(found.thread)}</span>
              </Link>
            </div>
            {original.isPending ? <Skeleton className="h-16" /> : <Entries entries={keptEntries} numbered="index" />}
            <KeyValue
              rows={[
                [
                  'Last write',
                  kept ? (
                    <span key="last" data-volatile>
                      {ago(kept.last_ms, now)}
                    </span>
                  ) : (
                    '—'
                  ),
                ],
                ['Entries', String(original.data?.total ?? '—')],
              ]}
            />
          </section>
          <section
            aria-labelledby="moved"
            className="flex flex-col gap-3 rounded-md border border-accent bg-elevated p-4 [&>dl]:border-t [&>dl]:border-line-subtle [&>dl]:pt-3"
          >
            <div className="-mx-4 flex items-center gap-2 border-b border-line-subtle px-4 pb-3">
              <Icon name="fork" className="text-accent-fg" />
              <h2 id="moved" className="text-sm font-medium">
                Work only on {device}, moved to a recovery thread
              </h2>
              <Link to="/t/$id" params={{ id: found.recovery }} className="ml-auto font-mono text-2xs text-accent-fg">
                <span data-volatile>{short(found.recovery)}</span>
              </Link>
            </div>
            {moved.isPending ? <Skeleton className="h-16" /> : <Entries entries={movedEntries} numbered="added" />}
            <KeyValue
              rows={[
                ['Entries', String(movedEntries.length === 0 ? 0 : (moved.data?.entries ?? []).filter((entry) => entry.origin === 'own').length)],
                [
                  'Checkout',
                  <Mock key="checkout" id="reconcile.checkout">
                    the clone on {device} now follows <span data-volatile>{short(found.recovery)}</span>
                  </Mock>,
                ],
                [
                  'Agent',
                  <Mock key="agent" id="reconcile.agent">
                    {recovery?.agent ?? 'its'} session moved with it
                  </Mock>,
                ],
              ]}
            />
          </section>
        </div>
        <section aria-label="what next" className="flex flex-col gap-3 rounded-md border border-line bg-surface-1 p-4">
          <span className="text-sm">Bring this machine’s work back with a merge request, or keep working in the recovery thread.</span>
          {offering && (
            <Input aria-label="merge request title" value={title} onChange={(event) => setTitle(event.target.value)} className="max-w-md" />
          )}
          <span className="flex flex-wrap gap-2">
            <Button variant="primary" icon="merge" onClick={() => (offering ? void offer() : setOffering(true))} disabled={offering && !title.trim()}>
              Open merge request into <span className="font-mono" data-volatile>{short(found.thread)}</span>
            </Button>
            <Link to="/t/$id" params={{ id: found.recovery }} className={buttonClass('secondary')}>
              Open recovery thread
            </Link>
            <Button variant="ghost" onClick={() => void setAside()} title="Settles the recovery thread; nothing is deleted">
              Set this machine’s work aside
            </Button>
          </span>
        </section>
      </div>
    </Page>
  )
}
