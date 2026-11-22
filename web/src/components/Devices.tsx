import { useQuery, useQueryClient } from '@tanstack/react-query'
import { AgentMark } from './AgentMark'
import { Link } from '@tanstack/react-router'
import { type ReactNode, useContext, useState } from 'react'
import { STATE_TAG } from '../agentState'
import { api, type Capabilities, type DeviceChange, type DeviceView, type LanPhone, type PhonePairing } from '../api'
import { useHasServer } from '../hasServer'
import { useNow } from '../now'
import { keys, message } from '../query'
import { mockRows } from '../mocks'
import { hosted, onLan } from '../served'
import { InSettings, SETTINGS_HEADING } from '../settingsContext'
import { ago } from '../time'
import { Button, Chip, Dot, EmptyState, Icon, Input, KeyValue, Skeleton, Tag, buttonClass, cx, toast } from '../ui'
import { useLan } from '../lan'
import { LanCard } from './LanPhones'
import { Mock } from './Mock'
import { Page } from './Page'

const CAPABILITIES: { id: keyof Capabilities; label: string }[] = [
  { id: 'answer_permissions', label: 'Answer permissions' },
  { id: 'send_prompts', label: 'Send prompts' },
  { id: 'run_agents', label: 'Start and stop agents' },
  { id: 'sample', label: "Ask an agent a page's question" },
  { id: 'page_tools', label: 'Call your page connectors' },
  { id: 'review', label: 'Review merge requests' },
]

function useDeviceChange(lanPhone: LanPhone | undefined) {
  const queries = useQueryClient()
  return (device: DeviceView, change: Partial<DeviceChange>, said: string) =>
    (lanPhone
      ? api.lanPhoneChange(device.id, { name: change.name ?? undefined, capabilities: change.capabilities ?? undefined })
      : api.changeDevice(device.id, { name: null, capabilities: null, ...change })
    ).then(
      () => {
        toast(said, 'ok')
        return queries.invalidateQueries({ queryKey: lanPhone ? keys.lan : keys.devices })
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
}

/** A phone paired over the network, as Devices lists every device (ADR 0112). */
function asDevice(phone: LanPhone): DeviceView {
  return {
    id: phone.id,
    kind: 'phone',
    name: phone.name,
    created_ms: phone.created_ms,
    last_used_ms: phone.last_seen_ms,
    online: phone.online,
    this: false,
    capabilities: phone.capabilities,
    agents: [],
    startable: [],
  }
}

function Device({ device, lanPhone }: { device: DeviceView; lanPhone?: LanPhone }) {
  const now = useNow()
  const queries = useQueryClient()
  const change = useDeviceChange(lanPhone)
  const [name, setName] = useState<string | null>(null)
  const phone = device.kind === 'phone'
  const rows: [ReactNode, ReactNode][] = lanPhone
    ? [
        [
          'Can',
          CAPABILITIES.filter((capability) => device.capabilities[capability.id])
            .map((capability) => capability.label.toLowerCase())
            .join(', ') || 'only read',
        ],
        ['Reaches', 'on this network'],
        ['Notifies', lanPhone.notifies ? 'on' : 'off: open Silt on it and allow notifications'],
        [
          'Last seen',
          <span key="seen" data-volatile>
            {ago(Number(device.last_used_ms), now)}
          </span>,
        ],
      ]
    : phone
      ? [
          [
            'Can',
            CAPABILITIES.filter((capability) => device.capabilities[capability.id])
              .map((capability) => capability.label.toLowerCase())
              .join(', ') || 'only read',
          ],
          [
            'Last seen',
            <span key="seen" data-volatile>
              {ago(Number(device.last_used_ms), now)}
            </span>,
          ],
        ]
      : [
          [
            'Runs',
            device.agents.length === 0
              ? 'no agents'
              : `${device.agents.length} ${device.agents.length === 1 ? 'agent' : 'agents'}`,
          ],
          ...(!device.online
            ? ([
                [
                  'Last seen',
                  <span key="seen" data-volatile>
                    {ago(Number(device.last_used_ms), now)}
                  </span>,
                ],
              ] as [ReactNode, ReactNode][])
            : []),
          ...mockRows<[ReactNode, ReactNode]>('devices.checkouts', [
            [
              'Checkouts',
              <Mock key="checkouts" id="devices.checkouts">
                3
              </Mock>,
            ],
          ]),
          ...mockRows<[ReactNode, ReactNode]>('devices.version', [
            [
              'Daemon',
              <Mock key="daemon" id="devices.version">
                0.0.1 · synced
              </Mock>,
            ],
          ]),
        ]
  return (
    <section
      aria-label={device.name}
      className="grid grid-cols-[20px_minmax(0,1fr)] gap-x-3 border-b border-line-subtle px-4 py-3.5 last:border-b-0"
    >
      <Icon name={phone ? 'phone' : 'laptop'} className="mt-0.5 h-5 w-5 text-fg-2" />
      <div className="flex min-w-0 flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2">
          {device.id === 'this' ? (
            <span className="text-sm font-medium">{device.name}</span>
          ) : name === null ? (
            <button
              type="button"
              className="text-sm font-medium hover:text-accent-fg max-md:min-h-11"
              title="rename"
              onClick={() => setName(device.name)}
            >
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
          {lanPhone && <Tag tone="info">this network</Tag>}
          <span className="ml-auto inline-flex items-center gap-1.5 text-xs">
            <Dot tone={device.online ? 'ok' : 'idle'} />
            {device.online ? 'online' : 'offline'}
          </span>
          {!device.this && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                void (lanPhone ? api.lanPhoneRevoke(device.id) : api.revokeDevice(device.id)).then(
                  () => {
                    toast(`${device.name} is signed out`, 'ok')
                    return queries.invalidateQueries({ queryKey: lanPhone ? keys.lan : keys.devices })
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
                <span className="flex text-fg-2" title={agent.agent}>
                  <AgentMark agent={agent.agent_id || agent.agent} name={agent.agent} labelled className="h-3.5 w-3.5" />
                </span>
                <Link
                  to="/t/$id"
                  params={{ id: agent.thread }}
                  className="truncate text-accent-fg hover:text-fg-1 max-md:inline-flex max-md:min-h-11 max-md:items-center"
                >
                  {agent.title ?? 'Untitled thread'}
                </Link>
                <Tag tone={STATE_TAG[agent.state].tone}>{STATE_TAG[agent.state].label}</Tag>
              </li>
            ))}
          </ul>
        )}
        {device.id !== 'this' && (
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
                      void change(
                        device,
                        { capabilities: { ...device.capabilities, [capability.id]: event.target.checked } },
                        'Saved',
                      )
                    }
                  />
                  {capability.label}
                </label>
              ))}
            </fieldset>
          </details>
        )}
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
          <li>
            Scan with the phone’s camera
            {pairing && <>, or open {pairing.url.replace(/\/pair\/.*$/, '/pair')} and type the code</>}.
          </li>
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
      <Mock id="devices.pair-capabilities">
        <fieldset className="flex flex-col gap-1.5 text-xs">
          <legend className="mb-1.5 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">The phone may</legend>
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
        </fieldset>
      </Mock>
      <p className="text-2xs text-fg-2">Give it more from its card once it’s paired.</p>
      <Link to="/needs-you" className={cx(buttonClass('secondary'), 'self-start')}>
        See the phone view
      </Link>
    </section>
  )
}

