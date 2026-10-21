import { useEffect, useState } from 'react'
import { api, type AgentStatus } from './api'
import { useBusEvents, useTopic } from './bus'

/** How the thread's agent stands here: fetched once, then followed on the event bus. */
export function useAgentStatus(thread: string): AgentStatus | null {
  const [status, setStatus] = useState<AgentStatus | null>(null)
  const [round, setRound] = useState(0)
  useEffect(() => {
    let current = true
    api.live(thread).then(
      (live) => current && setStatus(live.agent),
      () => undefined,
    )
    return () => {
      current = false
    }
  }, [thread, round])
  useTopic(`thread:${thread}`)
  useBusEvents((event) => {
    if (event.event === 'lagged') setRound((value) => value + 1)
    if (event.event === 'agent' && event.thread === thread) setStatus(event.status)
  })
  return status
}
