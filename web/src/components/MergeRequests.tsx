import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useParams, useRouterState } from '@tanstack/react-router'
import { useEffect, useMemo, useState } from 'react'
import { api, hiddenLines, short, type CommentView, type MrFile, type MrGuide, type MrSummary, type ShowLine } from '../api'
import { useTopic } from '../bus'
import { useHideGit, useHideOthers } from '../hideGit'
import { parseFiles } from '../diffParse'
import { WIDE, useMedia } from '../media'
import { useNow } from '../now'
import { keys, message } from '../query'
import { itemOf, linesOf, reviewPrompt } from '../review'
import { ago } from '../time'
import {
  Avatar,
  Button,
  Counts,
  Dot,
  type DotTone,
  EmptyState,
  Icon,
  Skeleton,
  Tabs,
  Textarea,
  buttonClass,
  cx,
  toast,
} from '../ui'
import { CommentThread } from './CommentThread'
import { Conversation } from './Conversation'
import { CopyLink } from './CopyLink'
import { useShared } from '../hideGit'
import { HideGitBox, HideOthersBox } from './DiffPanel'
import { useTextSettings } from '../text'
import { type Commenting, type DiffLayout, DiffView } from './DiffView'
import { Markdown } from './Markdown'
import { ListColumn, Page } from './Page'
import { SeenFiles } from './SeenFiles'

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

type ListTab = 'open' | 'merged' | 'declined'
const LIST_TABS: [ListTab, string][] = [
  ['open', 'Open'],
  ['merged', 'Merged'],
  ['declined', 'Declined'],
]

export function MergeRequests() {
  useTopic('mrs')
  const now = useNow()
  const requests = useQuery({ queryKey: keys.mrs, queryFn: api.mergeRequests })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const titleOf = (id: string) => threads.data?.find((thread) => thread.id === id)?.title ?? short(id)
  const ownerOf = (id: string) => threads.data?.find((thread) => thread.id === id)?.owner
  const [tab, setTab] = useState<ListTab>('open')
  const shown = [...(requests.data ?? [])].reverse()
  const inTab = (which: ListTab) =>
    shown.filter((mr) =>
      which === 'open' ? mr.status === 'open' : which === 'merged' ? mr.status === 'approved' : mr.status.startsWith('declined'),
    )
  const open = inTab('open')
  const list = (title: string, items: MrSummary[]) =>
    items.length === 0 ? (
      <p className="px-1 text-sm text-fg-2">
        No {title.toLowerCase()} merge requests.
        {title === 'Open' && ' A fork offers its work back with “Offer as merge request” on its page.'}
      </p>
    ) : (
      <section aria-label={title.toLowerCase()} className="flex flex-col gap-2">
        <ul className="overflow-hidden rounded-md border border-line bg-elevated">
          {items.map((mr) => {
            const state = standing(mr)
            return (
              <li
                key={mr.id}
                className="flex items-center gap-3 border-b border-line-subtle px-3 py-2 last:border-b-0 hover:bg-surface-2 max-md:min-h-14"
              >
                <Dot tone={state.tone} label={state.label} />
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <Link className="truncate font-medium text-fg-1 hover:text-accent-fg" to="/mr/$id" params={{ id: mr.id }}>
                    {mr.title}
                  </Link>
                  <span className="truncate text-xs text-fg-2">
                    {[ownerOf(mr.fork), `→ ${titleOf(mr.target)}`, state.label].filter(Boolean).join(' · ')}
                    {' · '}
                    <span data-volatile>{ago(Number(mr.decided_ms ?? mr.opened_ms), now)}</span>
                  </span>
                </div>
              </li>
            )
          })}
        </ul>
      </section>
    )
  return (
    <Page flush label="merge requests" title="Merge requests" meta={`${open.length} open`}>
      <ListColumn>
        {requests.error && <p className="text-danger">{message(requests.error)}</p>}
        {requests.isPending && <Skeleton className="h-24" />}
        <div data-testid="merge-requests" className="flex flex-col gap-4">
          {requests.data?.length === 0 && (
            <EmptyState title="Offer a fork's work back to review it here">
              A fork offers its work with "Offer as merge request" on its page, or{' '}
              <code className="font-mono">silt mr create</code>.
            </EmptyState>
          )}
          {(requests.data?.length ?? 0) > 0 && (
            <>
              <Tabs
                label="merge requests by state"
                value={tab}
                onChange={setTab}
                tabs={LIST_TABS.map(([id, label]) => ({
                  id,
                  label: (
                    <>
                      {label} <span className="font-mono text-fg-2">{inTab(id).length}</span>
                    </>
                  ),
                }))}
              />
              {list(LIST_TABS.find(([id]) => id === tab)![1], inTab(tab))}
            </>
          )}
        </div>
      </ListColumn>
    </Page>
  )
}

