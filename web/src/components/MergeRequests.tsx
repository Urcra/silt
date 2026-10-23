import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useParams, useRouterState } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { api, short, type MrFile, type MrGuide, type MrSummary, type ShowLine } from '../api'
import { useTopic } from '../bus'
import { parseFiles } from '../diffParse'
import { WIDE, useMedia } from '../media'
import { useNow } from '../now'
import { keys, message } from '../query'
import { linesOf, reviewPrompt } from '../review'
import { ago } from '../time'
import { Avatar, Button, Counts, Dot, type DotTone, EmptyState, Icon, Input, Skeleton, Tabs, Textarea, buttonClass, cx, toast } from '../ui'
import { CommentThread } from './CommentThread'
import { Conversation } from './Conversation'
import { CopyLink } from './CopyLink'
import { type Commenting, type DiffLayout, DiffView } from './DiffView'
import { Markdown } from './Markdown'
import { Page } from './Page'

/** The messages a conversation holds: its prompts and the agent's answers. */
function messageCount(lines: ShowLine[]): number {
  return lines.filter((line) => line.line === 'entry' && (line.kind === 'user' || line.kind === 'assistant')).length
}

function safeDecode(text: string): string {
  try {
    return decodeURIComponent(text)
  } catch {
    return text
  }
}

/** How a merge request stands, for its chip and dot. */
function standing(mr: MrSummary): { label: string; tone: DotTone } {
  if (mr.status === 'approved') return { label: 'Merged', tone: 'ok' }
  if (mr.status.startsWith('declined')) return { label: 'Declined', tone: 'idle' }
  if (mr.changes_requested) return { label: mr.updated_since ? 'Updated since changes asked' : 'Changes requested', tone: 'warn' }
  return { label: 'Open', tone: 'accent' }
}

export function MergeRequests() {
  useTopic('mrs')
  const now = useNow()
  const requests = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const titleOf = (id: string) => threads.data?.find((thread) => thread.id === id)?.title ?? short(id)
  const ownerOf = (id: string) => threads.data?.find((thread) => thread.id === id)?.owner
  const shown = [...(requests.data ?? [])].reverse()
  const open = shown.filter((mr) => mr.status === 'open')
  const closed = shown.filter((mr) => mr.status !== 'open')
  const list = (title: string, items: MrSummary[]) =>
    items.length > 0 && (
      <section aria-label={title.toLowerCase()} className="flex flex-col gap-2">
        <h2 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">
          {title} · {items.length}
        </h2>
        <ul className="overflow-hidden rounded-md border border-line bg-elevated">
          {items.map((mr) => {
            const state = standing(mr)
            return (
              <li key={mr.id} className="flex items-center gap-3 border-b border-line-subtle px-3 py-2 last:border-b-0 hover:bg-surface-2 max-md:min-h-14">
                <Dot tone={state.tone} label={state.label} />
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <Link className="truncate font-medium text-fg-1 hover:text-accent-fg" to="/mr/$id" params={{ id: mr.id }}>
                    {mr.title}
                  </Link>
                  <span className="truncate font-mono text-2xs text-fg-2">
                    {[ownerOf(mr.fork), `into ${titleOf(mr.target)}`, state.label].filter(Boolean).join(' · ')}
                    {' · '}
                    <span data-volatile>{ago(Number(mr.opened_ms), now)}</span>
                  </span>
                </div>
              </li>
            )
          })}
        </ul>
      </section>
    )
  return (
    <Page title="Merge requests" meta={`${open.length} open`} width="wide">
      {requests.error && <p className="text-danger">{message(requests.error)}</p>}
      {requests.isPending && <Skeleton className="h-24" />}
      <div data-testid="merge-requests" className="flex flex-col gap-6">
        {requests.data?.length === 0 && (
          <EmptyState title="Offer a fork's work back to review it here">
            A fork offers its work with "Offer as merge request" on its page, or <code className="font-mono">silt mr create</code>.
          </EmptyState>
        )}
        {list('Open', open)}
        {list('Closed', closed)}
      </div>
    </Page>
  )
}

