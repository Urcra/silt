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

const FORGES: [string, string][] = [
  ['github.com', 'GitHub'],
  ['gitlab.com', 'GitLab'],
  ['codeberg.org', 'Codeberg'],
  ['bitbucket.org', 'Bitbucket'],
]

/** The forge a repository's remote is on, by name, when it is a known one. */
export function forgeOf(repo: string): string | undefined {
  const host = repo.split('/')[0]?.toLowerCase()
  return FORGES.find(([known]) => host === known)?.[1]
}

/** A folder's path as it reads in a sentence: from the home folder as `~`, and only its last parts when it runs long. */
export function shortPath(path: string): string {
  const home = path.replace(/^(\/home\/[^/]+|\/Users\/[^/]+|[A-Za-z]:\\Users\\[^\\]+)(?=[/\\]|$)/, '~')
  const parts = home.split(/[/\\]/)
  return parts.length > 4 ? `…/${parts.slice(-3).join('/')}` : home
}

/** The last part of a repository's identity, its name. */
export function projectName(repo: string): string {
  return repo.split('/').filter(Boolean).pop() ?? repo
}