/** A walk-through for reviewers: written by the fork's agent when asked, or by hand (ADR 0025). */
function ChangeGuide({
  id,
  guide,
  act,
  files,
  decided,
}: {
  id: string
  guide: MrGuide | null
  act: (action: Promise<unknown>) => Promise<unknown>
  files: string[]
  /** Approved or declined, so there's no more reviewing to guide. */
  decided: boolean
}) {
  const [writing, setWriting] = useState(false)
  const [text, setText] = useState('')
  const [asked, setAsked] = useState(false)
  const ask = () => {
    setAsked(true)
    void act(api.writeGuide(id))
  }
  // With none written, one line offers both ways to write it rather than a card.
  if (!guide && !writing && !asked && !decided)
    return (
      <section aria-label="change guide" className="flex flex-wrap items-center gap-x-1 gap-y-1 text-xs text-fg-2">
        <Icon name="edit" className="mr-1 h-3.5 w-3.5" />
        No change guide yet.
        <Button
          variant="ghost"
          size="sm"
          className="text-accent-fg"
          onClick={ask}
          title="The agent running in the fork writes it, on this machine"
        >
          Ask the fork's agent
        </Button>
        to write one, or
        <Button variant="ghost" size="sm" className="text-accent-fg" onClick={() => setWriting(true)}>
          write it
        </Button>
      </section>
    )
  return (
    <section
      aria-label="change guide"
      className="flex max-w-[720px] flex-col gap-2 rounded-md border border-line bg-surface-1 px-3.5 py-3"
    >
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">Change guide</h2>
        {guide && <span className="text-2xs text-fg-2">written by {guide.by}</span>}
        <span className={cx('ml-auto flex gap-1', decided && 'hidden')}>
          <Button size="sm" onClick={ask} title="The agent running in the fork writes it, on this machine">
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
          {asked
            ? 'The fork’s agent is writing it…'
            : 'No guide yet. A guide walks a reviewer through the files in the order to read them.'}
        </p>
      )}
      {guide && files.length > 0 && (
        <p className="flex flex-wrap gap-x-3 gap-y-1 text-2xs">
          {files.map((path) => (
            <Link
              key={path}
              to="/mr/$id"
              params={{ id }}
              hash={`file-${path}`}
              className="font-mono text-accent-fg hover:text-fg-1"
            >
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

/** One line of what's left before merging: done, waiting, or optional (`null`). */
function Check({ done, label, detail }: { done: boolean | null; label: string; detail: string }) {
  return (
    <li className="flex items-center gap-2 text-xs">
      <Icon
        name={done === false ? 'alert' : done ? 'check' : 'edit'}
        className={cx('h-3.5 w-3.5', done === false ? 'text-warn' : done ? 'text-ok' : 'text-fg-3')}
      />
      <span className={done === null ? 'text-fg-2' : 'text-fg-1'}>{label}</span>
      <span className="ml-auto text-fg-2">{detail}</span>
    </li>
  )
}

/** Approving, asking for changes or declining, after what's left to check: the merge request's decision. */
function Approval({
  id,
  mr,
  target,
  canApprove,
  act,
  error,
  seen,
  files,
  conflicts,
  guided,
  added,
  removed,
}: {
  id: string
  mr: MrSummary
  /** The target thread's title, when this machine knows it. */
  target: string | undefined
  canApprove: boolean
  act: (action: Promise<unknown>) => Promise<unknown>
  error: string | undefined
  seen: number
  files: number
  conflicts: number
  guided: boolean
  added: number
  removed: number
}) {
  const [reason, setReason] = useState('')
  const [asking, setAsking] = useState(false)
  const now = useNow()
  const open = mr.status === 'open'
  const targetName = (
    <span className={target ? undefined : 'font-mono'} title={mr.target} data-volatile>
      {target ?? short(mr.target)}
    </span>
  )
  const declinedFor = mr.status.startsWith('declined: ') ? mr.status.slice('declined: '.length) : undefined
  const allSeen = seen === files
  const checks = [allSeen, conflicts === 0]
  const ready = checks.filter(Boolean).length
  return (
    <section aria-label="approval" className="flex w-full flex-col gap-3">
      <div className="flex items-center gap-2">
        <Dot tone={open ? 'warn' : standing(mr).tone} />
        <span className="text-xs font-medium">
          {open ? 'Needs 1 approval' : mr.status === 'approved' ? 'Approved and merged' : 'Declined'}
        </span>
        {open && <span className="ml-auto text-xs text-fg-2">{canApprove ? 'you can approve' : 'its owner approves'}</span>}
        {!open && mr.decided_ms !== null && (
          <span className="ml-auto text-xs text-fg-2" data-volatile>
            {ago(Number(mr.decided_ms), now)}
          </span>
        )}
      </div>
      {mr.status === 'approved' && <p className="text-xs text-fg-2">Its code and conversation are now part of {targetName}.</p>}
      {declinedFor && <p className="text-xs text-fg-2">{declinedFor}</p>}
      {open && (
        <>
          <ul aria-label="before merging" className="flex flex-col gap-2">
            <Check done={allSeen} label={allSeen ? 'All files seen' : 'Files to see'} detail={`${seen} of ${files}`} />
            <Check
              done={conflicts === 0}
              label={conflicts === 0 ? 'Merges cleanly' : 'Both sides changed the same files'}
              detail={conflicts === 0 ? 'no conflicts' : `${conflicts} ${conflicts === 1 ? 'file' : 'files'}`}
            />
            <Check
              done={guided || null}
              label={guided ? 'Change guide written' : 'No change guide'}
              detail={guided ? 'above' : 'optional'}
            />
          </ul>
          <span className="h-1 overflow-hidden rounded-sm bg-surface-2" aria-hidden="true">
            <span
              className={cx('block h-full', ready === checks.length ? 'bg-ok' : 'bg-accent')}
              style={{ width: `${(100 * ready) / checks.length}%` }}
            />
          </span>
          {/* Approving stays possible before every file is seen; it's only not the obvious next step yet. */}
          <Button
            variant={allSeen ? 'primary' : 'secondary'}
            size="lg"
            icon="check"
            className="justify-center"
            title={allSeen ? undefined : `${files - seen} of the files aren't marked seen yet`}
            onClick={() => void act(api.approve(id))}
          >
            Approve and merge
          </Button>
          {asking ? (
            <form
              className="flex flex-col gap-1.5"
              onSubmit={(event) => {
                event.preventDefault()
                void act(
                  api.requestChanges(id, reason).then(() => {
                    setReason('')
                    setAsking(false)
                  }),
                )
              }}
            >
              <Textarea
                aria-label="reason"
                rows={3}
                autoFocus
                placeholder="What should the fork change?"
                className="text-xs"
                value={reason}
                onChange={(event) => setReason(event.target.value)}
              />
              <span className="flex gap-1.5">
                <Button
                  type="submit"
                  variant="primary"
                  disabled={!reason.trim()}
                  title="Keeps it open and tells the fork what to change"
                >
                  Send the request
                </Button>
                <Button variant="ghost" onClick={() => setAsking(false)}>
                  Cancel
                </Button>
              </span>
            </form>
          ) : (
            <div className="flex gap-1.5">
              <Button className="flex-1 justify-center" onClick={() => setAsking(true)}>
                Request changes…
              </Button>
              <Button variant="danger" className="flex-1 justify-center" onClick={() => void act(api.decline(id, reason))}>
                Decline
              </Button>
            </div>
          )}
          <p className="text-2xs leading-normal text-fg-2">
            Merging brings <Counts added={added} removed={removed} /> and the fork’s conversation into {targetName}. History from
            both sides stays searchable.
          </p>
        </>
      )}
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
  /** Unset once it is decided, when there's nothing left to tick off. */
  onSeen: ((path: string, seen: boolean) => void) | undefined
  diff: string
  counts: Map<string, { added: number; removed: number; binary: boolean }>
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
              file {at + 1} of {files.length}
              {onSeen && (
                <>
                  {' · '}
                  <span data-testid="seen-count">
                    {seen}/{files.length} seen
                  </span>
                </>
              )}
            </span>
            <CopyLink
              path={`/mr/${id}#file-${file.path}`}
              thread={target}
              label="Copy link"
              name="link"
              className="-my-2 inline-flex min-h-11 items-center text-accent-fg"
            />
          </span>
        </span>
        {stats && <Counts added={stats.added} removed={stats.removed} binary={stats.binary} />}
        {onSeen && (
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
        )}
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
  const [hideGit, setHideGit] = useHideGit()
  const [hideOthers, setHideOthers] = useHideOthers()
  const preferredLayout = useTextSettings().shown.diff_layout
  const hidden = { hideGit, hideOthers }
  const details = useQuery({ queryKey: keys.mrShown(id, hidden), queryFn: () => api.mergeRequest(id, hidden) })
  const shared = useShared(details.data?.mr.fork)
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const queries = useQueryClient()
  const [error, setError] = useState<string>()
  const [ticked, setTicked] = useState<Record<string, boolean>>({})
  const [tab, setTab] = useState<MrTab>('changes')
  const [layout, setLayout] = useState<DiffLayout>(() => preferredLayout ?? 'unified')
  const wide = useMedia(WIDE)
  const phone = useMedia('(max-width: 767px)')
  const [fileAt, setFileAt] = useState(0)
  const hash = useRouterState({ select: (state) => state.location.hash }).replace(/^#/, '')
  const picked = /^file-(.+)$/.exec(safeDecode(hash))?.[1]
  const fork = details.data?.mr.fork
  // Placed as the diff numbers the lines, with git's rewrites and other agents' edits or without.
  const comments = useQuery({
    queryKey: [...keys.comments(fork ?? ''), '', hidden],
    queryFn: () => api.comments(fork!, undefined, hidden),
    enabled: fork !== undefined,
    retry: false,
  })
  const loaded = details.data !== undefined
  // Parsed once per diff, not at each render: a tick, a comment or a minute passing drew the page again.
  const parsed = useMemo(() => parseFiles(details.data?.diff ?? ''), [details.data?.diff])
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
  const open = mr.status === 'open'
  // A tick shows at once, before the daemon answers.
  const seenNow = (file: MrFile) => ticked[file.path] ?? file.seen
  const seenCount = files.filter(seenNow).length
  const counts = new Map(parsed.map((file) => [file.path, file]))
  const total = [...counts.values()].reduce(
    (sum, file) => ({ added: sum.added + file.added, removed: sum.removed + file.removed }),
    { added: 0, removed: 0 },
  )
  const decision = {
    id,
    mr,
    target: targetThread?.title ?? undefined,
    canApprove: targetThread?.writable ?? true,
    act,
    error,
    seen: seenCount,
    files: files.length,
    conflicts: conflicts.length,
    guided: details.data.guide !== null,
    ...total,
  }
  const reviewComments = (comments.data ?? []).filter((comment) => linesOf(comment) !== null)
  const reloadComments = () => queries.invalidateQueries({ queryKey: keys.comments(mr.fork) })
  const toFork = (sent: CommentView[]) =>
    api
      .prompt(mr.fork, reviewPrompt(sent.flatMap((comment) => itemOf(comment, parsed) ?? [])))
      .then(async (how) => {
        await api.commentsSent(
          mr.fork,
          sent.map((comment) => comment.id),
        )
        await reloadComments()
        toast(how === 'queued' ? "Queued for the fork's agent" : "Sent to the fork's agent", 'ok')
      })
      .catch((failure: unknown) => toast(message(failure), 'danger'))
  const unsent = reviewComments.filter((comment) => !comment.resolved && !comment.sent)
  const commenting: Commenting = {
    thread: mr.fork,
    comments: comments.data ?? [],
    agent: "the fork's agent",
    onChanged: reloadComments,
    onComment: async (at, text, send) => {
      const made = await api.addComment(mr.fork, { on: 'lines', ...at, ...hiddenLines(hidden) }, text)
      await reloadComments()
      if (send) await toFork([made])
    },
  }
  const markSeen = (path: string, seen: boolean) => {
    setTicked((before) => ({ ...before, [path]: seen }))
    void act(api.markSeen(id, path, seen))
  }
  return (
    <Page
      flush
      label="merge request"
      title={mr.title}
      meta={
        <span className="inline-flex items-center gap-1.5 rounded-full bg-accent-tint px-2 py-px font-sans text-2xs text-accent-fg">
          <Dot tone={state.tone} />
          <span data-testid="mr-status" className="sr-only">
            {mr.status}
          </span>
          {state.label}
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
            <Approval {...decision} />
          </div>
        )
      }
    >
      <div className="flex flex-col gap-3 border-b border-line-subtle px-4 py-4 md:px-6">
        {/* The route gets a row of its own under the header, so neither thread's title is cut by the header's buttons. */}
        <p aria-label="route" className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-fg-2">
          <Link
            to="/t/$id"
            params={{ id: mr.fork }}
            title={forkThread?.title ?? mr.fork}
            className={cx(buttonClass('secondary', 'sm'), 'max-w-[22rem] font-normal')}
          >
            <Icon name="fork" className="h-3 w-3" />
            <span className="truncate" data-volatile>
              {forkThread?.title ?? short(mr.fork)}
            </span>
          </Link>
          <span aria-label="into">→</span>
          <Link
            to="/t/$id"
            params={{ id: mr.target }}
            title={targetThread?.title ?? mr.target}
            className={cx(buttonClass('secondary', 'sm'), 'max-w-[22rem] font-normal')}
          >
            <span className="truncate" data-volatile>
              {targetThread?.title ?? short(mr.target)}
            </span>
          </Link>
          <span className="ml-1 inline-flex items-center gap-1.5">
            {forkThread?.owner && <Avatar name={forkThread.owner} size={16} />}
            {forkThread?.owner ?? 'Someone'} opened it <span data-volatile>{ago(Number(mr.opened_ms), now)}</span>
          </span>
        </p>
        {mr.summary && <Markdown text={mr.summary} className="max-w-[720px]" />}
        {(open || details.data.guide) && (
          <ChangeGuide id={id} guide={details.data.guide} act={act} files={files.map((file) => file.path)} decided={!open} />
        )}
      </div>
      {mr.changes_requested && (
        <div
          data-testid="changes-requested"
          role="note"
          className="mx-4 mt-4 flex flex-col gap-1 rounded-md bg-warn-tint px-3 py-2.5 text-xs md:mx-6"
        >
          <span className="font-medium">Changes requested{mr.updated_since ? '; the fork has moved on since' : ''}</span>
          <span>{mr.changes_requested}</span>
        </div>
      )}
      {conflicts.length > 0 && (
        <div
          data-testid="conflicts"
          role="note"
          className="mx-4 mt-4 flex flex-col gap-1 rounded-md bg-warn-tint px-3 py-2.5 text-xs md:mx-6"
        >
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
            {
              id: 'changes',
              label: (
                <>
                  Changes{' '}
                  <span className="font-mono text-fg-2">
                    {files.length} {files.length === 1 ? 'file' : 'files'}
                  </span>
                </>
              ),
            },
            {
              id: 'conversation',
              label: (
                <>
                  The fork’s conversation <span className="font-mono text-fg-2">{messageCount(conversation)}</span>
                </>
              ),
            },
            {
              id: 'review',
              label: (
                <>
                  Review <span className="font-mono text-fg-2">{reviewComments.length}</span>
                </>
              ),
            },
          ]}
        />
        {tab === 'changes' && (
          <span className="ml-auto flex flex-wrap items-center gap-2">
            <HideGitBox on={hideGit} onChange={setHideGit} />
            {shared && <HideOthersBox on={hideOthers} onChange={setHideOthers} />}
          </span>
        )}
        {tab === 'changes' && (
          <span className="max-md:hidden">
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
          onSeen={open ? markSeen : undefined}
          diff={diff}
          counts={counts}
          commenting={commenting}
          target={mr.target}
        />
      )}
      {tab === 'changes' && !(phone && files.length > 0) && (
        <div className="flex flex-col md:flex-row">
          {files.length > 0 && (
            <SeenFiles
              className="border-b border-line p-3 md:w-64 md:border-r md:border-b-0"
              files={files.map((file) => {
                const stats = counts.get(file.path)
                return {
                  path: file.path,
                  added: stats?.added,
                  removed: stats?.removed,
                  binary: stats?.binary,
                  seen: seenNow(file) ? 'yes' : 'no',
                }
              })}
              picked={picked}
              onSeen={open ? markSeen : undefined}
              hint="A file you marked seen comes back if the fork changes it again."
              link={(path) => (
                <Link
                  to="/mr/$id"
                  params={{ id }}
                  hash={`file-${path}`}
                  className="min-w-0 flex-1 truncate text-inherit hover:text-fg-1"
                  title={path}
                >
                  {path}
                </Link>
              )}
            />
          )}
          <div className="min-w-0 flex-1 px-4 py-4 md:px-6">
            <DiffView
              files={parsed}
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
              commenting={commenting}
            />
          </div>
        </div>
      )}
      {tab === 'conversation' && (
        <div role="tabpanel" aria-label="the fork's conversation" className="max-w-[880px] px-4 py-4 md:px-6">
          <Conversation lines={conversation} />
        </div>
      )}
      {tab === 'review' && (
        <div className="flex max-w-[880px] flex-col gap-3 px-4 py-4 md:px-6">
          {reviewComments.length === 0 && (
            <EmptyState title="Comment on the changes from their line numbers">
              Comments go to the fork, and its agent can take them as a prompt.
            </EmptyState>
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
                    <Button size="sm" icon="handoff" onClick={() => void toFork([comment])}>
                      {comment.sent ? 'Send again' : 'Send to the fork’s agent'}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => void api.resolveComment(mr.fork, comment.id).then(reloadComments)}
                    >
                      Resolve
                    </Button>
                  </span>
                )}
              </div>
            )
          })}
          {unsent.length > 0 && (
            <Button variant="primary" className="self-start" icon="handoff" onClick={() => void toFork(unsent)}>
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
          <Approval {...decision} />
        </div>
      )}
    </Page>
  )
}
