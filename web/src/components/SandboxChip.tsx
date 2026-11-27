import { Link } from '@tanstack/react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { api, type Devices, type Extra, type Network, type Reach, type SandboxChoice, type WorkspaceView } from '../api'
import { keys, message } from '../query'
import { DEVICE_ORDER, DEVICES, HANDS_OVER, NETWORK_ORDER, NETWORKS, choiceOf, chosenPolicy, sameChoice } from '../sandbox'
import { hosted } from '../served'
import { Button, Icon, type IconName, Popover, Select, cx } from '../ui'
import { GHOST_CHIP } from './ChipSelect'

/** Shorter than Settings' words, to fit the popover. */
const SAYS: Record<Network, string> = {
  none: 'Nothing leaves the sandbox',
  gateway: 'HTTP(S) to the hosts on its list',
  full: 'Any public host; never this machine or its networks',
  host: "Everything on your 127.0.0.1, Silt's web UI included",
}

const EXTRAS: { key: Extra & keyof SandboxChoice; label: string }[] = [
  { key: 'gpu', label: 'The GPU' },
  { key: 'docker', label: 'Docker socket' },
  { key: 'ssh_agent', label: 'ssh-agent' },
]

function networkLabel(network: Network): string {
  return NETWORKS.find((option) => option.value === network)?.label ?? network
}

/** What the run reaches, said in the chip's tooltip. */
function reachSaid(network: Network, devices: Devices): string {
  const device = devices === 'none' ? 'no devices' : (DEVICES.find((option) => option.value === devices)?.label ?? devices)
  return `${networkLabel(network).toLowerCase()}, ${device.toLowerCase()}`
}

/**
 * Silt's sandbox for one thread or chat, before its agent's mark in the composer (ADR 0140): on or off, and what it
 * reaches, starting from its workspace's rules, which an org's policy can lock and cap. A running agent keeps the
 * sandbox it started in, so a change offers to restart it.
 */
