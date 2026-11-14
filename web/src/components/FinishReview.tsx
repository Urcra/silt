import { useId, useState } from 'react'
import { type Finish, type FinishProps, useFinishReview } from '../finishReview'
import { linesOf, sideOf, spanOf } from '../review'
import { Button, Popover, Textarea, buttonClass, cx } from '../ui'

const CHOICE = 'grid cursor-pointer grid-cols-[16px_minmax(0,1fr)] gap-x-2 gap-y-px rounded-md border px-2 py-1.5'

function Choice({ value, picked, onPick, title, children }: { value: Finish; picked: Finish; onPick: (how: Finish) => void; title: string; children: string }) {
  const said = useId()
  return (
    <label className={cx(CHOICE, picked === value ? 'border-accent/60 bg-elevated' : 'border-transparent')}>
      <input
        type="radio"
        name="finish"
        value={value}
        aria-label={title}
        aria-describedby={said}
        checked={picked === value}
        onChange={() => onPick(value)}
        className="mt-0.5 accent-[var(--accent)]"
      />
      <span className="text-xs font-medium text-fg-1">{title}</span>
      <span id={said} className="col-start-2 text-2xs leading-snug text-fg-2">
        {children}
      </span>
    </label>
  )
}

/** The review's state and its Finish: what's pending, and a popover that sends it, as a forge's review does. */
export function FinishReview(props: FinishProps & { opener?: (open: (() => void) | null) => void }) {
  const { review, unsent, agent, working = false, opener } = props
  const { count, sending, error, finish } = useFinishReview(props)
  const { drafts, note } = review.review
  const [picked, setPicked] = useState<Finish>(agent ? 'send' : 'copy')
  const how: Finish = !agent && (picked === 'send' || picked === 'steer') ? 'copy' : !working && picked === 'steer' ? 'send' : picked
  const who = agent ?? 'the agent'
  if (count === 0) return null
  const comments = `${count} ${count === 1 ? 'comment' : 'comments'}`
  const submit = { send: 'Request changes', steer: 'Request changes now', copy: `Copy ${comments}`, save: 'Keep as comments' }[how]
  return (
    <span role="group" aria-label="review" className="flex items-center gap-2">
      <span className="text-2xs whitespace-nowrap text-fg-2">
        {drafts.length > 0 && `${drafts.length} pending`}
        {drafts.length > 0 && unsent.length > 0 && ' · '}
        {unsent.length > 0 && `${unsent.length} not sent yet`}
      </span>
      <Popover
        label="finish review"
        align="right"
        opener={opener}
        triggerClassName={buttonClass('primary', 'sm')}
        panelClassName="w-[440px] p-3"
        trigger={<>Finish review</>}
      >
        {(close) => (
          <form
            aria-label="finish review"
            className="flex flex-col gap-2"
            onSubmit={(event) => {
              event.preventDefault()
              void finish(how).then((done) => done && close())
            }}
          >
            <span className="flex items-baseline gap-2">
              <span className="text-sm font-medium">Finish review</span>
              <span className="text-2xs text-fg-2">{comments}</span>
            </span>
            <Textarea
              aria-label="review note"
              autoFocus
              rows={3}
              className="text-xs"
              placeholder={`A note for ${who} with the comments (optional)`}
              value={note}
              onChange={(event) => review.setNote(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
                  event.preventDefault()
                  void finish(how).then((done) => done && close())
                }
              }}
            />
            <ul aria-label="comments in the review" className="flex flex-col gap-0.5 font-mono text-2xs text-fg-2">
              {drafts.map((draft) => (
                <li key={draft.id} className="truncate">
                  {spanOf(draft.at)}
                  {sideOf(draft.at) === 'old' && ' (deleted)'}: <span className="font-sans text-fg-1">{draft.text}</span>
                </li>
              ))}
              {unsent.map((comment) => (
                <li key={comment.id} className="truncate">
                  {spanOf(linesOf(comment)!)} ({comment.by}): <span className="font-sans text-fg-1">{comment.text}</span>
                </li>
              ))}
            </ul>
            <fieldset className="flex flex-col gap-0.5">
              <legend className="pb-1 text-2xs text-fg-2">What {who} gets</legend>
              {agent ? (
                <>
                  <Choice value="send" picked={how} onPick={setPicked} title="Request changes">
                    {working
                      ? "One prompt: your note, then each comment under the lines it's on. It waits for the running turn to end."
                      : "One prompt: your note, then each comment under the lines it's on."}
                  </Choice>
                  {working && (
                    <Choice value="steer" picked={how} onPick={setPicked} title="Request changes, stopping the turn">
                      The running turn stops, and the review goes in its place.
                    </Choice>
                  )}
                </>
              ) : (
                <Choice value="copy" picked={how} onPick={setPicked} title="Copy for the agent's terminal">
                  No agent runs here from Silt: paste the review where it runs.
                </Choice>
              )}
              <Choice value="save" picked={how} onPick={setPicked} title="Keep as comments">
                Saved on the thread and synced; send them whenever you like.
              </Choice>
            </fieldset>
            <span className="flex flex-wrap items-center gap-1.5 border-t border-line-subtle pt-2">
              {drafts.length > 0 && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="text-danger hover:text-danger"
                  title="Drops the pending comments"
                  onClick={() => {
                    review.clear()
                    close()
                  }}
                >
                  Discard
                </Button>
              )}
              {agent && (
                <Button variant="ghost" size="sm" disabled={sending} onClick={() => void finish('copy').then((done) => done && close())}>
                  Copy for a terminal
                </Button>
              )}
              <span className="ml-auto" />
              <Button variant="ghost" size="sm" onClick={close}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" size="sm" disabled={sending}>
                {submit}
              </Button>
            </span>
            {error && <p className="text-xs text-danger">{error}</p>}
          </form>
        )}
      </Popover>
    </span>
  )
}