/** A walk-through for reviewers: written by the fork's agent when asked, or by hand (ADR 0025). */
function ChangeGuide({
  id,
  guide,
  act,
  files,
}: {
  id: string
  guide: MrGuide | null
  act: (action: Promise<unknown>) => Promise<unknown>
  files: string[]
}) {
  const [writing, setWriting] = useState(false)
  const [text, setText] = useState('')
  const [asked, setAsked] = useState(false)
  return (
    <section aria-label="change guide" className="flex max-w-[720px] flex-col gap-2 rounded-md border border-line bg-surface-1 px-3.5 py-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">Change guide</h2>
        {guide && <span className="text-2xs text-fg-2">written by {guide.by}</span>}
        <span className="ml-auto flex gap-1">
          <Button
            size="sm"
            onClick={() => {
              setAsked(true)
              void act(api.writeGuide(id))
            }}
            title="The agent running in the fork writes it, on this machine"
          >
            Ask the fork's agent
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setWriting(!writing)}>
            Write it
          </Button>
        </span>
      </div>
      {guide ? (
        <div data-testid="guide">
          <Markdown text={guide.text} className="text-xs" />
        </div>
      ) : (
        <p className="text-xs text-fg-2">
          {asked ? 'The fork’s agent is writing it…' : 'No guide yet. A guide walks a reviewer through the files in the order to read them.'}
        </p>
      )}
      {guide && files.length > 0 && (
        <p className="flex flex-wrap gap-x-3 gap-y-1 text-2xs">
          {files.map((path) => (
            <Link key={path} to="/mr/$id" params={{ id }} hash={`file-${path}`} className="font-mono text-accent-fg hover:text-fg-1">
              {path}
            </Link>
          ))}
        </p>
      )}
      {writing && (
        <form
          className="flex flex-col gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            void act(api.writeGuide(id, text)).then(() => {
              setWriting(false)
              setText('')
            })
          }}
        >
          <Textarea aria-label="guide" rows={5} value={text} onChange={(event) => setText(event.target.value)} />
          <Button type="submit" variant="primary" disabled={!text.trim()} className="self-start">
            Keep guide
          </Button>
        </form>
      )}
    </section>
  )
}

/** Approving, asking for changes or declining: the merge request's decision, as the board's card draws it. */
function Approval({
  id,
  mr,
  canApprove,
  act,
  error,
}: {
  id: string
  mr: MrSummary
  canApprove: boolean
  act: (action: Promise<unknown>) => Promise<unknown>
  error: string | undefined
}) {
  const [reason, setReason] = useState('')
  const open = mr.status === 'open'
  return (
    <section aria-label="approval" className="flex w-full flex-col gap-2.5">
      <div className="flex items-center gap-2">
        <Dot tone={open ? 'warn' : standing(mr).tone} />
        <span className="text-xs font-medium">
          {open ? 'Needs 1 approval' : mr.status === 'approved' ? 'Approved and merged' : 'Declined'}
        </span>
        {open && <span className="ml-auto text-xs text-fg-2">{canApprove ? 'you can approve' : 'its owner approves'}</span>}
      </div>
      {open && (
        <>
          <Button variant="primary" size="lg" icon="check" className="justify-center" onClick={() => void act(api.approve(id))}>
            Approve and merge
          </Button>
          <div className="flex gap-1.5">
            <Input
              aria-label="reason"
              placeholder="Reason (needed to ask for changes)"
              className="min-w-0 flex-1 text-xs"
              value={reason}
              onChange={(event) => setReason(event.target.value)}
            />
          </div>
          <div className="flex gap-1.5">
            <Button
              className="flex-1 justify-center"
              disabled={!reason.trim()}
              title="Keeps it open and tells the fork what to change"
              onClick={() => void act(api.requestChanges(id, reason).then(() => setReason('')))}
            >
              Request changes
            </Button>
            <Button variant="danger" className="flex-1 justify-center" onClick={() => void act(api.decline(id, reason))}>
              Decline
            </Button>
          </div>
        </>
      )}
      <p className="text-2xs leading-normal text-fg-2">
        Merging brings the fork’s code and its conversation into{' '}
        <span className="font-mono" data-volatile>
          {short(mr.target)}
        </span>
        . History from both sides stays searchable.
      </p>
      {error && <p className="text-xs text-danger">{error}</p>}
    </section>
  )
}

