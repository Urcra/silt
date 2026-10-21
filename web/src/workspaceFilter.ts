import { useQuery } from '@tanstack/react-query'
import { api, type ThreadSummary } from './api'
import { keys } from './query'
import { inWorkspace, orgsOf, useWorkspace } from './workspace'

/** Whether a thread is in the workspace this tab looks at (ADR 0032). */
export function useInWorkspace(): (thread: ThreadSummary) => boolean {
  const workspace = useWorkspace()
  const projects = useQuery({ queryKey: keys.projects, queryFn: api.projects, enabled: workspace.kind === 'server' })
  const orgs = orgsOf(projects.data ?? [])
  return (thread) => inWorkspace(thread, workspace, orgs)
}
