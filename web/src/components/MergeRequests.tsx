import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { api, short } from '../api'
import { useTopic } from '../bus'
import { keys, message } from '../query'
import { Conversation } from './Conversation'
import { Diff } from './Diff'

export function MergeRequests() {
  useTopic('mrs')
  const requests = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests })
  return (
    <div>
      <h2 className="mb-2 text-lg font-semibold">Merge requests</h2>
      {requests.error && <p className="text-rose-700">{message(requests.error)}</p>}
      <ul data-testid="merge-requests" className="space-y-1 text-sm">
        {requests.data?.length === 0 && <li className="text-neutral-500">No merge requests.</li>}
        {[...(requests.data ?? [])].reverse().map((request) => (
          <li key={request.id}>
            <Link className="text-sky-700" to="/mr/$id" params={{ id: request.id }}>
              {request.title}
            </Link>{' '}
            <span className="text-xs text-neutral-500">
              {short(request.fork)} → {short(request.target)} · {request.status}
            </span>
          </li>
        ))}
      </ul>
    </div>
  )
}

export function MergeRequest({ id }: { id: string }) {
  useTopic('mrs')
  const details = useQuery({ queryKey: keys.mr(id), queryFn: () => api.mergeRequest(id) })
  const queries = useQueryClient()
  const [error, setError] = useState<string>()
  const [reason, setReason] = useState('')
  const act = (action: Promise<unknown>) =>
    action.then(
      () => queries.invalidateQueries({ queryKey: keys.mrs }),
      (failure: unknown) => setError(message(failure)),
    )
  if (details.error) return <p className="text-rose-700">{message(details.error)}</p>
  if (!details.data) return <p className="text-neutral-500">Loading…</p>
  const { mr, diff, conflicts, conversation } = details.data
  return (
    <div className="space-y-4">
      <div>
        <h2 className="text-lg font-semibold">{mr.title}</h2>
        <p className="text-sm text-neutral-600">
          From fork{' '}
          <Link className="font-mono text-sky-700" to="/t/$id" params={{ id: mr.fork }}>
            {short(mr.fork)}
          </Link>{' '}
          into{' '}
          <Link className="font-mono text-sky-700" to="/t/$id" params={{ id: mr.target }}>
            {short(mr.target)}
          </Link>{' '}
          · <span data-testid="mr-status">{mr.status}</span>
        </p>
        <p className="mt-2 whitespace-pre-wrap text-sm">{mr.summary}</p>
      </div>
      {mr.status === 'open' && (
        <div className="flex flex-wrap items-center gap-2">
          <button className="rounded bg-emerald-700 px-3 py-1.5 text-sm text-white" onClick={() => void act(api.approve(id))}>
            Approve and merge
          </button>
          <input
            aria-label="reason"
            placeholder="reason (optional)"
            value={reason}
            onChange={(event) => setReason(event.target.value)}
            className="rounded border border-neutral-300 px-2 py-1 text-sm"
          />
          <button className="rounded border border-neutral-400 px-3 py-1.5 text-sm" onClick={() => void act(api.decline(id, reason))}>
            Decline
          </button>
          {error && <p className="w-full text-sm text-rose-700">{error}</p>}
        </div>
      )}
      {conflicts.length > 0 && (
        <div data-testid="conflicts" className="rounded border border-amber-300 bg-amber-50 p-3 text-sm">
          <p className="font-semibold">Conflict hints: the merge keeps both sides, so check these</p>
          <ul className="list-disc pl-5">
            {conflicts.map((conflict) => (
              <li key={conflict}>{conflict}</li>
            ))}
          </ul>
        </div>
      )}
      <section>
        <h3 className="mb-1 font-semibold">Changes</h3>
        <Diff text={diff} />
      </section>
      <section>
        <h3 className="mb-1 font-semibold">The fork's conversation</h3>
        <Conversation lines={conversation} />
      </section>
    </div>
  )
}
