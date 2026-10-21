import type { ThreadSummary } from './api'

/** Threads by project: the repositories they were recorded in, for now (ADR 0020). */
export function projects(threads: ThreadSummary[]): [string, ThreadSummary[]][] {
  const byRepo = new Map<string, ThreadSummary[]>()
  for (const thread of [...threads].sort((a, b) => b.created_ms - a.created_ms)) {
    const repo = thread.repo || 'no remote'
    byRepo.set(repo, [...(byRepo.get(repo) ?? []), thread])
  }
  return [...byRepo.entries()].sort(([a], [b]) => a.localeCompare(b))
}

/** The last part of a repository's identity, its name. */
export function projectName(repo: string): string {
  return repo.split('/').filter(Boolean).pop() ?? repo
}
