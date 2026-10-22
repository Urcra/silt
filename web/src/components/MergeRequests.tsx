import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useParams, useRouterState } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { api, short, type MrFile, type MrGuide } from '../api'
import { useTopic } from '../bus'
import { keys, message } from '../query'
import { Banner, Button, Input, Textarea } from '../ui'
import { Conversation } from './Conversation'
import { CopyLink } from './CopyLink'
import { Diff } from './Diff'

function safeDecode(text: string): string {
  try {
    return decodeURIComponent(text)
  } catch {
    return text
  }
}

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
              {short(request.fork)} → {short(request.target)} · {request.changes_requested ? 'changes requested' : request.status}
              {request.updated_since && ', updated since'}
            </span>
          </li>
        ))}
      </ul>
    </div>
  )
}

/** A walk-through for reviewers: written by the fork's agent when asked, or by hand (ADR 0025). */
function ChangeGuide({ id, guide, act }: { id: string; guide: MrGuide | null; act: (action: Promise<unknown>) => Promise<unknown> }) {
  const [writing, setWriting] = useState(false)
  const [text, setText] = useState('')
  const [asked, setAsked] = useState(false)
  return (
    <section aria-label="change guide" className="rounded-md border border-line p-3 text-sm">
      <div className="mb-1 flex items-center gap-2">
        <h3 className="font-semibold">Change guide</h3>
        {guide && <span className="text-xs text-fg-3">by {guide.by}</span>}
        <span className="ml-auto flex gap-2">
          <Button
            onClick={() => {
              setAsked(true)
              void act(api.writeGuide(id))
            }}
            title="The agent running in the fork writes it, on this machine"
          >
            Ask the fork's agent
          </Button>
          <Button variant="ghost" onClick={() => setWriting(!writing)}>
            Write it
          </Button>
        </span>
      </div>
      {guide ? (
        <p data-testid="guide" className="whitespace-pre-wrap">
          {guide.text}
        </p>
      ) : (
        <p className="text-xs text-fg-3">{asked ? 'The fork’s agent is writing it…' : 'No guide yet.'}</p>
      )}
      {writing && (
        <form
          className="mt-2 space-y-2"
          onSubmit={(event) => {
            event.preventDefault()
            void act(api.writeGuide(id, text)).then(() => {
              setWriting(false)
              setText('')
            })
          }}
        >
          <Textarea aria-label="guide" rows={5} value={text} onChange={(event) => setText(event.target.value)} />
          <Button type="submit" variant="primary" disabled={!text.trim()}>
            Keep guide
          </Button>
        </form>
      )}
    </section>
  )
}

export function MergeRequestPage() {
  const { id } = useParams({ from: '/mr/$id' })
  return <MergeRequest id={id} />
}

export function MergeRequest({ id }: { id: string }) {
  useTopic('mrs')
  const details = useQuery({ queryKey: keys.mr(id), queryFn: () => api.mergeRequest(id) })
  const queries = useQueryClient()
  const [error, setError] = useState<string>()
  const [reason, setReason] = useState('')
  const [ticked, setTicked] = useState<Record<string, boolean>>({})
  const hash = useRouterState({ select: (state) => state.location.hash }).replace(/^#/, '')
  const picked = /^file-(.+)$/.exec(safeDecode(hash))?.[1]
  const loaded = details.data !== undefined
  useEffect(() => {
    if (loaded && picked) document.getElementById(`file-${picked}`)?.scrollIntoView({ block: 'start' })
  }, [loaded, picked])
  const act = (action: Promise<unknown>) =>
    action.then(
      () => queries.invalidateQueries({ queryKey: keys.mrs }),
      (failure: unknown) => setError(message(failure)),
    )
  if (details.error) return <p className="text-danger">{message(details.error)}</p>
  if (!details.data) return <p className="text-fg-3">Loading…</p>
  const { mr, diff, conflicts, conversation, files } = details.data
  // A tick shows at once, before the daemon answers.
  const seenNow = (file: MrFile) => ticked[file.path] ?? file.seen
  const seenCount = files.filter(seenNow).length
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
      <ChangeGuide id={id} guide={details.data.guide} act={act} />
      {mr.changes_requested && (
        <div data-testid="changes-requested">
          <Banner tone={mr.updated_since ? 'info' : 'warn'}>
            <p className="font-medium">
              Changes requested{mr.updated_since ? '; the fork has moved on since' : ''}
            </p>
            <p>{mr.changes_requested}</p>
          </Banner>
        </div>
      )}
      {mr.status === 'open' && (
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="primary" icon="check" onClick={() => void act(api.approve(id))}>
            Approve and merge
          </Button>
          <Input aria-label="reason" placeholder="reason (optional to decline)" value={reason} onChange={(event) => setReason(event.target.value)} />
          <Button
            disabled={!reason.trim()}
            title="Keeps it open and tells the fork what to change"
            onClick={() => void act(api.requestChanges(id, reason).then(() => setReason('')))}
          >
            Request changes
          </Button>
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
      {files.length > 0 && (
        <section aria-label="files">
          <div className="mb-1 flex items-center gap-3">
            <h3 className="font-semibold">Files</h3>
            <span data-testid="seen-count" className="text-xs text-fg-2">
              {seenCount}/{files.length} seen
            </span>
            <span className="h-1.5 w-32 overflow-hidden rounded-full bg-surface-2" aria-hidden="true">
              <span className="block h-full bg-ok" style={{ width: `${(100 * seenCount) / files.length}%` }} />
            </span>
          </div>
          <ul className="space-y-0.5 text-sm">
            {files.map((file) => (
              <li key={file.path} className={`flex items-center gap-3 ${file.path === picked ? 'rounded bg-accent-tint' : ''}`}>
                <label className="flex items-center gap-2 font-mono text-xs">
                  <input
                    type="checkbox"
                    aria-label={`seen ${file.path}`}
                    checked={seenNow(file)}
                    onChange={(event) => {
                      const seen = event.target.checked
                      setTicked((before) => ({ ...before, [file.path]: seen }))
                      void act(api.markSeen(id, file.path, seen))
                    }}
                  />
                  {file.path}
                </label>
                <Link className="text-xs text-accent-fg" to="/mr/$id" params={{ id }} hash={`file-${file.path}`}>
                  diff
                </Link>
                <CopyLink path={`/mr/${id}#file-${file.path}`} thread={mr.target} label="link" />
              </li>
            ))}
          </ul>
        </section>
      )}
      <section>
        <h3 className="mb-1 font-semibold">Changes</h3>
        <Diff text={diff} anchors picked={picked} />
      </section>
      <section>
        <h3 className="mb-1 font-semibold">The fork's conversation</h3>
        <Conversation lines={conversation} />
      </section>
    </div>
  )
}
