import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { api, type CommentView } from '../api'
import { parseDiff } from '../diffParse'
import { lineClass } from '../diffStyle'
import { keys, message } from '../query'
import { type Lines, linesOf, reviewPrompt } from '../review'
import { useActions } from '../shortcuts'
import { Button, Input, Select, Tabs } from '../ui'
import { CommentThread } from './CommentThread'

type View = 'last-turn' | 'thread' | 'main' | 'other'

const VIEWS: { id: View; label: string }[] = [
  { id: 'last-turn', label: 'Last turn' },
  { id: 'thread', label: 'This thread' },
  { id: 'main', label: 'Against main' },
  { id: 'other', label: 'Other…' },
]

function LineComment({ at, onSave, onCancel }: { at: Lines; onSave: (text: string) => Promise<void>; onCancel: () => void }) {
  const [text, setText] = useState('')
  const [error, setError] = useState<string>()
  return (
    <form
      className="my-1 flex gap-2 px-2 font-sans"
      onSubmit={(event) => {
        event.preventDefault()
        setError(undefined)
        onSave(text).catch((failure: unknown) => setError(message(failure)))
      }}
    >
      <Input
        aria-label="code comment"
        autoFocus
        className="flex-1 text-xs"
        placeholder={`Comment on ${at.path}:${at.start}${at.end !== at.start ? `-${at.end}` : ''}`}
        value={text}
        onChange={(event) => setText(event.target.value)}
      />
      <Button type="submit" variant="primary" disabled={!text.trim()}>
        Add
      </Button>
      <Button variant="ghost" onClick={onCancel}>
        Cancel
      </Button>
      {error && <span className="text-xs text-danger">{error}</span>}
    </form>
  )
}

/** A diff whose new lines take comments from the gutter; shift-click picks a range. */
function ReviewDiff({
  thread,
  text,
  comments,
  onComment,
  onChanged,
}: {
  thread: string
  text: string
  comments: CommentView[]
  onComment: (at: Lines, text: string) => Promise<void>
  onChanged: () => Promise<unknown>
}) {
  const [picking, setPicking] = useState<Lines | null>(null)
  const [writing, setWriting] = useState(false)
  if (!text.trim()) return <p className="text-sm text-fg-3">No changes.</p>
  const lines = parseDiff(text)
  const endingAt = (path: string | null, line: number | null) =>
    comments.filter((comment) => {
      const at = linesOf(comment)
      return at !== null && !comment.resolved && at.path === path && at.end === line
    })
  return (
    <div data-testid="diff" className="overflow-x-auto rounded-md border border-line font-mono text-xs leading-5">
      {lines.map((line, index) => {
        const commentable = line.path !== null && line.newLine !== null
        const picked =
          picking && commentable && picking.path === line.path && line.newLine! >= picking.start && line.newLine! <= picking.end
        return (
          <div key={index}>
            <div className={`flex ${lineClass(line.text)} ${picked ? 'ring-1 ring-accent ring-inset' : ''}`}>
              <span className="w-10 shrink-0 pr-1 text-right text-fg-3 select-none">
                {commentable && (
                  <button
                    aria-label={`comment on ${line.path} line ${line.newLine}`}
                    className="hover:text-accent-fg"
                    onClick={(event) => {
                      const here = line.newLine!
                      const extend = event.shiftKey && picking?.path === line.path
                      setPicking(
                        extend
                          ? { path: line.path!, start: Math.min(picking.start, here), end: Math.max(picking.end, here) }
                          : { path: line.path!, start: here, end: here },
                      )
                      setWriting(true)
                    }}
                  >
                    {line.newLine}
                  </button>
                )}
              </span>
              <span className="pr-2 whitespace-pre">{line.text || ' '}</span>
            </div>
            {endingAt(line.path, line.newLine).map((comment) => (
              <div key={comment.id} className="mx-2 my-0.5">
                <CommentThread thread={thread} comment={comment} onChanged={onChanged} />
              </div>
            ))}
            {writing && picking && picking.path === line.path && picking.end === line.newLine && (
              <LineComment
                at={picking}
                onCancel={() => {
                  setWriting(false)
                  setPicking(null)
                }}
                onSave={async (comment) => {
                  await onComment(picking, comment)
                  setWriting(false)
                  setPicking(null)
                }}
              />
            )}
          </div>
        )
      })}
    </div>
  )
}

/** What the thread changed, by turn, overall or against main, with comments from the gutter. */
export function DiffPanel({
  thread,
  comments,
  onCommented,
}: {
  thread: string
  comments: CommentView[]
  onCommented: () => Promise<unknown>
}) {
  const [open, setOpen] = useState(false)
  const [view, setView] = useState<View>('thread')
  const [other, setOther] = useState('merge-base')
  const [commit, setCommit] = useState('')
  const [sent, setSent] = useState<string>()
  const [error, setError] = useState<string>()
  const base =
    view === 'thread' ? '' : view === 'other' ? (other === 'commit' ? (commit ? `commit:${commit}` : '') : other) : view
  const diff = useQuery({
    queryKey: keys.diff(thread, { base }),
    queryFn: () => api.diff(thread, { base }),
    enabled: open,
  })
  const pending = comments.filter((comment) => !comment.resolved && linesOf(comment) !== null)
  const show = (shown: View) => () => {
    setOpen(true)
    setView(shown)
  }
  useActions([
    { id: 'changes', label: open ? 'Hide changes' : 'Show changes', keys: 'd', run: () => setOpen(!open) },
    { id: 'last-turn', label: 'Changes in the last turn', run: show('last-turn') },
    { id: 'against-main', label: 'Changes against main', run: show('main') },
  ])

  return (
    <section aria-label="changes" className="my-3">
      <div className="flex flex-wrap items-center gap-3">
        <Button onClick={() => setOpen(!open)}>{open ? 'Hide changes' : 'Show changes'}</Button>
        {open && <Tabs label="compare" tabs={VIEWS} value={view} onChange={setView} />}
        {open && view === 'other' && (
          <>
            <Select aria-label="diff base" value={other} onChange={(event) => setOther(event.target.value)}>
              <option value="merge-base">Since the merge base with its source</option>
              <option value="last-commit">Since the last commit</option>
              <option value="commit">Since a commit…</option>
            </Select>
            {other === 'commit' && (
              <Input aria-label="commit" placeholder="sha" className="w-28 font-mono" value={commit} onChange={(event) => setCommit(event.target.value)} />
            )}
          </>
        )}
        {pending.length > 0 && (
          <Button
            variant="primary"
            onClick={() => {
              setError(undefined)
              api.prompt(thread, reviewPrompt(pending)).then(
                (how) => setSent(how === 'queued' ? 'Queued for the agent' : 'Sent to the agent'),
                (failure: unknown) => setError(message(failure)),
              )
            }}
          >
            Send {pending.length} {pending.length === 1 ? 'comment' : 'comments'} to the agent
          </Button>
        )}
        {sent && <span className="text-xs text-fg-2">{sent}</span>}
      </div>
      {error && <p className="text-sm text-danger">{error}</p>}
      {open && diff.error && <p className="mt-2 text-sm text-danger">{message(diff.error)}</p>}
      {open && diff.data !== undefined && (
        <div className="mt-2">
          <ReviewDiff
            thread={thread}
            text={diff.data}
            comments={comments}
            onComment={async (at, text) => {
              await api.addComment(thread, { on: 'lines', ...at }, text)
              await onCommented()
            }}
            onChanged={onCommented}
          />
        </div>
      )}
    </section>
  )
}
