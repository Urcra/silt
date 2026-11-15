// The capabilities this view serves an artifact's page, by name (ADR 0072). Each phase of the bridge adds its own;
// a page that asks for one not here gets null.

import type { ArtifactView } from '../api'
import type { CapabilityHandler } from './broker'

export function handlersFor(thread: string, artifact: ArtifactView): Record<string, CapabilityHandler> {
  void thread
  void artifact
  return {}
}
