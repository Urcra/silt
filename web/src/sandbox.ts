import { useQuery } from '@tanstack/react-query'
import { api, type Devices, type Extra, type Network, type Policy, type Reach } from './api'
import { keys } from './query'
import { hosted } from './served'

/** Silt's sandbox in the web UI (ADR 0086): its options as Settings shows them, and what a policy gives a run. */

export const NETWORKS: { value: Network; label: string; detail: string }[] = [
  { value: 'none', label: 'No network', detail: 'Nothing leaves the sandbox. For work on untrusted content.' },
  { value: 'gateway', label: 'Through Silt only', detail: "HTTP(S) through Silt's gateway, to the hosts on its list; it adds the credentials." },
  { value: 'full', label: 'Full internet', detail: "Any public host; never this machine, its networks or cloud metadata. Credentials are still added by Silt's gateway." },
  { value: 'host', label: "This machine's network", detail: "As if it ran outside the sandbox: everything on this machine's 127.0.0.1, Silt's own web UI included." },
]

export const DEVICES: { value: Devices; label: string; detail: string }[] = [
  { value: 'none', label: 'None', detail: 'No devices.' },
  { value: 'named', label: 'Named devices', detail: 'The device nodes themselves, inside the sandbox.' },
  { value: 'all_usb', label: 'All USB', detail: 'Every USB device, for tools that talk to them directly (libusb).' },
]

export const NETWORK_ORDER: Network[] = ['none', 'gateway', 'full', 'host']
export const DEVICE_ORDER: Devices[] = ['none', 'named', 'all_usb']

/** What turning it on hands the agent, as `silt-workspace`'s `Extra::hands_over` says it. */
export const HANDS_OVER: Record<Extra, string> = {
  ranges: "hosts on those networks, a VPN's included",
  ports: 'whatever listens on those ports',
  paths: "those folders' files",
  gpu: "the GPU's devices",
  docker: 'root on this machine: the agent could do anything you can',
  ssh_agent: 'signing as you, with every key loaded, to any host',
}

/** The reach a policy gives, as `Reach::of` works it out, for a run not started yet. */
export function reachOf(policy: Policy): Reach {
  const allowed = (extra: Extra) => !(policy.refuse ?? []).includes(extra)
  const devices = policy.devices ?? 'none'
  return {
    network: policy.network ?? 'full',
    hosts: policy.hosts ?? [],
    devices,
    device_nodes: devices === 'named' ? (policy.device_nodes ?? []) : [],
    ranges: allowed('ranges') ? (policy.ranges ?? []) : [],
    ports: allowed('ports') ? (policy.ports ?? []) : [],
    paths: allowed('paths') ? (policy.paths ?? {}) : {},
    gpu: Boolean(policy.gpu) && allowed('gpu'),
    docker: Boolean(policy.docker) && allowed('docker'),
    ssh_agent: Boolean(policy.ssh_agent) && allowed('ssh_agent'),
    nested: !policy.no_nested,
  }
}

/** How many plugin calls wait for the person, for Needs you's empty state. */
export function useApprovalsWaiting(): number {
  const approvals = useQuery({ queryKey: keys.allApprovals, queryFn: () => api.approvals(), enabled: !hosted(), refetchInterval: 30_000 })
  return (approvals.data ?? []).filter((approval) => approval.state === 'pending').length
}
