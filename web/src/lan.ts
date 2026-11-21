// Phones on the local network (ADR 0112): what the machine's UI and a paired phone each ask of the daemon.

import { useQuery } from '@tanstack/react-query'
import { api } from './api'
import { keys } from './query'
import { hosted, onLan } from './served'

/** The network listener's state, on the machine's own UI only: a phone may not read it. */
export function useLan() {
  return useQuery({ queryKey: keys.lan, queryFn: api.lan, enabled: !hosted() && !onLan() })
}

/** Which phone this is, on a phone paired over the network. */
export function useLanMe() {
  return useQuery({ queryKey: ['lan-me'], queryFn: api.lanMe, enabled: onLan(), retry: false })
}

/** Whether this is a phone on the network that isn't paired, or no longer is. */
export function useLanUnpaired(): boolean {
  const me = useLanMe()
  return onLan() && me.isError
}
