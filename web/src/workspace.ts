// Which workspace this tab looks at: one of this machine's workspaces
// (ADR 0065), or in the hosted app an org on its server or the whole server
// (ADR 0032), this machine's own threads, or everything.

import { useSyncExternalStore } from 'react'
import type { Project, ThreadSummary } from './api'

export type Workspace =
  | { kind: 'all' }
  | { kind: 'local' }
  | { kind: 'server'; server: string; org: string | null }
  | { kind: 'workspace'; id: string; server: string | null }

const STORED = 'silt-workspace'
const listeners = new Set<() => void>()
let current: Workspace = stored()

function stored(): Workspace {
  try {
    const text = sessionStorage.getItem(STORED)
    return text ? (JSON.parse(text) as Workspace) : { kind: 'all' }
  } catch {
    return { kind: 'all' }
  }
}

export function setWorkspace(workspace: Workspace) {
  current = workspace
  try {
    sessionStorage.setItem(STORED, JSON.stringify(workspace))
  } catch {
    // Kept for this page only, then.
  }
  for (const listener of listeners) listener()
}

export function useWorkspace(): Workspace {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => current,
  )
}

/** The server what belongs to no thread is asked of: the workspace's, or the default. */
export function askedServer(): string | null {
  return current.kind === 'server' || current.kind === 'workspace' ? current.server : null
}

/** The org each project is in, by its server and repository. */
export function orgsOf(projects: Project[]): Map<string, string | null> {
  return new Map(
    projects.filter((project) => project.server).map((project) => [`${project.server}\n${project.repo}`, project.org]),
  )
}

export function inWorkspace(thread: ThreadSummary, workspace: Workspace, orgs: Map<string, string | null>): boolean {
  switch (workspace.kind) {
    case 'all':
      return true
    case 'local':
      return thread.local_only || thread.server === null
    case 'server':
      return (
        thread.server === workspace.server &&
        (workspace.org === null || orgs.get(`${thread.server}\n${thread.repo}`) === workspace.org)
      )
    case 'workspace':
      return thread.workspace === workspace.id
  }
}

/** A workspace as a `<select>` value, and back. */
export function workspaceValue(workspace: Workspace): string {
  if (workspace.kind === 'workspace') return `workspace\n${workspace.id}\n${workspace.server ?? ''}`
  return workspace.kind === 'server' ? `server\n${workspace.server}\n${workspace.org ?? ''}` : workspace.kind
}

export function workspaceOf(value: string): Workspace {
  const [kind, server, org] = value.split('\n')
  if (kind === 'workspace' && server) return { kind: 'workspace', id: server, server: org || null }
  if (kind === 'server' && server) return { kind: 'server', server, org: org || null }
  return kind === 'local' ? { kind: 'local' } : { kind: 'all' }
}
