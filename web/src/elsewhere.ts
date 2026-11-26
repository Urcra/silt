// Every connected machine's threads in one list (ADR 0132): the desktop app hands the page each other
// machine's listing, and the sidebar lists them beside this machine's own.

import type { ThreadSummary } from './api'
import { type Backend, type Listing, type MachineColour, type Mark, useBackends, useListings, useMachinesView } from './desktop'
import { needsYou } from './sections'
import { inProgress } from './threadLook'

/** A thread as the sidebar lists it: on another machine when `backend` is set. */
export type Listed = ThreadSummary & { backend?: string }

/** The backend a listed thread is on, when it isn't the one the window shows. */
export function backendOf(thread: ThreadSummary): string | undefined {
  return (thread as Listed).backend
}

/**
 * This machine's threads with the other machines', each of those marked with its backend. A thread on two
 * machines through a server is listed once, from the machine that holds it.
 */
export function merged(local: ThreadSummary[], listings: Listing[], current: string | undefined): Listed[] {
  const byId = new Map<string, Listed>(local.map((thread) => [thread.id, thread]))
  for (const listing of listings) {
    if (listing.id === current || listing.state !== 'ready') continue
    for (const thread of listing.threads) {
      const known = byId.get(thread.id)
      if (known && (known.location === 'here' || thread.location !== 'here')) continue
      byId.set(thread.id, { ...thread, backend: listing.id })
    }
  }
  return [...byId.values()]
}

/** What the sidebar shows of the other machines: their threads beside `local`, and which are loading or lost. */
export function useAllMachines(local: ThreadSummary[]): { threads: Listed[]; loading: Backend[]; lost: Backend[]; all: boolean } {
  const view = useMachinesView()
  const listings = useListings()
  const { backends, current } = useBackends()
  const all = view.list === 'all' && backends.length > 0
  if (!all) return { threads: local, loading: [], lost: [], all }
  const named = (state: Listing['state']) =>
    listings
      .filter((listing) => listing.state === state && listing.id !== current)
      .map((listing) => backends.find((backend) => backend.id === listing.id))
      .filter((backend): backend is Backend => backend !== undefined)
  return { threads: merged(local, listings, current), loading: named('loading'), lost: named('lost'), all }
}

/** A machine's name as its threads are marked with it: This computer's own name, a distribution without "WSL: ". */
export function machineName(backend: Backend): string {
  if (backend.kind === 'native' && backend.host) return backend.host
  return backend.kind === 'wsl' ? backend.label.replace(/^WSL: /, '') : backend.label
}

/** The CSS colour of a machine's colour. */
export function colourOf(colour: MachineColour | undefined): string {
  return `var(--machine-${colour ?? 'ink'})`
}

/** Each other machine's threads that run and that wait on the person, as its row in the machine menu says. */
export function countsOf(listing: Listing | undefined): { running: number; needsYou: number } {
  const threads = listing?.state === 'ready' ? listing.threads.filter((thread) => thread.location === 'here') : []
  const running = threads.filter(inProgress).length
  const waiting = threads.filter(needsYou).length
  return { running, needsYou: waiting }
}

/** How a thread's machine is marked in the list, if it is (ADR 0132). */
export interface MachineMarked {
  name: string
  colour: string
  mark: Mark
}

/** The mark of the machine a listed thread is on: another machine's, or this one's when the person marks it too. */
export function useMachineMark(thread: ThreadSummary): MachineMarked | null {
  const view = useMachinesView()
  const { backends, current } = useBackends()
  if (view.list !== 'all' || view.mark === 'nothing') return null
  const on = backendOf(thread) ?? (view.markHere ? current : undefined)
  const backend = on === undefined ? undefined : backends.find((known) => known.id === on)
  if (!backend) return null
  return { name: machineName(backend), colour: colourOf(backend.colour), mark: view.mark }
}
