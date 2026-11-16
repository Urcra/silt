// The person's say before a page acts for them (ADR 0072): asked once per load, and kept for what the version they
// saw declares of the capability, so a later version declaring more asks again. The daemon and the server check the
// same grant, so a document the frame navigated to gets nothing it wasn't given.

import { api, type ArtifactView } from '../api'
import type { Holder } from './capabilities'

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
      if (grants[capability] === 'refused') return false
      const agreed = await holder.confirm(ask())
      await api.setArtifactGrant(thread, artifact.id, capability, agreed ? 'granted' : 'refused', artifact.version).catch(() => undefined)
      return agreed
    })())
}
