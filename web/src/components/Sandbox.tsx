import { Link } from '@tanstack/react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import {
  api,
  type AgentStatus,
  type ApprovalView,
  type BackendView,
  type Extra,
  type GrantScope,
  type Installed,
  type FolderAnswer,
  type FolderRule,
  type Policy,
  type Reach,
  type ThreadSummary,
} from '../api'
import { keys, message } from '../query'
import { workspaceOf } from '../rules'
import {
  BACKENDS,
  DEVICE_ORDER,
  DEVICES,
  HANDS_OVER,
  NETWORK_ORDER,
  NETWORKS,
  PLANNED_BACKENDS,
  choiceOf,
  chosenPolicy,
  reachOf,
  useSandboxChoice,
} from '../sandbox'
import { useNow } from '../now'
import { hosted } from '../served'
import { useShortcut } from '../shortcuts'
import { Button, Icon, Input, Kbd, Select, Tag, cx } from '../ui'
import { PluginCard } from './PluginCard'
import { TONE_DOT, callSaid, clock } from '../calls'

function networkLine(reach: Reach): string {
  switch (reach.network) {
    case 'none':
      return 'None'
    case 'gateway':
      return reach.hosts.length ? `Through Silt only, to ${reach.hosts.join(', ')}` : 'Through Silt only'
    case 'full':
      return 'Full internet, not this machine or its networks'
    case 'host':
      return "This machine's network"
  }
}

function devicesLine(reach: Reach): string {
  if (reach.devices === 'all_usb') return 'All USB'
  if (reach.devices === 'named' && reach.device_nodes.length) return reach.device_nodes.join(', ')
  if (reach.devices === 'relayed' && reach.device_nodes.length) return `${reach.device_nodes.join(', ')} (relayed)`
  return 'None'
}

function alsoLine(reach: Reach): string | null {
  const parts = [...reach.ranges]
  if (reach.ports.length) parts.push(`ports ${reach.ports.join(', ')}`)
  for (const [path, writable] of Object.entries(reach.paths)) parts.push(`${path}${writable ? ' (writable)' : ''}`)
  if (reach.gpu) parts.push('the GPU')
  if (reach.docker) parts.push('Docker')
  if (reach.ssh_agent) parts.push('ssh-agent')
  return parts.length ? parts.join(' · ') : null
}

/** What approving may also allow: nothing more, or calls like it for the run or the thread a while. */
const GRANTS: { value: string; label: (change: boolean) => string; scope: GrantScope | null }[] = [
  { value: 'once', label: () => 'ask again next time', scope: null },
  {
    value: 'always',
    label: (change) => `always allow ${change ? 'changes' : 'reads'} like it, by a rule of yours`,
    scope: { for: 'always' },
  },
  { value: 'run', label: (change) => `allow ${change ? 'changes' : 'reads'} like it for this run`, scope: { for: 'run' } },
  {
    value: 'hour',
    label: (change) => `allow ${change ? 'changes' : 'reads'} like it in this thread for an hour`,
    scope: { for: 'thread', minutes: 60 },
  },
  {
    value: 'day',
    label: (change) => `allow ${change ? 'changes' : 'reads'} like it in this thread for 8 hours`,
    scope: { for: 'thread', minutes: 480 },
  },
]

function remaining(createdMs: number, now: number): string {
  const left = Math.max(0, createdMs + 15 * 60 * 1000 - now)
  const minutes = Math.floor(left / 60000)
  const seconds = Math.floor((left % 60000) / 1000)
  return `${minutes}:${String(seconds).padStart(2, '0')}`
}

