import { QueryClient } from '@tanstack/react-query'

export const queryClient = new QueryClient({
  defaultOptions: {
    // The daemon is local: a failure is worth showing, not retrying, and
    // changes arrive over the event bus rather than on window focus.
    queries: { retry: false, refetchOnWindowFocus: false },
  },
})

/** Query keys, nested so that one event invalidates everything under it (ADR 0021). */
export const keys = {
  threads: ['threads'] as const,
  // Under `threads`, so a launch's thread arriving fetches them again.
  cloudLaunches: ['threads', 'cloud'] as const,
  thread: (id: string) => ['thread', id] as const,
  history: (id: string) => ['thread', id, 'history'] as const,
  turns: (id: string) => ['thread', id, 'turns'] as const,
  commits: (id: string) => ['thread', id, 'commits'] as const,
  fileAt: (id: string, path: string, at: string | undefined) => ['thread', id, 'file', path, at ?? 'head'] as const,
  graph: (query: object) => ['graph', query] as const,
  comments: (id: string) => ['thread', id, 'comments'] as const,
  queue: (id: string) => ['thread', id, 'queue'] as const,
  diff: (id: string, options: { turn?: number; base?: string }) => ['thread', id, 'diff', options] as const,
  diffPages: (id: string, base: string) => ['thread', id, 'diff-pages', base] as const,
  mrs: ['mrs'] as const,
  mr: (id: string) => ['mrs', id] as const,
  inbox: (all: boolean) => ['inbox', all] as const,
  agents: ['agents'] as const,
  branches: (clone: string) => ['branches', clone] as const,
  projects: ['projects'] as const,
  found: ['projects', 'found'] as const,
  orgs: ['orgs'] as const,
  devices: ['devices'] as const,
  links: (thread: string) => ['links', thread] as const,
  notificationSettings: ['notification-settings'] as const,
  status: ['status'] as const,
}

export function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