/**
 * How a phone joins: on this network, straight to this machine (ADR 0112), or through the server, from anywhere
 * (ADR 0028). The hosted app has only the server's way, and a phone pairs no other phone.
 */
function Pairing({ hasServer }: { hasServer: boolean }) {
  const [through, setThrough] = useState<'lan' | 'server'>('lan')
  if (onLan()) return null
  if (hosted()) return <PairPhone />
  if (!hasServer) return <LanCard />
  return (
    <div className="flex flex-col gap-3">
      <div role="group" aria-label="pair through" className="flex gap-1.5">
        <Chip selected={through === 'lan'} onClick={() => setThrough('lan')}>
          On this network
        </Chip>
        <Chip selected={through === 'server'} onClick={() => setThrough('server')}>
          Through your server
        </Chip>
      </div>
      {through === 'lan' ? <LanCard /> : <PairPhone />}
    </div>
  )
}

/** The person's devices: which are online, what runs on each, what each may do (ADR 0026, the Devices board). */
export function Devices({ children }: { children?: ReactNode }) {
  const hasServer = useHasServer()
  const devices = useQuery({ queryKey: keys.devices, queryFn: api.devices, enabled: hasServer })
  const lan = useLan()
  const lanPhones = lan.data?.phones ?? []
  // With no server, this machine is the one device besides its phones.
  const machine: DeviceView | null =
    !hasServer && lan.data
      ? {
          id: 'this',
          kind: 'device',
          name: lan.data.machine,
          created_ms: 0,
          last_used_ms: 0,
          online: true,
          this: true,
          capabilities: {
            answer_permissions: true,
            send_prompts: true,
            run_agents: true,
            review: true,
            sample: true,
            page_tools: true,
          },
          agents: [],
          startable: [],
        }
      : null
  const listed = [...(hasServer ? (devices.data ?? []) : machine ? [machine] : [])].sort(
    (a, b) => Number(b.this) - Number(a.this),
  )
  // In Settings the column is Settings' own, so the page keeps its edge and stacks.
  const embedded = useContext(InSettings)
  return (
    <Page flush label="devices">
      <div className={cx('flex flex-col gap-6', !embedded && 'px-4 py-6 md:px-8 lg:flex-row lg:items-start')}>
        <div className="flex min-w-0 flex-1 flex-col gap-3">
          <h2 id="your-devices" className={SETTINGS_HEADING}>
            Your devices
          </h2>
          <p className="text-sm text-fg-2">
            Threads move between these. A device with a checkout can run agents; a phone can answer and review.
          </p>
          {(hasServer ? devices.isPending : lan.isPending && !onLan()) && <Skeleton className="h-24" />}
          {hasServer && devices.error && <p className="text-danger">{message(devices.error)}</p>}
          {hasServer && devices.data?.length === 0 && lanPhones.length === 0 && (
            <EmptyState title="Sign a machine in with `silt auth login`">It shows here, with what runs on it.</EmptyState>
          )}
          {listed.length + lanPhones.length > 0 && (
            <div className="overflow-hidden rounded-md border border-line bg-elevated">
              {listed.map((device) => (
                <Device key={device.id} device={device} />
              ))}
              {lanPhones.map((phone) => (
                <Device key={`lan-${phone.id}`} device={asDevice(phone)} lanPhone={phone} />
              ))}
            </div>
          )}
          {children}
        </div>
        <div className={cx(!embedded && 'lg:w-[380px] lg:shrink-0')}>
          <Pairing hasServer={hasServer} />
        </div>
      </div>
    </Page>
  )
}