/** A plugin's call waiting for the person (ADR 0093), drawn from Silt's own copy of the request, never the agent's account of it. */
export function ApprovalCard({ approval, first, title }: { approval: ApprovalView; first: boolean; title?: string }) {
  const queries = useQueryClient()
  const [error, setError] = useState<string>()
  const [sending, setSending] = useState(false)
  const [grant, setGrant] = useState<string>('once')
  const answer = (approve: boolean) => {
    setError(undefined)
    setSending(true)
    api
      .answerApproval(
        approval.id,
        approve,
        approval.hash,
        approve ? (GRANTS.find((choice) => choice.value === grant)?.scope ?? null) : null,
      )
      .then(() => void queries.invalidateQueries({ queryKey: ['approvals'] }))
      .catch((failure: unknown) => setError(message(failure)))
      .finally(() => setSending(false))
  }
  useShortcut('enter', 'Approve the plugin call shown', () => {
    if (first && !sending && !document.querySelector('dialog[open]')) answer(true)
  })
  const now = useNow(1000)
  const change = approval.change
  return (
    <div
      role="group"
      aria-label="approval"
      className="flex flex-col gap-2.5 rounded-md border border-warn bg-elevated px-3.5 py-3"
    >
      <span className="flex min-w-0 flex-wrap items-center gap-2 text-xs text-fg-2">
        <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn" />
        The agent asks to {change ? 'change' : 'read'} {approval.environment} through{' '}
        <span className="font-medium text-fg-1">{approval.plugin}</span>
        <Tag tone="warn">{approval.environment}</Tag>
        <Tag>{approval.tool}</Tag>
        {title && <span className="truncate">· {title}</span>}
      </span>
      {approval.card ? (
        <PluginCard card={approval.card} effects={approval.effects} why={approval.why} />
      ) : (
        <>
          <span className="font-mono text-2xs tracking-[0.04em] text-fg-2 uppercase">Silt will send exactly this, once</span>
          <pre className="max-h-80 overflow-y-auto rounded-sm border border-line bg-bg px-2.5 py-2 font-mono text-xs whitespace-pre-wrap text-fg-1 [overflow-wrap:anywhere]">
            {approval.shown}
          </pre>
        </>
      )}
      <span className="font-mono text-2xs text-fg-2">
        {approval.target} · request {approval.hash.slice(0, 4)}…{approval.hash.slice(-4)} · expires in{' '}
        {remaining(approval.created_ms, now)}
      </span>
      <span className="flex flex-wrap items-center gap-1.5">
        <Button variant="primary" disabled={sending} onClick={() => answer(true)}>
          {approval.card ? 'Approve once' : 'Approve and send once'} {first && <Kbd onAccent>⏎</Kbd>}
        </Button>
        <Button disabled={sending} onClick={() => answer(false)}>
          Reject
        </Button>
        {approval.once ? (
          <span className="ml-auto text-xs text-fg-2">{approval.environment}: once only</span>
        ) : (
          <label className="ml-auto flex items-center gap-1.5 text-xs text-fg-2">
            Then
            <select
              aria-label="what else it allows"
              className="h-7 rounded-sm border border-line bg-bg px-1.5 text-xs text-fg-1"
              value={grant}
              onChange={(event) => setGrant(event.target.value)}
            >
              {GRANTS.filter((choice) => choice.value !== 'always' || approval.card).map((choice) => (
                <option key={choice.value} value={choice.value}>
                  {choice.label(change)}
                </option>
              ))}
            </select>
          </label>
        )}
      </span>
      <span className="text-2xs text-fg-2">
        Shown from Silt's own copy of the request, not the agent's description of it. The agent can't change it now; what you
        approve is what is sent.
      </span>
      {error && <span className="text-xs text-danger">{error}</span>}
    </div>
  )
}

/** The thread's plugin calls waiting for the person. */
export function Approvals({ thread }: { thread: string }) {
  const approvals = useQuery({
    queryKey: keys.approvals(thread),
    queryFn: () => api.approvals(thread),
    enabled: !hosted(),
    refetchInterval: 30_000,
  })
  const waiting = (approvals.data ?? []).filter((approval) => approval.state === 'pending')
  return (
    <>
      {waiting.map((approval, index) => (
        <ApprovalCard key={approval.id} approval={approval} first={index === 0} />
      ))}
    </>
  )
}

const ROW = 'grid grid-cols-1 gap-2 border-b border-line-subtle px-4 py-3 last:border-b-0 md:grid-cols-[200px_minmax(0,1fr)]'
const SUB = 'text-xs text-fg-2'

function Choice<T extends string>({
  name,
  options,
  value,
  most,
  orgName,
  onChange,
}: {
  name: string
  options: { value: T; label: string; detail: string }[]
  value: T
  most: number
  orgName: string | null
  onChange: (value: T) => void
}) {
  return (
    <div className="flex flex-col gap-2" role="radiogroup" aria-label={name}>
      {options.map((option, index) => {
        const capped = index > most
        return (
          <label
            key={option.value}
            className={cx('flex cursor-pointer items-start gap-2 text-sm', capped && 'cursor-not-allowed opacity-50')}
          >
            <input
              type="radio"
              name={name}
              className="mt-1 accent-[var(--accent)]"
              checked={value === option.value}
              disabled={capped}
              onChange={() => onChange(option.value)}
            />
            <span className="flex flex-col">
              <span>{option.label}</span>
              <span className={SUB}>{option.detail}</span>
              {capped && orgName && <span className="text-2xs text-danger">{orgName} allows less</span>}
            </span>
          </label>
        )
      })}
    </div>
  )
}

function ListInput({
  label,
  value,
  placeholder,
  onChange,
}: {
  label: string
  value: string[]
  placeholder: string
  onChange: (value: string[]) => void
}) {
  const [text, setText] = useState(value.join(', '))
  return (
    <input
      aria-label={label}
      className="h-7 w-full rounded-sm border border-line bg-bg px-2 font-mono text-xs"
      placeholder={placeholder}
      value={text}
      onChange={(event) => setText(event.target.value)}
      onBlur={() =>
        onChange(
          text
            .split(/[\s,]+/)
            .map((part) => part.trim())
            .filter(Boolean),
        )
      }
    />
  )
}

