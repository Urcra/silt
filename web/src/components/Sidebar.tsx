import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { api } from '../api'
import { useTopic } from '../bus'
import { projectName, projects } from '../projects'
import { keys } from '../query'
import { Icon, Skeleton } from '../ui'

export function Sidebar() {
  useTopic('threads')
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
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
      {threads.isPending && (
        <div className="space-y-2 px-4">
          <Skeleton />
          <Skeleton className="h-4 w-2/3" />
        </div>
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
              <li key={thread.id}>
                <Link
                  to="/t/$id"
                  params={{ id: thread.id }}
                  className="block truncate px-4 py-1 text-fg-1 hover:bg-surface-2"
                  activeProps={{ className: 'bg-accent-tint text-accent-fg' }}
                >
                  {thread.title ?? 'Untitled thread'}
                </Link>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </nav>
  )
}
