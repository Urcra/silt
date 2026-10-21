import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { api, short } from '../api'
import { useTopic } from '../bus'
import { keys, message } from '../query'
import { Banner, Button, Input } from '../ui'
import { Conversation } from './Conversation'
import { Diff } from './Diff'

export function MergeRequests() {
  useTopic('mrs')
  const requests = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests })
  return (
    <div>
      <h2 className="mb-2 text-lg font-semibold">Merge requests</h2>
      {requests.error && <p className="text-danger">{message(requests.error)}</p>}
      <ul data-testid="merge-requests" className="space-y-1 text-sm">
        {requests.data?.length === 0 && <li className="text-fg-3">No merge requests.</li>}
        {[...(requests.data ?? [])].reverse().map((request) => (
          <li key={request.id}>
            <Link className="text-accent-fg" to="/mr/$id" params={{ id: request.id }}>
              {request.title}
            </Link>{' '}
            <span className="text-xs text-fg-3">
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
  if (details.error) return <p className="text-danger">{message(details.error)}</p>
  if (!details.data) return <p className="text-fg-3">Loading…</p>
  const { mr, diff, conflicts, conversation } = details.data
  return (
    <div className="space-y-4">
      <div>
        <h2 className="text-lg font-semibold">{mr.title}</h2>
        <p className="text-sm text-fg-2">
          From fork{' '}
          <Link className="font-mono text-accent-fg" to="/t/$id" params={{ id: mr.fork }}>
            {short(mr.fork)}
          </Link>{' '}
          into{' '}
          <Link className="font-mono text-accent-fg" to="/t/$id" params={{ id: mr.target }}>
            {short(mr.target)}
          </Link>{' '}
          · <span data-testid="mr-status">{mr.status}</span>
        </p>
        <p className="mt-2 whitespace-pre-wrap text-sm">{mr.summary}</p>
      </div>
      {mr.status === 'open' && (
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="primary" icon="check" onClick={() => void act(api.approve(id))}>
            Approve and merge
          </Button>
          <Input aria-label="reason" placeholder="reason (optional)" value={reason} onChange={(event) => setReason(event.target.value)} />
          <Button variant="danger" onClick={() => void act(api.decline(id, reason))}>
            Decline
          </Button>
          {error && <p className="w-full text-sm text-danger">{error}</p>}
        </div>
      )}
      {conflicts.length > 0 && (
        <div data-testid="conflicts">
          <Banner tone="warn">
            <p className="font-medium">Conflict hints: the merge keeps both sides, so check these</p>
            <ul className="list-disc pl-5 font-mono text-xs">
              {conflicts.map((conflict) => (
                <li key={conflict}>{conflict}</li>
              ))}
            </ul>
          </Banner>
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