/** Settings › Workspaces' "What its agents can reach" (ADR 0093): the sandbox's network, devices and what of the machine it's handed. */
export function ReachSettings({
  workspace,
  policy,
  org,
  backends,
  onChange,
}: {
  workspace: string
  policy: Policy
  org: { name: string; policy: Policy } | null
  /** What can make Silt's sandbox, and why one can't on this machine. */
  backends: BackendView[]
  onChange: (next: Partial<Policy>) => void
}) {
  const orgRequires = Boolean(org?.policy.silt_sandbox)
  const sandboxed = Boolean(policy.silt_sandbox) || orgRequires
  const orgBackend = org?.policy.backend ?? null
  // Unset, it's the one this machine has, as the daemon picks it.
  const here = backends.find((known) => known.unavailable === null)?.backend ?? 'bubblewrap'
  const backend = orgBackend ?? policy.backend ?? here
  const unavailable = backends.find((known) => known.backend === backend)?.unavailable ?? null
  const refused = new Set(org?.policy.refuse ?? [])
  const mostNetwork = org?.policy.network ? NETWORK_ORDER.indexOf(org.policy.network) : NETWORK_ORDER.length - 1
  const mostDevices = org?.policy.devices ? DEVICE_ORDER.indexOf(org.policy.devices) : DEVICE_ORDER.length - 1
  const extra = (key: Extra, label: string, detail: string, on: boolean, change: (on: boolean) => void, more?: ReactNode) => (
    <div className={cx('flex flex-col gap-1', refused.has(key) && 'opacity-50')}>
      <label className="flex items-start gap-2 text-sm">
        <input
          type="checkbox"
          aria-label={label}
          className="mt-1 accent-[var(--accent)]"
          checked={on && !refused.has(key)}
          disabled={refused.has(key)}
          onChange={(event) => change(event.target.checked)}
        />
        <span className="flex flex-col">
          <span>{label}</span>
          <span className={SUB}>{detail}</span>
          {(key === 'docker' || key === 'ssh_agent') && (
            <span className="inline-flex items-center gap-1 text-2xs text-danger">
              <Icon name="alert" className="h-3 w-3" /> {HANDS_OVER[key]}
              {refused.has(key) && org ? ` · ${org.name} never allows it` : ''}
            </span>
          )}
        </span>
      </label>
      {on && !refused.has(key) && more}
    </div>
  )
  return (
    <section aria-label="What its agents can reach" className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">What its agents can reach</h3>
      <div className="overflow-hidden rounded-md border border-line bg-elevated">
        <div className={ROW}>
          <span className="flex flex-col">
            <span className="text-sm">Silt's sandbox</span>
            <span className={SUB}>Around the whole agent</span>
          </span>
          <div className="flex flex-col gap-1.5">
            <label className={cx('flex items-start gap-2 text-sm', orgRequires && 'opacity-50')}>
              <input
                type="checkbox"
                aria-label="Run its agents in Silt's sandbox"
                className="mt-1 accent-[var(--accent)]"
                checked={sandboxed}
                disabled={orgRequires}
                onChange={(event) => onChange({ silt_sandbox: event.target.checked })}
              />
              <span className="flex flex-col">
                <span>Run its agents in Silt's sandbox</span>
                <span className={SUB}>
                  Every agent Silt starts in {workspace}, from a terminal or here: the thread's checkout, your toolchains
                  read-only, no credentials and none of your sockets.
                  {orgRequires && org ? ` ${org.name} requires it.` : ''}
                </span>
              </span>
            </label>
            {sandboxed && unavailable && (
              <p role="alert" className="flex items-start gap-1 text-xs text-danger">
                <Icon name="alert" className="mt-0.5 h-3 w-3 shrink-0" />
                <span>
                  This machine can't make it with {backend}, so agents won't start in {workspace} here: {unavailable}.{' '}
                  <code className="font-mono">silt access doctor</code> says more.
                </span>
              </p>
            )}
            {!sandboxed && (
              <p className={SUB}>
                Off: its agents run as you, held only by their permissions and their own sandbox. What's below applies once it's
                on.
              </p>
            )}
          </div>
        </div>
        <fieldset
          disabled={!sandboxed}
          aria-label="What the sandbox reaches"
          className={cx('min-w-0', !sandboxed && 'opacity-50')}
        >
          <div className={ROW}>
            <span className="flex flex-col">
              <span className="text-sm">Isolation</span>
              <span className={SUB}>What makes the sandbox</span>
            </span>
            <div className="flex flex-col gap-2" role="radiogroup" aria-label="Isolation">
              {BACKENDS.map((option) => {
                const locked = orgBackend !== null && orgBackend !== option.value
                const why = backends.find((known) => known.backend === option.value)?.unavailable
                return (
                  <label
                    key={option.value}
                    className={cx('flex cursor-pointer items-start gap-2 text-sm', locked && 'cursor-not-allowed opacity-50')}
                  >
                    <input
                      type="radio"
                      name="Isolation"
                      className="mt-1 accent-[var(--accent)]"
                      checked={backend === option.value}
                      disabled={locked}
                      onChange={() => onChange({ backend: option.value })}
                    />
                    <span className="flex flex-col">
                      <span>{option.label}</span>
                      <span className={SUB}>{option.detail}</span>
                      {why && <span className="text-2xs text-danger">Not on this machine: {why}</span>}
                      {orgBackend === option.value && org && <span className="text-2xs text-fg-2">{org.name} chooses it</span>}
                    </span>
                  </label>
                )
              })}
              {PLANNED_BACKENDS.map((planned) => (
                <label key={planned.label} className="flex cursor-not-allowed items-start gap-2 text-sm opacity-50">
                  <input type="radio" name="Isolation" className="mt-1" checked={false} disabled readOnly />
                  <span className="flex flex-col">
                    <span className="inline-flex items-center gap-1.5">
                      {planned.label} <Tag>Not yet</Tag>
                    </span>
                    <span className={SUB}>{planned.detail}</span>
                  </span>
                </label>
              ))}
            </div>
          </div>
          <div className={ROW}>
            <span className="flex flex-col">
              <span className="text-sm">Network</span>
              <span className={SUB}>What it can connect to</span>
            </span>
            <div className="flex flex-col gap-3">
              <Choice
                name="Network"
                options={NETWORKS}
                value={policy.network ?? 'full'}
                most={mostNetwork}
                orgName={org?.name ?? null}
                onChange={(network) => onChange({ network })}
              />
              {(policy.network ?? 'full') === 'gateway' && (
                <ListInput
                  label="Hosts it reaches"
                  placeholder="api.github.com, *.npmjs.org; any public host when empty"
                  value={policy.hosts ?? []}
                  onChange={(hosts) => onChange({ hosts: hosts.length ? hosts : null })}
                />
              )}
              <div className="rounded-sm border border-line-subtle bg-bg px-3 py-2 text-xs">
                <span className="font-mono text-2xs tracking-[0.04em] text-fg-2 uppercase">Always blocked</span>
                <p className="mt-1 flex flex-wrap gap-1">
                  {[
                    'this machine',
                    '127.0.0.0/8',
                    '10.0.0.0/8',
                    '172.16.0.0/12',
                    '192.168.0.0/16',
                    '169.254.0.0/16 · cloud metadata',
                    '100.64.0.0/10',
                    'fc00::/7',
                    'fe80::/10',
                  ].map((range) => (
                    <Tag key={range}>{range}</Tag>
                  ))}
                </p>
                <p className={cx(SUB, 'mt-1')}>
                  Hosts on a VPN are in these ranges: reach them through a plugin, or allow a range.
                </p>
              </div>
            </div>
          </div>
          <div className={ROW}>
            <span className="flex flex-col">
              <span className="text-sm">Devices</span>
              <span className={SUB}>USB serial, adb</span>
            </span>
            <div className="flex flex-col gap-2">
              <Choice
                name="Devices"
                options={DEVICES}
                value={policy.devices ?? 'none'}
                most={mostDevices}
                orgName={org?.name ?? null}
                onChange={(devices) => onChange({ devices })}
              />
              {policy.devices === 'named' && (
                <ListInput
                  label="Device nodes"
                  placeholder="/dev/ttyUSB0, /dev/bus/usb/001/004"
                  value={policy.device_nodes ?? []}
                  onChange={(device_nodes) => onChange({ device_nodes })}
                />
              )}
              {policy.devices === 'relayed' && (
                <ListInput
                  label="Devices to relay"
                  placeholder="/dev/ttyUSB0@115200, adb:R58M30ABCD"
                  value={policy.device_nodes ?? []}
                  onChange={(device_nodes) => onChange({ device_nodes })}
                />
              )}
            </div>
          </div>
          <div className={ROW}>
            <span className="flex flex-col">
              <span className="text-sm">From this machine</span>
              <span className={SUB}>Each one hands over something; off unless you turn it on</span>
            </span>
            <div className="flex flex-col gap-2.5">
              {extra(
                'ranges',
                'Address ranges',
                'Past those always blocked, such as a VPN’s',
                Boolean(policy.ranges?.length),
                (on) => onChange({ ranges: on ? ['10.0.0.0/8'] : [] }),
                <ListInput
                  label="Address ranges"
                  placeholder="10.20.0.0/16"
                  value={policy.ranges ?? []}
                  onChange={(ranges) => onChange({ ranges })}
                />,
              )}
              {extra(
                'ports',
                'Ports on this machine',
                'Named ports only, as the sandbox’s own localhost, such as a dev database: 5432',
                Boolean(policy.ports?.length),
                (on) => onChange({ ports: on ? [5432] : [] }),
                <ListInput
                  label="Ports"
                  placeholder="5432, 6379"
                  value={(policy.ports ?? []).map(String)}
                  onChange={(ports) =>
                    onChange({ ports: ports.map(Number).filter((port) => Number.isInteger(port) && port > 0 && port < 65536) })
                  }
                />,
              )}
              {extra(
                'paths',
                'More folders',
                'Read-only, or writable with :rw after it, beyond the thread’s checkout and your toolchains',
                Boolean(Object.keys(policy.paths ?? {}).length),
                (on) => onChange({ paths: on ? { '~/datasets': false } : {} }),
                <ListInput
                  label="Folders"
                  placeholder="~/datasets, ~/scratch:rw"
                  value={Object.entries(policy.paths ?? {}).map(([path, writable]) => (writable ? `${path}:rw` : path))}
                  onChange={(paths) =>
                    onChange({
                      paths: Object.fromEntries(
                        paths.map((path) => (path.endsWith(':rw') ? [path.slice(0, -3), true] : [path, false])),
                      ),
                    })
                  }
                />,
              )}
              {extra('gpu', 'The GPU', '/dev/dri, /dev/nvidia*', Boolean(policy.gpu), (gpu) => onChange({ gpu }))}
              {extra('docker', 'Docker', 'This machine’s Docker socket', Boolean(policy.docker), (docker) =>
                onChange({ docker }),
              )}
              {extra('ssh_agent', 'ssh-agent', 'Your SSH agent’s socket', Boolean(policy.ssh_agent), (ssh_agent) =>
                onChange({ ssh_agent }),
              )}
            </div>
          </div>
          <div className={ROW}>
            <span className="flex flex-col">
              <span className="text-sm">Folders agents ask for</span>
              <span className={SUB}>With Silt's Folders plugin, beyond what's above</span>
            </span>
            <FolderRules
              rules={policy.folders ?? []}
              org={org ? { name: org.name, rules: org.policy.folders ?? [] } : null}
              onChange={(folders) => onChange({ folders })}
            />
          </div>
          <div className={ROW}>
            <span className="text-sm">Agents' own sandboxes</span>
            <label className={cx('flex items-start gap-2 text-sm', org?.policy.no_nested && 'opacity-50')}>
              <input
                type="checkbox"
                aria-label="agents' own sandboxes inside"
                className="mt-1 accent-[var(--accent)]"
                checked={!policy.no_nested && !org?.policy.no_nested}
                disabled={Boolean(org?.policy.no_nested)}
                onChange={(event) => onChange({ no_nested: !event.target.checked })}
              />
              <span className="flex flex-col">
                <span>Allowed inside</span>
                <span className={SUB}>Codex's and Claude Code's own sandbox run inside Silt's, as a second wall</span>
              </span>
            </label>
          </div>
        </fieldset>
      </div>
    </section>
  )
}

