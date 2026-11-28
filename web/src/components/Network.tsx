import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { api, type NetworkSettings } from '../api'
import { keys, message } from '../query'
import { serverName } from '../served'
import { SETTINGS_HEADING } from '../settingsContext'
import { Button, Skeleton, Switch, cx, toast } from '../ui'

const GUIDE = 'https://github.com/Urcra/silt/blob/main/docs/guide/network.md'

type Row = { key: keyof Omit<NetworkSettings, 'offline'>; label: string; text: string; hosts: string }

const ROWS: Row[] = [
  {
    key: 'update_checks',
    label: 'Check for updates',
    text: "Asks whether a newer Silt, or a newer version of a plugin you added, is out: when Silt starts and every six hours. It sends Silt's version and nothing else.",
    hosts: 'api.github.com · your plugins’ repositories',
  },
  {
    key: 'plan_usage',
    label: 'Plan usage',
    text: "Shows how much of your plan is left in the message box. Silt asks each agent's own provider with the sign-in that agent keeps on this machine, or for Antigravity asks agy itself, and the answer stays here.",
    hosts: 'api.anthropic.com · chatgpt.com · api.github.com · agy',
  },
  {
    key: 'agent_registry',
    label: 'Agent registry',
    text: 'Keeps the list of agents you can add from the web UI up to date, once a day. Off, Silt lists the agents it was built with.',
    hosts: 'cdn.agentclientprotocol.com',
  },
]

const PROXY_LABEL = "Load artifacts' libraries and fonts through Silt"

function Code({ children }: { children: ReactNode }) {
  return <code className="rounded-sm border border-line-subtle bg-surface-1 px-1 font-mono text-xs text-fg-1">{children}</code>
}

function Head() {
  return (
    <>
      <div className="flex min-h-7 items-center gap-2.5">
        <h2 className={SETTINGS_HEADING}>Network</h2>
      </div>
      <p className="text-sm text-fg-2">
        Silt sends no telemetry. These are the only connections it makes on its own, and none of them carries your threads or
        code.{' '}
        <a className="text-accent-fg underline underline-offset-2" href={GUIDE} target="_blank" rel="noreferrer">
          What Silt sends over the network
        </a>
      </p>
    </>
  )
}

function Line({
  label,
  text,
  hosts,
  note,
  dim,
  control,
}: {
  label: string
  text: string
  hosts?: string
  note?: ReactNode
  dim?: boolean
  control?: ReactNode
}) {
  return (
    <li className="flex items-start gap-3 border-t border-line-subtle px-3 py-2.5">
      <div className={cx('flex min-w-0 flex-1 flex-col gap-0.5', dim && 'opacity-60')}>
        <span className="text-sm text-fg-1">{label}</span>
        <span className="text-xs text-fg-2">{text}</span>
        {hosts && <span className="font-mono text-2xs break-words text-fg-3">{hosts}</span>}
        {note && <span className="text-xs text-accent-fg">{note}</span>}
      </div>
      {control}
    </li>
  )
}

