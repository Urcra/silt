import type { ThreadSummary } from '../api'
import { threadTone } from '../threadLook'
import { Dot, Spinner } from '../ui'

/** Before a thread's title: a spinner while its agent works, else the dot of what it waits for. */
export function ThreadMark({ thread }: { thread: ThreadSummary }) {
  if (thread.agent_state === 'working' || thread.agent_state === 'starting') return <Spinner label="working" />
  return (
    <span className="inline-flex w-[9px] justify-center">
      <Dot tone={threadTone(thread) === 'ok' ? 'idle' : threadTone(thread)} />
    </span>
  )
}