type MrTab = 'changes' | 'conversation' | 'review'

/** A merge request's files one at a time, as a phone reviews them (the Phone — review a diff board). */
function PhoneFiles({
  id,
  files,
  at,
  onAt,
  seenNow,
  onSeen,
  diff,
  counts,
  commenting,
  target,
}: {
  id: string
  files: MrFile[]
  at: number
  onAt: (at: number) => void
  seenNow: (file: MrFile) => boolean
  onSeen: (path: string, seen: boolean) => void
  diff: string
  counts: Map<string, { added: number; removed: number }>
  commenting: Commenting
  target: string
}) {
  const file = files[at]
  const seen = files.filter(seenNow).length
  const stats = counts.get(file.path)
  const base = (path: string) => path.split('/').pop() ?? path
  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2 border-b border-line bg-surface-1 px-4 py-1">
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate font-mono text-xs">{file.path}</span>
          <span className="flex items-center gap-2 font-mono text-2xs text-fg-2">
            <span>
              file {at + 1} of {files.length} · <span data-testid="seen-count">{seen}/{files.length} seen</span>
            </span>
            <CopyLink path={`/mr/${id}#file-${file.path}`} thread={target} label="Copy link" name="link" className="-my-2 inline-flex min-h-11 items-center text-accent-fg" />
          </span>
        </span>
        {stats && <Counts added={stats.added} removed={stats.removed} />}
        <label className="flex min-h-11 items-center gap-1.5 text-xs">
          <input
            type="checkbox"
            aria-label={`seen ${file.path}`}
            checked={seenNow(file)}
            onChange={(event) => onSeen(file.path, event.target.checked)}
            className="h-5 w-5 accent-[var(--accent)]"
          />
          Seen
        </label>
      </div>
      <DiffView text={diff} only={file.path} commenting={commenting} bare />
      {files.length > 1 && (
        <nav aria-label="files" className="flex items-center justify-between gap-2 border-t border-line px-2 py-1">
          {at > 0 ? (
            <Button variant="ghost" onClick={() => onAt(at - 1)}>
              ← {base(files[at - 1].path)}
            </Button>
          ) : (
            <span />
          )}
          {at < files.length - 1 && (
            <Button variant="ghost" onClick={() => onAt(at + 1)}>
              {base(files[at + 1].path)} →
            </Button>
          )}
        </nav>
      )}
    </div>
  )
}

export function MergeRequestRoute() {
  const { id } = useParams({ from: '/mr/$id' })
  return <MergeRequest key={id} id={id} />
}

