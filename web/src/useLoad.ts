import { useEffect, useRef, useState } from 'react'

export interface Loaded<T> {
  data: T | undefined
  error: string | undefined
  reload: () => void
}

/** Loads `load()` whenever `key` changes, and again on `reload()`. */
export function useLoad<T>(load: () => Promise<T>, key: string): Loaded<T> {
  const [state, setState] = useState<{ key: string; round: number; data?: T; error?: string }>({ key, round: 0 })
  const [round, setRound] = useState(0)
  const latest = useRef(load)
  useEffect(() => {
    latest.current = load
  })
  useEffect(() => {
    let current = true
    latest.current().then(
      (data) => current && setState({ key, round, data }),
      (error: unknown) =>
        current && setState({ key, round, error: error instanceof Error ? error.message : String(error) }),
    )
    return () => {
      current = false
    }
  }, [key, round])
  const fresh = state.key === key
  return {
    data: fresh ? state.data : undefined,
    error: fresh ? state.error : undefined,
    reload: () => setRound((value) => value + 1),
  }
}
