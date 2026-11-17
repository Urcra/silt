// The person's say before a page acts for them (ADR 0072): asked once per load. A yes is kept for what the version
// they saw declares of the capability, so a later version declaring more asks again; the daemon and the server check
// the same grant, so a document the frame navigated to gets nothing it wasn't given. A no holds for the page's load
// only, so it's kept here rather than stored.

import { api, type ArtifactView } from '../api'
import type { Holder } from './capabilities'

const refusals = new WeakMap<Holder, Set<string>>()

/** What the viewer refused the page holding `holder` while it's been loaded. */
export function refusedHere(holder: Holder): Set<string> {
  let refused = refusals.get(holder)
  if (!refused) {
    refused = new Set()
    refusals.set(holder, refused)
  }
  return refused
}

export function consentFor(
  thread: string,
  artifact: ArtifactView,
  holder: Holder,
  capability: string,
  ask: () => { title: string; detail: string; action: string },
): () => Promise<boolean> {
  let consent: Promise<boolean> | null = null
  return () =>
    (consent ??= (async () => {
      const grants = await api.artifactGrants(thread, artifact.id).catch(() => ({}) as Record<string, string>)
      if (grants[capability] === 'granted') return true
      if (refusedHere(holder).has(capability)) return false
      const agreed = await holder.confirm(ask())
      if (agreed) await api.setArtifactGrant(thread, artifact.id, capability, 'granted', artifact.version).catch(() => undefined)
      else refusedHere(holder).add(capability)
      return agreed
    })())
}