export function MergeRequest({ id }: { id: string }) {
  useTopic('mrs')
  const now = useNow()
  const details = useQuery({ queryKey: keys.mr(id), queryFn: () => api.mergeRequest(id) })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const queries = useQueryClient()
  const [error, setError] = useState<string>()
  const [ticked, setTicked] = useState<Record<string, boolean>>({})
  const [tab, setTab] = useState<MrTab>('changes')
  const [layout, setLayout] = useState<DiffLayout>('unified')
  const wide = useMedia(WIDE)
  const phone = useMedia('(max-width: 767px)')
  const [fileAt, setFileAt] = useState(0)
  const hash = useRouterState({ select: (state) => state.location.hash }).replace(/^#/, '')
  const picked = /^file-(.+)$/.exec(safeDecode(hash))?.[1]
  const fork = details.data?.mr.fork
  const comments = useQuery({ queryKey: keys.comments(fork ?? ''), queryFn: () => api.comments(fork!), enabled: fork !== undefined, retry: false })
  const loaded = details.data !== undefined
  useEffect(() => {
    if (loaded && picked) document.getElementById(`file-${picked}`)?.scrollIntoView({ block: 'start' })
  }, [loaded, picked])
  const act = (action: Promise<unknown>) =>
    action.then(
      () => {
        setError(undefined)
        return queries.invalidateQueries({ queryKey: keys.mrs })
      },
      (failure: unknown) => setError(message(failure)),
    )
  if (details.error) return <p className="p-6 text-danger">{message(details.error)}</p>
  if (!details.data)
    return (
      <div className="flex flex-col gap-3 p-6">
        <Skeleton className="h-6 w-1/3" />
        <Skeleton className="h-40" />
      </div>
    )
  const { mr, diff, conflicts, conversation, files } = details.data
  const forkThread = threads.data?.find((thread) => thread.id === mr.fork)
  const targetThread = threads.data?.find((thread) => thread.id === mr.target)
  const state = standing(mr)
  // A tick shows at once, before the daemon answers.
  const seenNow = (file: MrFile) => ticked[file.path] ?? file.seen
  const seenCount = files.filter(seenNow).length
  const counts = new Map(parseFiles(diff).map((file) => [file.path, file]))
  const reviewComments = (comments.data ?? []).filter((comment) => linesOf(comment) !== null)
  const reloadComments = () => queries.invalidateQueries({ queryKey: keys.comments(mr.fork) })
  const toFork = (text: string) =>
    api.prompt(mr.fork, text).then(
      (how) => toast(how === 'queued' ? "Queued for the fork's agent" : "Sent to the fork's agent", 'ok'),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const markSeen = (path: string, seen: boolean) => {
    setTicked((before) => ({ ...before, [path]: seen }))
    void act(api.markSeen(id, path, seen))
  }
  const seenBox = (file: MrFile) => (
    <input
      type="checkbox"
      aria-label={`seen ${file.path}`}
      checked={seenNow(file)}
      onChange={(event) => markSeen(file.path, event.target.checked)}
      className="accent-[var(--accent)] max-md:h-5 max-md:w-5"
    />
  )
  return (
    <Page
      flush
      label="merge request"
      title={mr.title}
      meta={
        <span className="inline-flex items-center gap-x-1.5">
          <span className="inline-flex items-center gap-1.5 rounded-full border border-line px-2 py-px font-sans text-2xs text-fg-1">
            <Dot tone={state.tone} />
            <span data-testid="mr-status" className="sr-only">
              {mr.status}
            </span>
            {state.label}
          </span>
          {forkThread?.owner && <Avatar name={forkThread.owner} size={16} />}
          <span>{forkThread?.owner ? `${forkThread.owner} wants to merge fork` : 'merge fork'}</span>
          <Link className="text-accent-fg" to="/t/$id" params={{ id: mr.fork }}>
            <span data-volatile>{short(mr.fork)}</span>
          </Link>
          <span>into</span>
          <Link className="text-accent-fg" to="/t/$id" params={{ id: mr.target }}>
            <span data-volatile>{short(mr.target)}</span>
          </Link>
          <span>
            {targetThread?.title && `· ${targetThread.title} `}· opened <span data-volatile>{ago(Number(mr.opened_ms), now)}</span>
          </span>
        </span>
      }
      actions={
        <Link to="/t/$id" params={{ id: mr.fork }} className={buttonClass('secondary')}>
          Open the fork
        </Link>
      }
      inspectorLabel="decision"
      inspectorTitle="Decision"
      inspector={
        wide && (
          <div className="flex flex-col gap-5 p-4">
            <Approval id={id} mr={mr} canApprove={targetThread?.writable ?? true} act={act} error={error} />
          </div>
        )
      }
    >
      <div className="flex flex-col gap-3 border-b border-line-subtle px-4 py-4 md:px-6">
        {mr.summary && <Markdown text={mr.summary} className="max-w-[720px]" />}
        <ChangeGuide id={id} guide={details.data.guide} act={act} files={files.map((file) => file.path)} />
      </div>
      {mr.changes_requested && (
        <div data-testid="changes-requested" role="note" className="mx-4 mt-4 flex flex-col gap-1 rounded-md bg-warn-tint px-3 py-2.5 text-xs md:mx-6">
          <span className="font-medium">Changes requested{mr.updated_since ? '; the fork has moved on since' : ''}</span>
          <span>{mr.changes_requested}</span>
        </div>
      )}
      {conflicts.length > 0 && (
        <div data-testid="conflicts" role="note" className="mx-4 mt-4 flex flex-col gap-1 rounded-md bg-warn-tint px-3 py-2.5 text-xs md:mx-6">
          <span className="font-medium">The merge keeps both sides in one place. Check these.</span>
          {conflicts.map((conflict) => (
            <span key={conflict} className="font-mono text-2xs">
              {conflict}
            </span>
          ))}
        </div>
      )}
      <div className="flex min-h-10 flex-wrap items-center gap-2 border-b border-line px-4 py-1.5 md:px-6">
        <Tabs
          label="merge request views"
          value={tab}
          onChange={setTab}
          tabs={[
            { id: 'changes', label: <>Changes <span className="font-mono text-fg-2">{files.length} {files.length === 1 ? 'file' : 'files'}</span></> },
            { id: 'conversation', label: <>The fork’s conversation <span className="font-mono text-fg-2">{messageCount(conversation)}</span></> },
            { id: 'review', label: <>Review <span className="font-mono text-fg-2">{reviewComments.length}</span></> },
          ]}
        />
        {tab === 'changes' && (
          <span className="ml-auto max-md:hidden">
            <Tabs
              label="layout"
              value={layout}
              onChange={setLayout}
              tabs={[
                { id: 'unified', label: 'Unified' },
                { id: 'split', label: 'Split' },
              ]}
            />
          </span>
        )}
      </div>
      {tab === 'changes' && phone && files.length > 0 && (
        <PhoneFiles
          id={id}
          files={files}
          at={Math.min(fileAt, files.length - 1)}
          onAt={setFileAt}
          seenNow={seenNow}
          onSeen={markSeen}
          diff={diff}
          counts={counts}
          commenting={{
            thread: mr.fork,
            comments: comments.data ?? [],
            agent: "the fork's agent",
            onChanged: reloadComments,
            onComment: async (at, text, send) => {
              const made = await api.addComment(mr.fork, { on: 'lines', ...at }, text)
              await reloadComments()
              if (send) await toFork(reviewPrompt([made]))
            },
          }}
          target={mr.target}
        />
      )}
      {tab === 'changes' && !(phone && files.length > 0) && (
        <div className="flex flex-col md:flex-row">
          {files.length > 0 && (
            <nav aria-label="files" className="flex shrink-0 flex-col gap-0.5 border-b border-line p-3 md:w-64 md:border-r md:border-b-0">
              <div className="flex items-center gap-2 px-2 pb-2 text-xs text-fg-2">
                <span className="h-1 flex-1 overflow-hidden rounded-sm bg-surface-2" aria-hidden="true">
                  <span className="block h-full bg-accent" style={{ width: `${(100 * seenCount) / files.length}%` }} />
                </span>
                <span data-testid="seen-count" className="font-mono text-2xs">
                  {seenCount}/{files.length} seen
                </span>
              </div>
              {files.map((file) => {
                const stats = counts.get(file.path)
                return (
                  <div
                    key={file.path}
                    className={cx(
                      'flex items-center gap-2 rounded-sm px-2 py-1 font-mono text-2xs hover:bg-surface-2 max-md:min-h-11',
                      file.path === picked ? 'bg-surface-2 text-fg-1' : 'text-fg-2',
                    )}
                  >
                    {seenBox(file)}
                    <Link to="/mr/$id" params={{ id }} hash={`file-${file.path}`} className="min-w-0 flex-1 truncate text-inherit hover:text-fg-1" title={file.path}>
                      {file.path}
                    </Link>
                    {stats && <Counts added={stats.added} removed={stats.removed} />}
                  </div>
                )
              })}
              <p className="px-2 pt-2 text-2xs leading-normal text-fg-2">A file you marked seen comes back if the fork changes it again.</p>
            </nav>
          )}
          <div className="min-w-0 flex-1 px-4 py-4 md:px-6">
            <DiffView
              text={diff}
              layout={layout}
              anchors
              picked={picked}
              viewFiles={mr.fork}
              actions={(file) => (
                <CopyLink
                  path={`/mr/${id}#file-${file.path}`}
                  thread={mr.target}
                  label="Copy link"
                  name="link"
                  className="font-sans text-accent-fg hover:text-fg-1 max-md:min-h-11"
                />
              )}
              commenting={{
                thread: mr.fork,
                comments: comments.data ?? [],
                agent: "the fork's agent",
                onChanged: reloadComments,
                onComment: async (at, text, send) => {
                  const made = await api.addComment(mr.fork, { on: 'lines', ...at }, text)
                  await reloadComments()
                  if (send) await toFork(reviewPrompt([made]))
                },
              }}
            />
          </div>
        </div>
      )}
      {tab === 'conversation' && (
        <div className="max-w-[880px] px-4 py-4 md:px-6">
          <Conversation lines={conversation} />
        </div>
      )}
      {tab === 'review' && (
        <div className="flex max-w-[880px] flex-col gap-3 px-4 py-4 md:px-6">
          {reviewComments.length === 0 && (
            <EmptyState title="Comment on the changes from their line numbers">Comments go to the fork, and its agent can take them as a prompt.</EmptyState>
          )}
          {reviewComments.map((comment) => {
            const at = linesOf(comment)!
            return (
              <div key={comment.id} className="flex flex-col gap-2 rounded-md border border-line bg-elevated px-3 py-2.5">
                <Link to="/mr/$id" params={{ id }} hash={`file-${at.path}`} className="font-mono text-2xs text-accent-fg">
                  {at.path}:{at.start}
                  {at.end !== at.start ? `–${at.end}` : ''}
                </Link>
                <CommentThread thread={mr.fork} comment={comment} onChanged={reloadComments} />
                {!comment.resolved && (
                  <span className="flex gap-1.5">
                    <Button size="sm" icon="handoff" onClick={() => void toFork(reviewPrompt([comment]))}>
                      Send to the fork’s agent
                    </Button>
                    <Button variant="ghost" size="sm" onClick={() => void api.resolveComment(mr.fork, comment.id).then(reloadComments)}>
                      Resolve
                    </Button>
                  </span>
                )}
              </div>
            )
          })}
          {reviewComments.some((comment) => !comment.resolved) && (
            <Button
              variant="primary"
              className="self-start"
              icon="handoff"
              onClick={() => void toFork(reviewPrompt(reviewComments.filter((comment) => !comment.resolved)))}
            >
              Send all to the fork’s agent
            </Button>
          )}
          {reviewComments.length > 0 && (
            <p className="flex items-center gap-1.5 text-2xs text-fg-2">
              <Icon name="info" className="h-3.5 w-3.5" />
              The fork’s agent takes them as its next prompt, on the machine it runs on.
            </p>
          )}
        </div>
      )}
      {/* On a phone the decision comes after the code, as the phone board orders it. */}
      {!wide && (
        <div className="px-4 pb-6">
          <Approval id={id} mr={mr} canApprove={targetThread?.writable ?? true} act={act} error={error} />
        </div>
      )}
    </Page>
  )
}