const FOLDER_ANSWERS: { value: FolderAnswer; label: string }[] = [
  { value: 'allow', label: 'given' },
  { value: 'ask', label: 'asked' },
  { value: 'never', label: 'never' },
]

/** The person's rules for folders an agent asks for, under their org's, which only tighten (ADR 0102). */
function FolderRules({
  rules,
  org,
  onChange,
}: {
  rules: FolderRule[]
  org: { name: string; rules: FolderRule[] } | null
  onChange: (rules: FolderRule[]) => void
}) {
  const [root, setRoot] = useState('')
  const put = (index: number, next: Partial<FolderRule>) =>
    onChange(rules.map((rule, at) => (at === index ? { ...rule, ...next } : rule)))
  const add = () => {
    const trimmed = root.trim()
    if (!trimmed || rules.some((rule) => rule.root === trimmed)) return
    onChange([...rules, { root: trimmed, answer: 'ask', writable: false }])
    setRoot('')
  }
  return (
    <div className="flex min-w-0 flex-col gap-1.5" aria-label="folder rules">
      {org?.rules.map((rule) => (
        <div key={`org-${rule.root}`} className="flex min-w-0 items-center gap-2 text-sm opacity-70">
          <code className="min-w-0 flex-1 truncate font-mono text-xs">{rule.root}</code>
          <Tag tone={rule.answer === 'never' ? 'danger' : 'neutral'}>
            {FOLDER_ANSWERS.find((known) => known.value === rule.answer)?.label}
            {rule.writable ? ', writable' : ''}
          </Tag>
          <span className={SUB}>{org.name}</span>
        </div>
      ))}
      {rules.map((rule, index) => (
        <div key={rule.root} className="flex min-w-0 flex-wrap items-center gap-2 text-sm">
          <code className="min-w-0 flex-1 truncate font-mono text-xs">{rule.root}</code>
          <Select
            aria-label={`answer for ${rule.root}`}
            value={rule.answer}
            onChange={(event) => put(index, { answer: event.target.value as FolderAnswer })}
          >
            {FOLDER_ANSWERS.map((answer) => (
              <option key={answer.value} value={answer.value}>
                {answer.label}
              </option>
            ))}
          </Select>
          <label className={cx('flex items-center gap-1 text-xs', rule.answer === 'never' && 'opacity-50')}>
            <input
              type="checkbox"
              aria-label={`writable ${rule.root}`}
              className="accent-[var(--accent)]"
              checked={rule.writable}
              disabled={rule.answer === 'never'}
              onChange={(event) => put(index, { writable: event.target.checked })}
            />
            writable
          </label>
          <Button
            variant="ghost"
            aria-label={`remove ${rule.root}`}
            onClick={() => onChange(rules.filter((_, at) => at !== index))}
          >
            Remove
          </Button>
        </div>
      ))}
      <form
        className="flex min-w-0 items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault()
          add()
        }}
      >
        <Input
          aria-label="folder to add a rule for"
          placeholder="~/datasets"
          className="min-w-0 flex-1 font-mono"
          value={root}
          onChange={(event) => setRoot(event.target.value)}
        />
        <Button type="submit" disabled={!root.trim()}>
          Add
        </Button>
      </form>
      <span className={SUB}>
        Folders no rule names are asked. Silt's data, credentials and other workspaces' folders are never given.
      </span>
    </div>
  )
}

