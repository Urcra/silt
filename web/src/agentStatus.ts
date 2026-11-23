import { useEffect, useRef, useState } from 'react'
import { api, type AgentStatus } from './api'
import { useBusEvents, useTopic } from './bus'

/** How the thread's agent stands here: fetched once, then followed on the event bus. */
export function useAgentStatus(thread: string): AgentStatus | null {
  const [status, setStatus] = useState<AgentStatus | null>(null)
  const [round, setRound] = useState(0)
  // Subscribed before the fetch, so what changes once the daemon has answered it is heard.
  useTopic(`thread:${thread}`)
  const heard = useRef(0)
  useEffect(() => {
    let current = true
    const before = heard.current
    api.live(thread).then(
      // A status heard since the fetch went out is at least as new as its answer, which can come after it.
      (live) => current && heard.current === before && setStatus(live.agent),
      () => undefined,
    )
    return () => {
      current = false
    }
  }, [thread, round])
  useBusEvents((event) => {
    if (event.event === 'lagged') setRound((value) => value + 1)
    if (event.event === 'agent' && event.thread === thread) {
      heard.current += 1
      setStatus(event.status)
    }
  })
  return status
}
