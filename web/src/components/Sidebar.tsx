import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { STATE_TAG } from '../agentState'
import { api, type ThreadSummary } from '../api'
import { useTopic } from '../bus'
import { projectName, projects } from '../projects'
import { keys } from '../query'
import { Icon, Skeleton, Tag } from '../ui'

/** An agent state worth showing next to its thread; a stopped one isn't. */
function shownState(thread: ThreadSummary) {
  return thread.agent_state && thread.agent_state !== 'stopped' && thread.agent_state !== 'idle'
    ? STATE_TAG[thread.agent_state]
    : null
}

function ThreadItem({ thread }: { thread: ThreadSummary }) {
  const state = shownState(thread)
  return (
    <li className="flex items-center gap-2 pr-3 hover:bg-surface-2">
      <Link
        to="/t/$id"
        params={{ id: thread.id }}
        className="block min-w-0 flex-1 truncate py-1 pl-4 text-fg-1"
        activeProps={{ className: 'bg-accent-tint text-accent-fg' }}
      >
        {thread.title ?? 'Untitled thread'}
      </Link>
      {state && <Tag tone={state.tone}>{state.label}</Tag>}
    </li>
  )
}

export function Sidebar() {
  useTopic('threads')
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const waiting = (threads.data ?? []).filter(
    (thread) => thread.agent_state === 'awaiting_permission' || thread.agent_state === 'awaiting_input' || thread.agent_state === 'error',
  )
  return (
    <nav aria-label="projects" className="flex min-h-0 flex-1 flex-col overflow-y-auto py-2 text-sm">
      <Link
        to="/"
        className="mx-2 mb-2 flex items-center gap-2 rounded-sm px-2 py-1 text-fg-1 hover:bg-surface-2"
        activeOptions={{ exact: true }}
        activeProps={{ className: 'bg-accent-tint text-accent-fg' }}
      >
        <Icon name="thread" />
        All threads
      </Link>
      <Link
        to="/new"
        className="mx-2 mb-2 flex items-center gap-2 rounded-sm px-2 py-1 text-fg-1 hover:bg-surface-2"
        activeProps={{ className: 'bg-accent-tint text-accent-fg' }}
      >
        <Icon name="plus" />
        New thread
      </Link>
      {threads.isPending && (
        <div className="space-y-2 px-4">
          <Skeleton />
          <Skeleton className="h-4 w-2/3" />
        </div>
      )}
      {waiting.length > 0 && (
        <section aria-label="needs you" className="mb-2">
          <h2 className="px-4 pt-2 pb-1 font-mono text-[10px] tracking-wider text-accent-fg uppercase">Needs you</h2>
          <ul>
            {waiting.map((thread) => (
              <ThreadItem key={thread.id} thread={thread} />
            ))}
          </ul>
        </section>
      )}
      {projects(threads.data ?? []).map(([repo, inRepo]) => (
        <section key={repo} aria-label={repo} className="mb-2">
          <h2 className="flex items-center justify-between px-4 pt-2 pb-1 font-mono text-[10px] tracking-wider text-fg-3 uppercase">
            <span className="truncate" title={repo}>
              {projectName(repo)}
            </span>
            <span>{inRepo.length}</span>
          </h2>
          <ul>
            {inRepo.map((thread) => (
              <ThreadItem key={thread.id} thread={thread} />
            ))}
          </ul>
        </section>
      ))}
    </nav>
  )
}