/** Whether `plugin` is on in `workspace`; `*` is every workspace. */
function onIn(plugin: Installed, workspace: string): boolean {
  return plugin.workspaces.includes(workspace) || plugin.workspaces.includes('*')
}

/** Where a plugin came from, as its tag says it. */
function sourceLabel(source: Installed['source']): string {
  switch (source.type) {
    case 'shipped':
      return 'Silt'
    case 'git':
      return 'git'
    case 'dev':
      return 'dev'
    case 'marketplace':
      return 'marketplace'
    case 'org':
      return source.org
  }
}

/** Settings › Workspaces' plugins (ADR 0093): which of this machine's plugins the workspace's agents get. Saved at once. */
export function WorkspacePlugins({ workspace, name }: { workspace: string; name: string }) {
  const queries = useQueryClient()
  const plugins = useQuery({ queryKey: keys.plugins, queryFn: api.plugins })
  const [error, setError] = useState<string>()
  const turn = (plugin: Installed, on: boolean) => {
    setError(undefined)
    api.turnPlugin(plugin.id, workspace, on).then(
      () => void queries.invalidateQueries({ queryKey: keys.plugins }),
      (failure: unknown) => setError(message(failure)),
    )
  }
  return (
    <section aria-label="Plugins its agents can use" className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">Plugins its agents can use</h3>
      <p className={SUB}>
        A sandboxed agent started in {name} gets these plugins' tools and no others, run by Silt outside the sandbox with
        credentials it never sees. Other workspaces' agents never see them.
      </p>
      <div className="overflow-hidden rounded-md border border-line bg-elevated">
        {(plugins.data ?? []).map(({ installed: plugin, manifest }) => {
          const on = onIn(plugin, workspace)
          const everywhere = plugin.workspaces.includes('*')
          const elsewhere = plugin.workspaces.filter((other) => other !== workspace && other !== '*')
          return (
            <label
              key={plugin.id}
              className="flex cursor-pointer items-center gap-2.5 border-b border-line-subtle px-4 py-2.5 text-sm last:border-b-0"
            >
              <input
                type="checkbox"
                aria-label={`${plugin.id} in ${name}`}
                className="accent-[var(--accent)]"
                checked={on}
                onChange={(event) => turn(plugin, event.target.checked)}
              />
              <Link to="/settings/plugins/plugin" search={{ id: plugin.id }} className="font-medium hover:text-accent-fg">
                {manifest?.name ?? plugin.id}
              </Link>
              <Tag>{sourceLabel(plugin.source)}</Tag>
              <span className="font-mono text-2xs text-fg-2">{plugin.version}</span>
              <span className="ml-auto text-xs text-fg-2">
                {everywhere ? 'in every workspace' : elsewhere.length ? `also in ${elsewhere.join(', ')}` : ''}
              </span>
            </label>
          )
        })}
        {plugins.data?.length === 0 && (
          <p className="px-4 py-3 text-xs text-fg-2">
            No plugins on this machine yet. Add one from git with{' '}
            <code className="font-mono">silt plugin add &lt;url&gt;#&lt;tag&gt;</code>.
          </p>
        )}
      </div>
      {error && <p className="text-xs text-danger">{error}</p>}
    </section>
  )
}