/** What this machine connects to on its own, each with a switch, and Offline above them (ADR 0134). */
export function NetworkSettingsSection() {
  const queries = useQueryClient()
  const network = useQuery({ queryKey: keys.networkSettings, queryFn: api.networkSettings })
  const artifacts = useQuery({ queryKey: keys.artifactSettings, queryFn: api.artifactSettings })
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })

  if (network.isError) {
    return (
      <section aria-label="Network" className="flex flex-col gap-3.5">
        <Head />
        <div className="flex flex-col gap-1.5 rounded-md bg-danger-tint px-4 py-3 text-sm" role="alert">
          <span className="font-medium text-danger">Couldn't read this machine's network settings</span>
          <span className="text-fg-1">
            {message(network.error)}. Until it's fixed, Silt stays offline. Fix or delete <Code>network.json</Code> in Silt's data
            folder, or change a switch here to write it again.
          </span>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="primary" onClick={() => void network.refetch()}>
            Retry
          </Button>
        </div>
      </section>
    )
  }

  if (!network.data || !artifacts.data) {
    return (
      <section aria-label="Network" className="flex flex-col gap-3.5" aria-busy="true">
        <Head />
        <Skeleton className="h-16" />
        <div className="overflow-hidden rounded-md border border-line bg-surface-1">
          <h3 className="px-3 pt-2.5 pb-1.5 font-medium">What Silt connects to on its own</h3>
          <ul>
            {[0, 1, 2, 3].map((index) => (
              <li key={index} className="flex flex-col gap-1.5 border-t border-line-subtle px-3 py-3">
                <Skeleton className="h-4 w-40" />
                <Skeleton className="h-4 w-full" />
              </li>
            ))}
          </ul>
        </div>
      </section>
    )
  }

  const view = network.data
  const settings = view.settings
  const offline = view.offline
  const save = (next: NetworkSettings) =>
    api.setNetworkSettings(next).then(
      () => void queries.invalidateQueries({ queryKey: keys.networkSettings }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const saveProxy = (proxy: boolean) =>
    api.setArtifactSettings({ proxy }).then(
      () => void queries.invalidateQueries({ queryKey: keys.artifactSettings }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const servers = status.data?.servers ?? []

  return (
    <section aria-label="Network" className="flex flex-col gap-3.5">
      <Head />
      <div
        className={cx(
          'flex items-start gap-3 rounded-md border px-4 py-3',
          offline ? 'border-accent bg-accent-tint' : 'border-line bg-elevated',
        )}
      >
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-fg-1">Offline</span>
            {offline && (
              <span className="inline-flex items-center rounded-sm bg-accent px-1.5 font-sans text-label text-on-accent">On</span>
            )}
          </div>
          <span className="text-xs text-fg-2">
            {offline
              ? 'Silt connects only to your servers. Downloads you start, such as a local model or a plugin, still go ahead.'
              : 'Turn off everything below at once, so Silt connects only to your servers. Downloads you start, such as a local model or a plugin, still go ahead.'}
          </span>
          {view.offline_env && (
            <span className="text-xs text-fg-2">
              Set by <Code>SILT_OFFLINE=1</Code> in the daemon's environment, so it can't be changed here.
            </span>
          )}
        </div>
        <Switch
          on={offline}
          label="Offline"
          disabled={view.offline_env}
          onChange={(on) => void save({ ...settings, offline: on })}
        />
      </div>

      <section aria-label="connections" className="overflow-hidden rounded-md border border-line bg-surface-1">
        <h3 className="px-3 pt-2.5 pb-1.5 font-medium">What Silt connects to on its own</h3>
        <ul>
          {ROWS.map((row) => {
            const locked = row.key === 'update_checks' && view.no_update_checks_env
            const note = offline ? (
              'Off while offline.'
            ) : locked ? (
              <>
                Off: <Code>SILT_UPDATE_CHECK=never</Code> is set in the daemon's environment.
              </>
            ) : undefined
            return (
              <Line
                key={row.key}
                label={row.label}
                text={row.text}
                hosts={row.hosts}
                note={note}
                dim={offline}
                control={
                  <Switch
                    on={!offline && !locked && settings[row.key]}
                    label={row.label}
                    disabled={offline || locked}
                    onChange={(on) => void save({ ...settings, [row.key]: on })}
                  />
                }
              />
            )
          })}
          <Line
            label={PROXY_LABEL}
            text="Silt fetches each library and font once and keeps it, so opening an artifact never contacts a CDN, and pages you've opened work offline. Off, each page loads them from the CDNs itself."
            hosts="cdnjs.cloudflare.com · cdn.jsdelivr.net · unpkg.com · fonts.googleapis.com · 3 more"
            note={offline ? 'Offline: pages get only the files Silt already kept.' : undefined}
            dim={offline}
            control={
              <Switch
                on={offline || artifacts.data.proxy}
                label={PROXY_LABEL}
                disabled={offline}
                onChange={(on) => void saveProxy(on)}
              />
            }
          />
        </ul>
      </section>

      <section aria-label="always" className="overflow-hidden rounded-md border border-line bg-surface-1">
        <h3 className="px-3 pt-2.5 pb-1.5 font-medium">Always, offline or not</h3>
        <ul>
          <Line
            label="Your servers"
            text="Threads sync with their home server and nowhere else. Secret-looking values are replaced before they leave."
            hosts={
              servers.length
                ? servers.map((server) => serverName(server.url)).join(' · ')
                : 'none yet: every thread stays on this machine'
            }
          />
          <Line
            label="Your agents' providers"
            text="Claude Code, Codex and the others talk to their own providers, under their own settings. Silt doesn't see or change that traffic. A local model keeps the conversation on this machine."
          />
        </ul>
      </section>
    </section>
  )
}
