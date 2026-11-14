import { useEffect, useRef } from 'react'

/** A send that, made while what it needs is still loading, goes once that's in rather than be lost. */
export function useHeldSend(loading: boolean, ready: boolean, go: () => void): () => void {
  const held = useRef(false)
  useEffect(() => {
    if (!held.current || loading) return
    held.current = false
    if (ready) go()
  })
  return () => {
    if (loading) held.current = true
    else if (ready) go()
  }
}
