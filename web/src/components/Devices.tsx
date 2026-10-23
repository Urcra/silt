import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { type ReactNode, useState } from 'react'
import { STATE_TAG } from '../agentState'
import { api, type Capabilities, type DeviceChange, type DeviceView, type PhonePairing } from '../api'
import { useNow } from '../now'
import { keys, message } from '../query'
import { ago } from '../time'
import { Button, Dot, EmptyState, Icon, Input, KeyValue, Skeleton, Tag, buttonClass, cx, toast } from '../ui'
import { Mock } from './Mock'
import { Page } from './Page'

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
  const now = useNow()
  const queries = useQueryClient()
  const change = useDeviceChange()
  const [name, setName] = useState<string | null>(null)
  const phone = device.kind === 'phone'
  const rows: [ReactNode, ReactNode][] = phone
    ? [
        ['Can', CAPABILITIES.filter((capability) => device.capabilities[capability.id]).map((capability) => capability.label.toLowerCase()).join(', ') || 'only read'],
        ['Last seen', <span key="seen" data-volatile>{ago(Number(device.last_used_ms), now)}</span>],
      ]
    : [
        [
          'Runs',
          device.agents.length === 0
            ? 'no agents'
            : `${device.agents.length} ${device.agents.length === 1 ? 'agent' : 'agents'}`,
        ],
        ...(!device.online ? ([['Last seen', <span key="seen" data-volatile>{ago(Number(device.last_used_ms), now)}</span>]] as [ReactNode, ReactNode][]) : []),
        ['Checkouts', <Mock key="checkouts" id="devices.checkouts">3</Mock>],
        ['Daemon', <Mock key="daemon" id="devices.version">0.0.1 · synced</Mock>],
      ]
  return (
    <section aria-label={device.name} className="grid grid-cols-[20px_minmax(0,1fr)] gap-x-3 border-b border-line-subtle px-4 py-3.5 last:border-b-0">
      <Icon name={phone ? 'phone' : 'laptop'} className="mt-0.5 h-5 w-5 text-fg-2" />
      <div className="flex min-w-0 flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        {name === null ? (
          <button type="button" className="text-sm font-medium hover:text-accent-fg max-md:min-h-11" title="rename" onClick={() => setName(device.name)}>
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
        {device.this && <span className="text-xs text-fg-2">this device</span>}
        {phone && <span className="text-xs text-fg-2">phone</span>}
        <span className="ml-auto inline-flex items-center gap-1.5 text-xs">
          <Dot tone={device.online ? 'ok' : 'idle'} />
          {device.online ? 'online' : 'offline'}
        </span>
        {!device.this && (
          <Button
            variant="ghost"
            size="sm"
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
        )}
      </div>
      <KeyValue rows={rows} />
      {device.agents.length > 0 && (
        <ul aria-label={`running on ${device.name}`} className="flex flex-col gap-1 text-xs">
          {device.agents.map((agent) => (
            <li key={agent.thread} className="flex items-center gap-2">
              <Link to="/t/$id" params={{ id: agent.thread }} className="truncate text-accent-fg hover:text-fg-1 max-md:inline-flex max-md:min-h-11 max-md:items-center">
                {agent.title ?? 'Untitled thread'}
              </Link>
              <span className="font-mono text-2xs text-fg-2">{agent.agent}</span>
              <Tag tone={STATE_TAG[agent.state].tone}>{STATE_TAG[agent.state].label}</Tag>
            </li>
          ))}
        </ul>
      )}
      <details className="group/may text-xs">
        <summary className="inline-flex cursor-pointer list-none items-center gap-1.5 text-fg-2 hover:text-fg-1 max-md:min-h-11 [&::-webkit-details-marker]:hidden">
          <Icon name="chevronRight" className="h-3 w-3 transition-transform group-open/may:rotate-90" />
          What it may do on your other devices
        </summary>
        <fieldset className="mt-2 flex flex-wrap gap-x-4 gap-y-1.5">
          <legend className="sr-only">From this device you may, on your other devices:</legend>
          {CAPABILITIES.map((capability) => (
            <label key={capability.id} className="flex items-center gap-1.5 max-md:min-h-11">
              <input
                type="checkbox"
                checked={device.capabilities[capability.id]}
                className="accent-[var(--accent)]"
                onChange={(event) =>
                  void change(device, { capabilities: { ...device.capabilities, [capability.id]: event.target.checked } }, 'Saved')
                }
              />
              {capability.label}
            </label>
          ))}
        </fieldset>
      </details>
      </div>
    </section>
  )
}

