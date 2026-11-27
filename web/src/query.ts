import { QueryClient } from '@tanstack/react-query'
import type { DiffSpec, Hidden } from './api'

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
  // Not under the thread, whose every event would ask the agent's provider again.
  usage: (agent: string, thread: string | undefined) => ['usage', agent, thread ?? ''] as const,
  history: (id: string) => ['thread', id, 'history'] as const,
  // Not under the thread: what a blob holds never changes, so a thread event needn't fetch it again.
  artifact: (id: string, blob: string) => ['artifact', id, blob] as const,
  turns: (id: string) => ['thread', id, 'turns'] as const,
  commits: (id: string) => ['thread', id, 'commits'] as const,
  unwritten: (id: string) => ['thread', id, 'unwritten'] as const,
  fileAt: (id: string, path: string, at: string | undefined) => ['thread', id, 'file', path, at ?? 'head'] as const,
  graph: (query: object) => ['graph', query] as const,
  comments: (id: string) => ['thread', id, 'comments'] as const,
  queue: (id: string) => ['thread', id, 'queue'] as const,
  diff: (id: string, options: DiffSpec) => ['thread', id, 'diff', options] as const,
  diffPages: (id: string, spec: DiffSpec) => ['thread', id, 'diff-pages', spec] as const,
  diffStats: (id: string, options: DiffSpec) => ['thread', id, 'diff-stats', options] as const,
  diffFile: (id: string, options: DiffSpec, path: string) => ['thread', id, 'diff-file', options, path] as const,
  diffTurns: (id: string, spec: DiffSpec) => ['thread', id, 'diff-turns', spec] as const,
  // Not under the thread: what a turn changed stays as it was once a later one began, so a thread event needn't fetch
  // it again for each turn shown.
  settledTurnStats: (id: string, options: DiffSpec) => ['settled-turn', id, 'stats', options] as const,
  settledTurnDiff: (id: string, options: DiffSpec) => ['settled-turn', id, 'diff', options] as const,
  threadSeen: (id: string, spec: DiffSpec) => ['thread', id, 'seen', spec] as const,
  mrs: ['mrs'] as const,
  asks: ['asks'] as const,
  approvals: (thread: string) => ['approvals', thread] as const,
  allApprovals: ['approvals'] as const,
  access: (thread: string) => ['approvals', 'access', thread] as const,
  sandboxChoice: (thread: string) => ['sandbox-choice', thread] as const,
  plugins: ['plugins'] as const,
  // Apart from `plugins`, so a change to one doesn't ask every repository again.
  pluginUpdates: ['pluginUpdates'] as const,
  pluginActivity: (id: string) => ['pluginActivity', id] as const,
  artifacts: ['artifacts'] as const,
  mr: (id: string) => ['mrs', id] as const,
  mrBrief: (id: string) => ['mrs', id, 'brief'] as const,
  mrShown: (id: string, hidden: Hidden) => ['mrs', id, hidden] as const,
  inbox: (all: boolean) => ['inbox', all] as const,
  agents: ['agents'] as const,
  designSystems: ['design-systems'] as const,
  designCatalog: ['design-systems', 'catalog'] as const,
  designSystem: (where: object) => ['design-systems', 'system', where] as const,
  designAsset: (where: object, path: string) => ['design-systems', 'asset', where, path] as const,
  threadDesign: (id: string) => ['thread', id, 'design-system'] as const,
  artifactSettings: ['artifact-settings'] as const,
  memory: ['memory'] as const,
  memorySettings: ['memory-settings'] as const,
  // Under `memory`, so a change refreshes the overview's counts too.
  memoryOf: (clone: string | null) => ['memory', clone ?? ''] as const,
  networkSettings: ['network-settings'] as const,
  themes: ['themes'] as const,
  themeDetail: (id: string) => ['themes', 'detail', id] as const,
  themeCss: ['themes', 'css'] as const,
  cloneDesignSystems: (clone: string) => ['design-systems', 'clone', clone] as const,
  projectDesignSystems: ['design-systems', 'projects'] as const,
  voice: ['voice'] as const,
  models: ['models'] as const,
  modelSearch: (words: string) => ['models', 'search', words] as const,
  modelFiles: (repo: string) => ['models', 'files', repo] as const,
  branches: (clone: string) => ['branches', clone] as const,
  projects: ['projects'] as const,
  found: ['projects', 'found'] as const,
  subfolders: (dir: string) => ['subfolders', dir] as const,
  orgs: ['orgs'] as const,
  devices: ['devices'] as const,
  lan: ['lan'] as const,
  links: (thread: string) => ['links', thread] as const,
  notificationSettings: ['notification-settings'] as const,
  status: ['status'] as const,
  update: ['update'] as const,
  chatSettings: ['chat-settings'] as const,
  // Under `threads`, which the bus refreshes when workspaces or org policies change.
  workspaces: ['threads', 'workspaces'] as const,
}

export function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
