// Comments on an artifact (ADR 0067), each on the version it was made on,
// sent to the agent as one prompt as a review is (ADR 0055).

import { useState } from 'react'
import type { CommentView } from '../api'
import type { ArtifactGroup, ArtifactVersion } from '../artifacts'
import { useArtifactComments } from '../artifactComments'
import { message } from '../query'
import { Button, Icon, Popover, Textarea, cx, toast } from '../ui'
import { CommentThread } from './CommentThread'

/** The header's comments button: the artifact's comments, a box for a new one, and Send to the agent. */
export function ArtifactCommentsButton({
  thread,
  group,
  shown,
  comments,
  quote,
  onClearQuote,
  onChanged,
}: {
  thread: string
  group: ArtifactGroup
  shown: ArtifactVersion
  comments: CommentView[]
  /** Text selected in the artifact, which a new comment quotes. */
  quote: string | null
  onClearQuote: () => void
  onChanged: () => Promise<unknown>
}) {
  const { mine, waiting, add, send } = useArtifactComments(thread, group, shown, comments, onChanged)
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)
  const submit = async (andSend: boolean) => {
    setBusy(true)
    try {
      const made = text.trim() ? [await add(text, quote)] : []
      setText('')
      onClearQuote()
      if (andSend) {
        await send(made)
        toast('Sent to the agent', 'ok')
      } else await onChanged()
    } catch (failure) {
      toast(message(failure), 'danger')
    } finally {
      setBusy(false)
    }
  }
  return (
    <Popover
      label="Comments on the artifact"
      triggerLabel={mine.length > 0 ? `Comments, ${mine.length}` : 'Comments'}
      align="right"
      triggerClassName="relative inline-flex items-center gap-1 rounded-sm p-1.5 text-fg-1 hover:bg-surface-2 max-md:min-h-11 max-md:min-w-11"
      panelClassName="flex w-80 flex-col gap-2 p-3"
      trigger={
        <>
          <Icon name="comment" className="h-4 w-4" />
          {waiting.length > 0 && <span aria-hidden="true" className="absolute top-1 right-1 h-1.5 w-1.5 rounded-full bg-info" />}
        </>
      }
    >
      <div className="flex max-h-72 flex-col gap-1.5 overflow-y-auto" data-testid="artifact-comments">
        {mine.length === 0 && <p className="text-xs text-fg-2">Select something in the artifact to quote it, then say what to change.</p>}
        {mine.map((comment) => (
          <CommentThread key={comment.id} thread={thread} comment={comment} onChanged={onChanged} />
        ))}
      </div>
      {quote && (
        <blockquote className="flex items-start gap-2 border-l-2 border-accent pl-2 text-xs text-fg-2">
          <span className="line-clamp-3 flex-1">{quote}</span>
          <button type="button" aria-label="Don't quote it" onClick={onClearQuote} className="text-fg-3 hover:text-fg-1">
            <Icon name="x" className="h-3 w-3" />
          </button>
        </blockquote>
      )}
      <Textarea aria-label="comment on the artifact" rows={3} value={text} onChange={(event) => setText(event.target.value)} placeholder={`On v${shown.artifact.version}: what should change?`} />
      <div className="flex items-center justify-end gap-1.5">
        <Button variant="ghost" size="sm" disabled={busy || !text.trim()} onClick={() => void submit(false)}>
          Comment
        </Button>
        <Button variant="primary" size="sm" disabled={busy || (!text.trim() && waiting.length === 0)} onClick={() => void submit(true)}>
          Send to the agent{waiting.length > 0 ? ` (${waiting.length + (text.trim() ? 1 : 0)})` : ''}
        </Button>
      </div>
    </Popover>
  )
}

/** On a phone, the box under an open artifact: what to change, sent to the agent at once. */
export function AskForChange({ thread, group, shown, comments, quote, onClearQuote, onChanged }: Parameters<typeof ArtifactCommentsButton>[0]) {
  const { add, send } = useArtifactComments(thread, group, shown, comments, onChanged)
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)
  return (
    <form
      className="flex shrink-0 flex-col gap-1.5 border-t border-line bg-bg px-3 pt-2 pb-3"
      onSubmit={(event) => {
        event.preventDefault()
        if (!text.trim()) return
        setBusy(true)
        add(text, quote)
          .then((made) => send([made]))
          .then(() => {
            setText('')
            onClearQuote()
            toast('Sent to the agent', 'ok')
          })
          .catch((failure: unknown) => toast(message(failure), 'danger'))
          .finally(() => setBusy(false))
      }}
    >
      {quote && <p className="line-clamp-2 border-l-2 border-accent pl-2 text-xs text-fg-2">{quote}</p>}
      <label className={cx('flex min-h-11 items-center gap-2 rounded-lg border border-line-strong bg-elevated pr-1.5 pl-3')}>
        <span className="sr-only">Ask the agent for a change to this artifact</span>
        <input value={text} onChange={(event) => setText(event.target.value)} placeholder="Ask for a change to this" className="min-w-0 flex-1 bg-transparent outline-none" />
        <Button type="submit" variant="primary" size="sm" icon="arrowUp" aria-label="Send" disabled={busy || !text.trim()} />
      </label>
    </form>
  )
}
