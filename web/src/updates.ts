// What Settings › Updates and the sidebar show of an update (ADR 0098), from
// what the daemon says: the build, the last check and the last run.

import { useQuery } from '@tanstack/react-query'
import { api, type OffReason, type RunView, type UpdateView } from './api'
import { desktopCommit } from './desktop'
import { keys } from './query'
import { hosted } from './served'

export type UpdateState =
  | 'running'
  | 'waiting'
  | 'failed'
  | 'relaunch'
  | 'behind'
  | 'release'
  | 'done'
  | 'off'
  | 'up-to-date'
  | 'unknown'

/** Whether two commits are the same, one perhaps abbreviated. */
export function sameCommit(a: string | null | undefined, b: string | null | undefined): boolean {
  return !!a && !!b && (a.startsWith(b) || b.startsWith(a))
}

/** The run, if it's under way. */
export function inFlight(run: RunView | null | undefined): boolean {
  return run?.outcome.kind === 'running' || run?.outcome.kind === 'waiting'
}

export function updateState(view: UpdateView | undefined): UpdateState {
  if (!view) return 'unknown'
  const { run, availability, build } = view
  if (run?.outcome.kind === 'running') return 'running'
  if (run?.outcome.kind === 'waiting') return 'waiting'
  // A failure says something only while this is still the build it failed to replace.
  if (run?.outcome.kind === 'failed' && sameCommit(run.from, build.commit)) return 'failed'
  const behind = availability?.kind === 'behind'
  if (view.relaunch && !behind) return 'relaunch'
  if (behind) return 'behind'
  if (availability?.kind === 'release') return 'release'
  if (run?.outcome.kind === 'done' && sameCommit(run.to, build.commit)) return 'done'
  if (availability?.kind === 'off') return 'off'
  if (availability?.kind === 'up_to_date') return 'up-to-date'
  return 'unknown'
}

/** What the sidebar's button says, if anything. */
export type Pill = { kind: 'update' | 'running' | 'waiting' | 'relaunch' | 'failed'; text: string }

export function pillOf(view: UpdateView | undefined): Pill | undefined {
  const state = updateState(view)
  switch (state) {
    case 'behind':
    case 'release':
      return { kind: 'update', text: 'Update' }
    case 'running':
      return { kind: 'running', text: `Updating · ${view?.run?.percent ?? 0}%` }
    case 'waiting':
      return { kind: 'waiting', text: 'Restart pending' }
    case 'relaunch':
      return { kind: 'relaunch', text: 'Relaunch' }
    case 'failed':
      return { kind: 'failed', text: 'Update failed' }
    default:
      return undefined
  }
}

/** Why there are no updates, in a sentence. */
export function offReason(reason: OffReason): string {
  switch (reason.kind) {
    case 'debug_build':
      return 'This is a debug build. Install one with `just install` to get updates.'
    case 'no_clone':
      return "This build doesn't say which clone it was built from."
    case 'clone_gone':
      return `The clone it was built from, ${reason.path}, is gone.`
    case 'built_off_main':
      return reason.branch ? `It was built from \`${reason.branch}\`, not \`main\`.` : 'It was built from a detached HEAD, not `main`.'
    case 'built_with_changes':
      return 'It was built with local changes, not from a clean `main`.'
    case 'off_main':
      return reason.branch ? `Its clone is on \`${reason.branch}\`, not \`main\`.` : 'Its clone has a detached HEAD, not `main`.'
    case 'local_changes':
      return 'Its clone has uncommitted changes.'
    case 'no_upstream':
      return '`main` in its clone has no upstream to follow.'
    case 'local_commits':
      return `\`main\` in its clone has ${reason.count} ${reason.count === 1 ? 'commit' : 'commits'} its upstream lacks.`
    case 'unknown_build':
      return "Its clone doesn't have the commit it was built from on `main`."
  }
}

/** A duration as the update screen says it: `2s`, `3m 40s`. */
export function took(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000))
  const minutes = Math.floor(seconds / 60)
  return minutes > 0 ? `${minutes}m ${seconds % 60}s` : `${seconds}s`
}

/** The daemon's update view: polled each second while something moves, else every ten minutes. */
export function useUpdate() {
  return useQuery({
    queryKey: keys.update,
    queryFn: () => api.update(desktopCommit()),
    enabled: !hosted(),
    // Kept through the daemon's restart, whose page picks up where it was.
    retry: false,
    // A failed ask polls on only while an update runs: that's the daemon restarting.
    refetchInterval: (query) => {
      const view = query.state.data
      return inFlight(view?.run) || view?.checking ? 1000 : 600_000
    },
  })
}