/** A QR code and a short code that sign a phone in, for ten minutes (ADR 0028). */
function PairPhone() {
  const now = useNow(1000)
  const queries = useQueryClient()
  const [pairing, setPairing] = useState<PhonePairing>()
  const start = () => void api.pairPhone().then(setPairing, (failure: unknown) => toast(message(failure), 'danger'))
  const left = pairing ? Math.max(0, Math.round((Number(pairing.expires_ms) - now) / 1000)) : 0
  return (
    <section aria-label="pair a phone" className="flex flex-col gap-3 rounded-md border border-line bg-elevated p-4">
      <div className="flex items-center gap-2">
        <h2 className="text-sm font-medium">Pair a phone</h2>
        <Button className="ml-auto" onClick={start}>
          {pairing ? 'New code' : 'Show a code'}
        </Button>
      </div>
      <div className="flex flex-wrap items-start gap-4">
        {pairing ? (
          <img
            alt="QR code to pair a phone"
            className="h-40 w-40 rounded-md border border-line bg-[var(--paper-50)] p-1.5"
            src={`data:image/svg+xml;utf8,${encodeURIComponent(pairing.qr_svg)}`}
          />
        ) : (
          <div className="flex h-40 w-40 items-center justify-center rounded-md border border-dashed border-line text-center text-2xs text-fg-2">
            A code shows here
          </div>
        )}
        <ol className="flex flex-[1_1_140px] list-decimal flex-col gap-1 pl-4 text-xs leading-normal">
          <li>Scan with the phone’s camera{pairing && <>, or open {pairing.url.replace(/\/pair\/.*$/, '/pair')} and type the code</>}.</li>
          <li>Sign in on the phone as you do here.</li>
          <li>Check the code matches, then approve.</li>
        </ol>
      </div>
      {pairing && (
        <div className="flex flex-wrap items-baseline gap-3">
          <span className="font-mono text-lg tracking-[0.12em]" data-testid="pairing-code">
            {pairing.code}
          </span>
          <span className="text-xs text-fg-2" data-volatile>
            {left > 0 ? `expires in ${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}` : 'expired; show a new code'}
          </span>
          <Button className="ml-auto" onClick={() => void queries.invalidateQueries({ queryKey: keys.devices })}>
            It's paired
          </Button>
        </div>
      )}
      <fieldset className="flex flex-col gap-1.5 text-xs">
        <legend className="mb-1.5 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">The phone may</legend>
        <Mock id="devices.pair-capabilities">
          {[
            ['Answer agents’ permission requests', true],
            ['Approve and decline merge requests', true],
            ['Send prompts and start threads', false],
          ].map(([label, on]) => (
            <label key={label as string} className="flex items-center gap-1.5 text-fg-2 max-md:min-h-11">
              <input type="checkbox" checked={on as boolean} disabled className="accent-[var(--accent)]" />
              {label as string}
            </label>
          ))}
        </Mock>
        <p className="text-2xs text-fg-2">Give it more from its card once it’s paired.</p>
      </fieldset>
      <Link to="/needs-you" className={cx(buttonClass('secondary'), 'self-start')}>
        See the phone view
      </Link>
    </section>
  )
}

/** The person's devices: which are online, what runs on each, what each may do (ADR 0026, the Devices board). */
export function Devices() {
  const devices = useQuery({ queryKey: keys.devices, queryFn: api.devices })
  return (
    <Page flush label="devices">
      <div className="flex flex-col gap-6 px-4 py-6 md:px-8 lg:flex-row lg:items-start">
        <div className="flex min-w-0 flex-1 flex-col gap-3">
          <h2 id="your-devices" className="text-sm font-medium">
            Your devices
          </h2>
          <p className="text-sm text-fg-2">Threads move between these. A device with a checkout can run agents; a phone can answer and review.</p>
          {devices.isPending && <Skeleton className="h-24" />}
          {devices.error && <p className="text-danger">{message(devices.error)}</p>}
          {devices.data?.length === 0 && <EmptyState title="Sign a machine in with `silt auth login`">It shows here, with what runs on it.</EmptyState>}
          {devices.data && devices.data.length > 0 && (
            <div className="overflow-hidden rounded-md border border-line bg-elevated">
              {[...devices.data]
                .sort((a, b) => Number(b.this) - Number(a.this))
                .map((device) => (
                  <Device key={device.id} device={device} />
                ))}
            </div>
          )}
        </div>
        <div className="lg:w-[380px] lg:shrink-0">
          <PairPhone />
        </div>
      </div>
    </Page>
  )
}
