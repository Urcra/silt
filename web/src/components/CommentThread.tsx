import { useState } from 'react'
import { api, type CommentView } from '../api'
import { message } from '../query'
import { Button, Input } from '../ui'
import { CopyLink } from './CopyLink'

/** A comment with the conversation under it, and a box to add to it (ADR 0025). */
export function CommentThread({
  thread,
  comment,
  onChanged,
  highlighted = false,
}: {
  thread: string
  comment: CommentView
  onChanged: () => Promise<unknown>
  highlighted?: boolean
}) {
  const [replying, setReplying] = useState(false)
  const [text, setText] = useState('')
  const [error, setError] = useState<string>()
  return (
    <div
      role="group"
      aria-label={`comment by ${comment.by}`}
      data-highlighted={highlighted || undefined}
      className={`rounded-sm bg-accent-tint px-2 py-1 font-sans text-xs text-fg-1 ${comment.resolved ? 'opacity-60' : ''} ${highlighted ? 'ring-2 ring-accent' : ''}`}
    >
      <p>
        {comment.by}: {comment.text} {comment.resolved && '(resolved)'}
      </p>
      {comment.replies.map((reply) => (
        <p key={reply.id} className="mt-0.5 border-l-2 border-line pl-2 text-fg-2">
          {reply.by}: {reply.text}
        </p>
      ))}
      {replying ? (
        <form
          className="mt-1 flex items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            setError(undefined)
            api.reply(thread, comment.id, text).then(
              () => {
                setText('')
                setReplying(false)
                return onChanged()
              },
              (failure: unknown) => setError(message(failure)),
            )
          }}
        >
          <Input aria-label="reply" autoFocus className="flex-1 text-xs" value={text} onChange={(event) => setText(event.target.value)} />
          <Button type="submit" disabled={!text.trim()}>
            Reply
          </Button>
          {error && <span className="text-danger">{error}</span>}
        </form>
      ) : (
        <span className="mt-0.5 flex gap-3">
          <button type="button" className="text-accent-fg" onClick={() => setReplying(true)}>
            reply
          </button>
          <CopyLink path={`/t/${thread}#comment-${comment.id}`} label="link" />
        </span>
      )}
    </div>
  )
}