export function SandboxChip({
  workspace,
  choice,
  onChoice,
  running,
  scope = 'thread',
}: {
  workspace: WorkspaceView | undefined
  /** The thread's own choice, or null where it follows its workspace. */
  choice: SandboxChoice | null
  onChoice: (choice: SandboxChoice | null) => Promise<unknown> | void
  /** The agent running now and the sandbox it started in, if any. */
  running?: { name: string; reach: Reach | null; onRestart: () => void } | null
  scope?: 'thread' | 'chat'
}) {
  const queries = useQueryClient()
  const views = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces, enabled: !hosted() })
  const plugins = useQuery({ queryKey: keys.plugins, queryFn: api.plugins, enabled: !hosted() })
  const [error, setError] = useState<string>()
  if (hosted()) return null
  if (!workspace) {
    return (
      <span
        role="img"
        aria-label="sandbox"
        title="Checking whether this machine can make the sandbox"
        className={cx(GHOST_CHIP, 'px-1.5 text-fg-3')}
      >
        <Icon name="shield" className="h-4 w-4 animate-pulse" />
      </span>
    )
  }
  const name = workspace.workspace.name
  const own = choiceOf(workspace.workspace.policy)
  const current = choice ?? own
  const policy = chosenPolicy(workspace, current)
  const org = workspace.org_policy
  const cap = org?.policy
  const on = Boolean(policy.silt_sandbox)
  const locked = Boolean(cap?.silt_sandbox)
  const backend = policy.backend ?? views.data?.backends.find((known) => known.unavailable === null)?.backend ?? 'bubblewrap'
  const unavailable = views.data?.backends.find((known) => known.backend === backend)?.unavailable ?? null
  const network = policy.network ?? 'full'
  const devices = policy.devices ?? 'none'
  const refused = new Set(cap?.refuse ?? [])
  const reach = running?.reach ?? null
  const pending =
    running !== undefined &&
    running !== null &&
    (Boolean(reach) !== on ||
      (reach !== null &&
        (reach.network !== network ||
          reach.devices !== devices ||
          reach.gpu !== (Boolean(policy.gpu) && !refused.has('gpu')) ||
          reach.docker !== (Boolean(policy.docker) && !refused.has('docker')) ||
          reach.ssh_agent !== (Boolean(policy.ssh_agent) && !refused.has('ssh_agent')))))
  // A running agent's chip says what it runs in; before one starts, what it would.
  const shown = running ? Boolean(reach) : on
  const icon: IconName = !running && on && unavailable ? 'shieldAlert' : shown ? 'shieldCheck' : 'shield'
  const tone = !running && on && unavailable ? 'text-danger' : shown ? 'text-ok' : 'text-fg-2'
  const title =
    !running && on && unavailable
      ? "Silt's sandbox can't start on this machine"
      : running && reach
        ? `Sandboxed · ${reachSaid(reach.network, reach.devices)}`
        : shown
          ? `Sandboxed · ${reachSaid(network, devices)}`
          : "Not in Silt's sandbox · runs as you"
  const others = [
    ...(policy.hosts?.length && network === 'gateway' ? [`hosts ${policy.hosts.join(', ')}`] : []),
    ...(refused.has('ranges') ? [] : (policy.ranges ?? [])),
    ...(refused.has('ports') || !policy.ports?.length ? [] : [`ports ${policy.ports.join(', ')}`]),
    ...(refused.has('paths') ? [] : Object.keys(policy.paths ?? {})),
  ]
  const on_ = (plugins.data ?? [])
    .map((plugin) => plugin.installed)
    .filter((plugin) => plugin.workspaces.includes(workspace.workspace.id) || plugin.workspaces.includes('*'))
  const choose = (next: Partial<SandboxChoice>) => {
    setError(undefined)
    const chosen = { ...current, ...next }
    Promise.resolve(onChoice(sameChoice(chosen, own) ? null : chosen)).catch((failure: unknown) => setError(message(failure)))
  }
  const makeDefault = () => {
    if (!views.data) return
    setError(undefined)
    const workspaces = views.data.workspaces.map((view) =>
      view.workspace.id === workspace.workspace.id
        ? { ...view.workspace, policy: { ...view.workspace.policy, silt_sandbox: current.on, ...withoutOn(current) } }
        : view.workspace,
    )
    api
      .setWorkspaces({ default: views.data.default, workspaces })
      .then(() => queries.invalidateQueries({ queryKey: keys.workspaces }))
      .then(() => onChoice(null))
      .catch((failure: unknown) => setError(message(failure)))
  }
  const mostNetwork = cap?.network ? NETWORK_ORDER.indexOf(cap.network) : NETWORK_ORDER.length - 1
  const mostDevices = cap?.devices ? DEVICE_ORDER.indexOf(cap.devices) : DEVICE_ORDER.length - 1
  return (
    <Popover
      label="sandbox"
      triggerLabel="sandbox"
      triggerTitle={title}
      triggerValue={shown ? 'on' : 'off'}
      align="right"
      triggerClassName={cx(GHOST_CHIP, 'relative px-1.5 aria-expanded:bg-surface-2')}
      panelClassName="w-[22rem] p-1"
      trigger={
        <>
          <Icon name={icon} className={cx('h-4 w-4', tone)} />
          {pending && (
            <span
              aria-label="changed for its next start"
              className="absolute top-1 right-0.5 h-1.5 w-1.5 rounded-full bg-accent ring-2 ring-bg"
            />
          )}
        </>
      }
    >
      {(close) => (
        <div className="flex flex-col text-sm">
          <div className="flex items-center gap-1.5 px-2 pt-1.5 pb-1 text-2xs tracking-[0.06em] text-fg-3 uppercase">
            <Icon name={on ? 'shieldCheck' : 'shield'} className="h-3 w-3" />
            Sandbox
            <span className="ml-auto font-mono tracking-normal normal-case">{backend}</span>
          </div>
          {on && unavailable && (
            <p role="alert" className="m-1 flex items-start gap-2 rounded-md bg-danger-tint px-2.5 py-2 text-xs text-fg-1">
              <Icon name="alert" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-danger" />
              <span>
                This machine can't make it with {backend}: {unavailable}. <code className="font-mono">silt access doctor</code>{' '}
                says more.
              </span>
            </p>
          )}
          {pending && running && (
            <div className="m-1 flex flex-col gap-2 rounded-md bg-warn-tint px-2.5 py-2 text-xs text-fg-1">
              <span className="flex items-start gap-2">
                <Icon name="reload" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warn-fg" />
                {running.name} keeps the sandbox it started in. The change applies when it restarts; the {scope} keeps everything.
              </span>
              <span className="flex gap-1.5 pl-5.5">
                <Button
                  size="sm"
                  variant="primary"
                  onClick={() => {
                    close()
                    running.onRestart()
                  }}
                >
                  Restart {running.name} now
                </Button>
                <Button size="sm" onClick={close}>
                  At its next start
                </Button>
              </span>
            </div>
          )}
          <div className="flex items-start gap-2.5 px-2 pt-1.5 pb-2">
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="font-medium">Run in Silt's sandbox</span>
              <span className="text-xs text-fg-3">
                {locked && org ? (
                  <>
                    <Icon name="lock" className="mr-1 inline h-3 w-3 align-[-1px]" />
                    {org.org} requires it in {name}.
                  </>
                ) : (
                  `${scope === 'chat' ? "The chat's own folder" : "This thread's checkout"} and your toolchains, read-only. No credentials, none of your sockets.`
                )}
              </span>
            </span>
            <button
              type="button"
              role="switch"
              aria-checked={on}
              aria-label="Run in Silt's sandbox"
              disabled={locked}
              onClick={() => choose({ on: !current.on })}
              className={cx(
                'relative mt-0.5 h-[18px] w-[30px] shrink-0 rounded-full border transition-colors duration-100 disabled:cursor-not-allowed disabled:opacity-50 max-md:mt-0 max-md:min-h-0',
                on ? 'border-accent bg-accent' : 'border-line bg-surface-3',
              )}
            >
              <span
                className={cx(
                  'absolute top-[2px] h-3 w-3 rounded-full bg-elevated shadow-raised transition-[left] duration-100',
                  on ? 'left-[14px]' : 'left-[2px]',
                )}
              />
            </button>
          </div>
          <fieldset disabled={!on} aria-label="what the sandbox reaches" className={cx('min-w-0', !on && 'opacity-45')}>
            <Section>Network</Section>
            <div role="radiogroup" aria-label="Network" className="flex flex-col">
              {NETWORK_ORDER.map((option, index) => {
                const beyond = index > mostNetwork && cap?.network
                const checked = option === network
                return (
                  <button
                    key={option}
                    type="button"
                    role="radio"
                    aria-checked={checked}
                    aria-disabled={beyond ? true : undefined}
                    onClick={() => !beyond && choose({ network: option })}
                    className={cx(
                      'flex w-full items-center gap-2.5 rounded-sm px-2 py-1.5 text-left max-md:min-h-11',
                      beyond ? 'cursor-not-allowed text-fg-3' : 'hover:bg-surface-2 focus:bg-surface-2 focus:outline-none',
                      checked && !beyond && 'bg-surface-2',
                    )}
                  >
                    <span className="flex min-w-0 flex-1 flex-col">
                      <span className="flex items-center gap-1">
                        {networkLabel(option)}
                        {option === 'host' && !beyond && <Icon name="alert" className="h-3 w-3 text-danger" />}
                      </span>
                      <span className={cx('text-xs', beyond ? 'text-warn-fg' : 'text-fg-3')}>
                        {beyond && org ? `${org.org} allows up to ${networkLabel(cap.network!)}` : SAYS[option]}
                      </span>
                    </span>
                    {checked && !beyond ? (
                      <Icon name="check" className="h-3.5 w-3.5 shrink-0 text-accent-fg" />
                    ) : (
                      <span className="h-3.5 w-3.5 shrink-0" />
                    )}
                  </button>
                )
              })}
            </div>
            <div className="my-1 border-t border-line-subtle" />
            <Row name="Devices">
              <Select
                aria-label="Devices"
                className="w-full text-xs"
                value={devices}
                onChange={(event) => choose({ devices: event.target.value as Devices })}
              >
                {DEVICES.map((option, index) => (
                  <option key={option.value} value={option.value} disabled={index > mostDevices} title={option.detail}>
                    {option.label}
                    {index > mostDevices && org ? ` · ${org.org} doesn't allow it` : ''}
                  </option>
                ))}
              </Select>
            </Row>
            <Row name="From this machine">
              <div className="flex flex-col gap-1">
                {EXTRAS.map((extra) => {
                  const never = refused.has(extra.key)
                  return (
                    <label key={extra.key} className={cx('flex items-start gap-2 text-xs', never && 'opacity-50')}>
                      <input
                        type="checkbox"
                        aria-label={extra.label}
                        className="mt-0.5 accent-[var(--accent)]"
                        checked={Boolean(policy[extra.key]) && !never}
                        disabled={never}
                        onChange={(event) => choose({ [extra.key]: event.target.checked })}
                      />
                      <span className="flex flex-col">
                        <span className={extra.key === 'ssh_agent' ? 'font-mono' : undefined}>{extra.label}</span>
                        {(extra.key === 'docker' || extra.key === 'ssh_agent') && (
                          <span className="text-2xs text-danger">
                            {never && org ? `${org.org} never allows it` : HANDS_OVER[extra.key]}
                          </span>
                        )}
                      </span>
                    </label>
                  )
                })}
                {others.length > 0 && (
                  <span className="truncate font-mono text-2xs text-fg-3" title={others.join(', ')}>
                    and {name}'s {others.join(', ')}
                  </span>
                )}
              </div>
            </Row>
            <Row name="Plugins">
              <span className="truncate font-mono text-xs">
                {on_.length ? on_.map((plugin) => plugin.id).join(', ') : 'None'}
              </span>
            </Row>
          </fieldset>
          {error && <p className="px-2 py-1 text-xs text-danger">{error}</p>}
          <div className="mt-1 flex items-start gap-2 border-t border-line-subtle px-2 pt-2 pb-1 text-2xs text-fg-3">
            <span className="flex-1">
              {locked && org
                ? `Set by ${org.org} in ${name}; you can narrow it.`
                : `This ${scope} only. ${name} ${own.on ? 'runs its agents in it' : 'leaves it off'}.`}
              {choice && (
                <>
                  {' '}
                  <button type="button" className="text-accent-fg hover:underline" onClick={makeDefault}>
                    Make it {name}'s
                  </button>
                  {' · '}
                  <button type="button" className="text-accent-fg hover:underline" onClick={() => choose(own)}>
                    Use {name}'s
                  </button>
                </>
              )}
            </span>
            <Link
              to="/settings/$section"
              params={{ section: 'workspaces' }}
              className="text-xs whitespace-nowrap text-accent-fg hover:underline"
              onClick={close}
            >
              Workspace settings
            </Link>
          </div>
        </div>
      )}
    </Popover>
  )
}

/** A choice's reach, as a workspace's policy spells it. */
function withoutOn(choice: SandboxChoice) {
  return { network: choice.network, devices: choice.devices, gpu: choice.gpu, docker: choice.docker, ssh_agent: choice.ssh_agent }
}

function Section({ children }: { children: ReactNode }) {
  return <div className="px-2 pt-1.5 pb-1 text-2xs tracking-[0.06em] text-fg-3 uppercase">{children}</div>
}

function Row({ name, children }: { name: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[112px_minmax(0,1fr)] items-start gap-2 px-2 py-1.5 text-xs">
      <span className="pt-0.5 text-fg-2">{name}</span>
      {children}
    </div>
  )
}
