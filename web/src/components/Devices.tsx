import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { STATE_TAG } from '../agentState'
import { api, type Capabilities, type DeviceChange, type DeviceView, type PhonePairing } from '../api'
import { keys, message } from '../query'
import { Button, EmptyState, Input, Skeleton, Tag, toast } from '../ui'

const CAPABILITIES: { id: keyof Capabilities; label: string }[] = [
  { id: 'answer_permissions', label: 'Answer permissions' },
  { id: 'send_prompts', label: 'Send prompts' },
  { id: 'run_agents', label: 'Start and stop agents' },
  { id: 'review', label: 'Review merge requests' },
]

function useDeviceChange() {
  const queries = useQueryClient()
  return (device: DeviceView, change: Partial<DeviceChange>, said: string) =>
    api.changeDevice(device.id, { name: null, capabilities: null, ...change }).then(
      () => {
        toast(said, 'ok')
        return queries.invalidateQueries({ queryKey: keys.devices })
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
}

function Device({ device }: { device: DeviceView }) {
  const queries = useQueryClient()
  const change = useDeviceChange()
  const [name, setName] = useState<string | null>(null)
  return (
    <section aria-label={device.name} className="space-y-3 rounded-md border border-line p-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        {name === null ? (
          <button className="text-base font-medium" title="rename" onClick={() => setName(device.name)}>
            {device.name}
          </button>
        ) : (
          <form
            className="flex items-center gap-2"
            onSubmit={(event) => {
              event.preventDefault()
              void change(device, { name }, 'Renamed').then(() => setName(null))
            }}
          >
            <Input aria-label="device name" autoFocus value={name} onChange={(event) => setName(event.target.value)} />
            <Button type="submit" variant="primary" disabled={!name.trim()}>
              Save
            </Button>
            <Button onClick={() => setName(null)}>Cancel</Button>
          </form>
        )}
        {device.kind === 'phone' ? (
          <Tag tone="info">phone</Tag>
        ) : (
          <Tag tone={device.online ? 'ok' : 'neutral'}>{device.online ? 'online' : 'offline'}</Tag>
        )}
        {device.this && <Tag tone="accent">this device</Tag>}
        <span className="ml-auto text-xs text-fg-3">signed in {new Date(Number(device.created_ms)).toLocaleDateString()}</span>
      </div>
      {device.agents.length > 0 && (
        <ul aria-label={`running on ${device.name}`} className="space-y-1">
          {device.agents.map((agent) => (
            <li key={agent.thread} className="flex items-center gap-2">
              <Link to="/t/$id" params={{ id: agent.thread }} className="truncate text-accent-fg">
                {agent.title ?? 'Untitled thread'}
              </Link>
              <span className="text-fg-3">{agent.agent}</span>
              <Tag tone={STATE_TAG[agent.state].tone}>{STATE_TAG[agent.state].label}</Tag>
            </li>
          ))}
        </ul>
      )}
      <fieldset className="flex flex-wrap gap-x-4 gap-y-1">
        <legend className="mb-1 text-xs text-fg-2">From this device you may, on your other devices:</legend>
        {CAPABILITIES.map((capability) => (
          <label key={capability.id} className="flex items-center gap-1 text-xs">
            <input
              type="checkbox"
              checked={device.capabilities[capability.id]}
              onChange={(event) =>
                void change(device, { capabilities: { ...device.capabilities, [capability.id]: event.target.checked } }, 'Saved')
              }
            />
            {capability.label}
          </label>
        ))}
      </fieldset>
      <Button
        variant="danger"
        onClick={() =>
          void api.revokeDevice(device.id).then(
            () => {
              toast(`${device.name} is signed out`, 'ok')
              return queries.invalidateQueries({ queryKey: keys.devices })
            },
            (failure: unknown) => toast(message(failure), 'danger'),
          )
        }
      >
        Sign out
      </Button>
    </section>
  )
}

/** A QR code and a short code that sign a phone in, for ten minutes (ADR 0028). */
function PairPhone() {
  const queries = useQueryClient()
  const [pairing, setPairing] = useState<PhonePairing>()
  const start = () =>
    void api.pairPhone().then(setPairing, (failure: unknown) => toast(message(failure), 'danger'))
  return (
    <section aria-label="pair a phone" className="space-y-2 rounded-md border border-line p-3 text-sm">
      <div className="flex items-center gap-2">
        <h3 className="font-medium">Pair a phone</h3>
        <Button className="ml-auto" onClick={start}>
          {pairing ? 'New code' : 'Show a code'}
        </Button>
      </div>
      {pairing && (
        <div className="flex flex-wrap items-center gap-4">
          <img alt="QR code to pair a phone" className="h-44 w-44 rounded bg-white p-1" src={`data:image/svg+xml;utf8,${encodeURIComponent(pairing.qr_svg)}`} />
          <div className="space-y-1">
            <p>Scan it with the phone's camera, or open {pairing.url.replace(/\/pair\/.*$/, '/pair')} and type</p>
            <p className="font-mono text-lg tracking-widest" data-testid="pairing-code">
              {pairing.code}
            </p>
            <p className="text-xs text-fg-3">
              It works once, until {new Date(Number(pairing.expires_ms)).toLocaleTimeString()}. The phone may answer permissions and review;
              give it more below once it's paired.
            </p>
            <Button className="text-xs" onClick={() => void queries.invalidateQueries({ queryKey: keys.devices })}>
              It's paired
            </Button>
          </div>
        </div>
      )}
    </section>
  )
}

/** The person's devices: which are online, what runs on each, what each may do (ADR 0026). */
export function Devices() {
  const devices = useQuery({ queryKey: keys.devices, queryFn: api.devices })
  return (
    <div className="mx-auto max-w-3xl space-y-4">
      <h2 className="text-lg font-semibold">Devices</h2>
      <PairPhone />
      {devices.isPending && <Skeleton className="h-24" />}
      {devices.error && <p className="text-danger">{message(devices.error)}</p>}
      {devices.data?.length === 0 && <EmptyState title="No devices">Sign one in with `silt auth login`.</EmptyState>}
      {devices.data?.map((device) => <Device key={device.id} device={device} />)}
    </div>
  )
}
