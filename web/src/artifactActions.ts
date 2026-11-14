import { useQuery } from '@tanstack/react-query'
import { createContext } from 'react'
import { api, type ArtifactView } from './api'
import { keys } from './query'

/** Which artifact the panel shows, and which version: the latest when `version` is null. */
export type Opened = { id: string; version: number | null }

/** Opens an artifact in the thread's panel, from a card anywhere in the conversation. */
export const ArtifactActions = createContext<{ thread: string; open: (id: string, version?: number | null) => void; opened: Opened | null } | null>(null)

export function useArtifactContent(thread: string, artifact: ArtifactView) {
  // A blob never changes, so neither does what it holds.
  return useQuery({ queryKey: keys.artifact(thread, artifact.blob), queryFn: () => api.artifact(thread, artifact.blob), staleTime: Infinity })
}
