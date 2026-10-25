import { useQuery } from '@tanstack/react-query'
import { api } from './api'
import { keys } from './query'
import { hosted } from './served'

/** Whether there's a server to ask, for what only a server has: people, devices, notifications. A machine without one works on its own. */
export function useHasServer(): boolean {
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  return hosted() || (status.data?.servers.length ?? 0) > 0
}