function Heading({ children }: { children: ReactNode }) {
  return <h3 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">{children}</h3>
}

/**
 * The thread's Access tab (ADR 0093): what its sandbox reaches, the grants in force, each plugin call and how it
 * ended, and what the gateway refused.
 */
export function AccessPanel({
  thread,
  summary,
  agent,
  embedded = false,
}: {
  thread: string
  summary: ThreadSummary | undefined
  agent: AgentStatus | null | undefined
  /** Its sections only, in a list of others such as Details. */
  embedded?: boolean
}) {
  const workspaces = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces, enabled: !hosted() })
  const queries = useQueryClient()
  const access = useQuery({ queryKey: keys.access(thread), queryFn: () => api.access(thread), enabled: !hosted() })
  const [error, setError] = useState<string>()
  const revoke = (id: number) => {
    setError(undefined)
    api.revokeGrant(id).then(
      () => void queries.invalidateQueries({ queryKey: keys.allApprovals }),
      (failure: unknown) => setError(message(failure)),
    )
  }
  const release = (device: string) => {
    setError(undefined)
    api.releaseDevice(thread, device).then(
      () => void queries.invalidateQueries({ queryKey: keys.allApprovals }),
      (failure: unknown) => setError(message(failure)),
    )
  }
  const unfolder = (path: string) => {
    setError(undefined)
    api.unfolder(thread, path).then(
      () => void queries.invalidateQueries({ queryKey: keys.access(thread) }),
      (failure: unknown) => setError(message(failure)),
    )
  }
  const live = agent && agent.state !== 'stopped' ? agent : null
  const view = summary ? workspaceOf(workspaces.data, summary.repo, summary.workspace) : undefined
  // With nothing running, what the thread's choice or its workspace's rules give the next run, when they sandbox it.
  const chosen = useSandboxChoice(thread).choice
  const upcoming = view ? chosenPolicy(view, chosen ?? choiceOf(view.workspace.policy)) : null
  const next = !live && upcoming?.silt_sandbox ? reachOf(upcoming) : null
  const reach = live?.sandbox ?? next
  const grants = (access.data?.grants ?? []).filter((grant) => grant.active)
  return (
    <div className={cx('flex flex-col gap-6 text-sm', !embedded && 'px-5 py-5')}>
      <section aria-label="this run" className="flex flex-col gap-2">
        <Heading>{next ? 'The next run' : 'This run'}</Heading>
        {reach ? (
          <div className="grid grid-cols-[110px_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-xs">
            <span className="text-fg-2">Sandbox</span>
            <span className="font-mono">{reach.backend}</span>
            <span className="text-fg-2">Network</span>
            <span className="font-mono">{networkLine(reach)}</span>
            {alsoLine(reach) && (
              <>
                <span className="text-fg-2">Also allowed</span>
                <span className="font-mono">{alsoLine(reach)}</span>
              </>
            )}
            <span className="text-fg-2">Devices</span>
            <span className="font-mono">{devicesLine(reach)}</span>
          </div>
        ) : (
          <p className="text-xs text-fg-2">
            {live
              ? "Not in Silt's sandbox."
              : chosen
                ? "No agent runs here now. This thread runs its agents outside Silt's sandbox, so they run as you."
                : `No agent runs here now. ${view?.workspace.name ?? 'Its workspace'} doesn't run its agents in Silt's sandbox, so they run as you.`}
          </p>
        )}
      </section>
      <section aria-label="granted" className="flex flex-col gap-2">
        <Heading>Granted</Heading>
        {grants.length === 0 && <p className="text-xs text-fg-2">Nothing beyond each call you approve.</p>}
        {grants.map((grant) => (
          <div key={grant.id} className="flex items-start gap-2">
            <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-ok" />
            <span className="flex min-w-0 flex-1 flex-col">
              <span>
                {grant.plugin} · {grant.environment} · {grant.change ? 'changes' : 'reads'}
              </span>
              <span className="font-mono text-2xs text-fg-2">
                {grant.run_only ? 'this run' : 'this thread'}
                {grant.until_ms !== null ? ` · until ${clock(grant.until_ms)}` : ''}
              </span>
            </span>
            <Button variant="ghost" onClick={() => revoke(grant.id)} aria-label={`revoke ${grant.plugin} ${grant.environment}`}>
              Revoke
            </Button>
          </div>
        ))}
        <p className="text-2xs text-fg-2">Grants stay with this thread. A fork starts with none.</p>
      </section>
      {(access.data?.folders.length ?? 0) > 0 && (
        <section aria-label="folders" className="flex flex-col gap-2">
          <Heading>Folders</Heading>
          {access.data?.folders.map((folder) => (
            <div key={folder.path} className="flex items-start gap-2">
              <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-ok" />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate font-mono text-xs">{folder.path}</span>
                <span className="font-mono text-2xs text-fg-2">
                  {folder.writable ? 'writable' : 'read-only'} · {folder.run_only ? 'this run' : 'this thread'} · since{' '}
                  {clock(folder.granted_ms)}
                  {folder.until_ms !== null ? ` · until ${clock(folder.until_ms)}` : ''}
                </span>
              </span>
              <Button variant="ghost" onClick={() => unfolder(folder.path)} aria-label={`unfolder ${folder.path}`}>
                Unfolder
              </Button>
            </div>
          ))}
          <p className="text-2xs text-fg-2">
            Taken back, a folder leaves the agent's sandbox when its turn ends; it restarts where it was without it.
          </p>
        </section>
      )}
      {(access.data?.devices.length ?? 0) > 0 && (
        <section aria-label="devices and sessions" className="flex flex-col gap-2">
          <Heading>Devices and sessions</Heading>
          {access.data?.devices.map((device) => (
            <div key={device.device} className="flex items-start gap-2">
              <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-ok" />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate font-mono text-xs">{device.device}</span>
                <span className="font-mono text-2xs text-fg-2">
                  {device.kind === 'adb' ? 'adb, for this device only' : 'serial session'} · held since {clock(device.since_ms)}
                </span>
              </span>
              <Button variant="ghost" onClick={() => release(device.device)} aria-label={`release ${device.device}`}>
                Release
              </Button>
            </div>
          ))}
          <p className="text-2xs text-fg-2">Silt keeps each session while the agent restarts; releasing ends it.</p>
        </section>
      )}
      <section aria-label="plugin calls" className="flex flex-col gap-2">
        <Heading>Plugin calls</Heading>
        {access.data?.calls.length === 0 && <p className="text-xs text-fg-2">None yet.</p>}
        {(access.data?.calls ?? []).map((call) => {
          const { tone, said } = callSaid(call)
          return (
            <details key={call.id} className="group">
              <summary className="flex cursor-pointer list-none items-start gap-2">
                <span className={cx('mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full', TONE_DOT[tone])} />
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate">
                    {call.plugin || 'no plugin'} · {call.tool} · {call.environment}
                  </span>
                  <span className="truncate font-mono text-2xs text-fg-2">
                    {clock(call.created_ms)} · {said}
                  </span>
                </span>
              </summary>
              <pre className="mt-1.5 ml-3.5 max-h-48 overflow-y-auto rounded-sm border border-line bg-bg px-2 py-1.5 font-mono text-2xs whitespace-pre-wrap [overflow-wrap:anywhere]">
                {call.shown}
              </pre>
            </details>
          )
        })}
      </section>
      {(access.data?.refused.length ?? 0) > 0 && (
        <section aria-label="refused" className="flex flex-col gap-2">
          <Heading>Refused by the gateway</Heading>
          {access.data?.refused.map((refused, index) => (
            <div key={`${refused.time_ms}-${index}`} className="flex items-start gap-2">
              <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-danger" />
              <span className="flex min-w-0 flex-col">
                <span className="truncate font-mono text-xs">
                  {refused.host}:{refused.port}
                </span>
                <span className="font-mono text-2xs text-fg-2">
                  {clock(refused.time_ms)} · {refused.why}
                </span>
              </span>
            </div>
          ))}
        </section>
      )}
      {error && <p className="text-xs text-danger">{error}</p>}
    </div>
  )
}

/** Every plugin call waiting for the person, for Needs you. */
export function PendingApprovals({ titleOf }: { titleOf: (thread: string) => string }) {
  const approvals = useQuery({
    queryKey: keys.allApprovals,
    queryFn: () => api.approvals(),
    enabled: !hosted(),
    refetchInterval: 30_000,
  })
  const waiting = (approvals.data ?? []).filter((approval) => approval.state === 'pending')
  if (waiting.length === 0) return null
  return (
    <section aria-label="plugin calls waiting on you" className="flex flex-col gap-3">
      {waiting.map((approval, index) => (
        <div key={approval.id} className="flex flex-col gap-1">
          <ApprovalCard approval={approval} first={index === 0} title={titleOf(approval.thread)} />
          <Link to="/t/$id" params={{ id: approval.thread }} className="self-end text-xs text-accent-fg">
            Open the thread
          </Link>
        </div>
      ))}
    </section>
  )
}
