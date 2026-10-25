import type { ThreadSummary } from './api'

/** Threads by project: the repositories they were recorded in, for now (ADR 0020). */
export function projects(threads: ThreadSummary[]): [string, ThreadSummary[]][] {
  const byRepo = new Map<string, ThreadSummary[]>()
  for (const thread of [...threads].sort((a, b) => b.created_ms - a.created_ms)) {
    const repo = thread.repo || 'no remote'
    byRepo.set(repo, [...(byRepo.get(repo) ?? []), thread])
  }
  // The project last worked in first, as the person comes back to it; by name between equals.
  const latest = (inRepo: ThreadSummary[]) => Math.max(...inRepo.map((thread) => thread.last_ms))
  return [...byRepo.entries()].sort(([a, inA], [b, inB]) => latest(inB) - latest(inA) || a.localeCompare(b))
}

/** The last part of a repository's identity, its name. */
export function projectName(repo: string): string {
  return repo.split('/').filter(Boolean).pop() ?? repo
}
