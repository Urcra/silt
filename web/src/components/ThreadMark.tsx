import type { ThreadSummary } from '../api'
import { settled } from '../sections'
import { threadDoing } from '../threadLook'
import { Spinner, cx } from '../ui'

/**
 * Before a thread's title, its state by shape as well as colour: a turning ring while
 * its agent works, a filled ring while it waits on someone, a solid dot once settled,
 * an open ring otherwise.
 */
export function ThreadMark({ thread }: { thread: ThreadSummary }) {
  if (thread.agent_state === 'working' || thread.agent_state === 'starting') return <Spinner label="working" />
  const waiting = thread.agent_state === 'awaiting_permission' || thread.agent_state === 'awaiting_input' || thread.review
  const done = !waiting && settled(thread)
  const label = threadDoing(thread) ?? (done ? 'settled' : undefined)
  return (
    // Hidden from the name of the link it's in, which starts with the title; the state is said in words beside it.
    <span aria-hidden="true" title={label} className="inline-flex h-[9px] w-[9px] shrink-0 items-center justify-center">
      <span
        aria-hidden="true"
        className={cx(
          'block rounded-full',
          thread.agent_state === 'error'
            ? 'h-1.5 w-1.5 bg-danger'
            : waiting
              ? 'h-[9px] w-[9px] border-[1.5px] border-accent bg-accent-tint'
              : done
                ? 'h-1.5 w-1.5 bg-fg-3'
                : 'h-[9px] w-[9px] border-[1.5px] border-fg-3',
        )}
      />
    </span>
  )
}
