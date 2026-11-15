// The capabilities this view serves an artifact's page, by name (ADR 0072). A page that asks for one not here gets
// null.

import type { ArtifactView } from '../api'
import type { CapabilityHandler } from './broker'
import { dbHandler } from './db'

export function handlersFor(thread: string, artifact: ArtifactView): Record<string, CapabilityHandler> {
  return { db: dbHandler(thread, artifact) }
}
