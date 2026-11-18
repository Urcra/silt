import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useSearch } from '@tanstack/react-router'
import { type ReactNode, useState } from 'react'
import { api, type ActionSpec, type PluginView, type Rule, type SettingSpec } from '../api'
import { keys, message } from '../query'
import { SETTINGS_HEADING } from '../settingsContext'
import { Banner, Button, Chip, Dialog, EmptyState, Icon, Input, Select, Skeleton, Tag, cx, toast } from '../ui'
import { Markdown } from './Markdown'
import { Page } from './Page'

/** A manifest's text, its `backticked` parts as code. */
function withCode(text: string): ReactNode[] {
  return text.split('`').map((part, index) =>
    index % 2 === 1 ? (
      <code key={index} className="font-mono text-[0.95em]">
        {part}
      </code>
    ) : (
      part
    ),
  )
}

const SUB = 'text-xs text-fg-2'
const ROW = 'grid grid-cols-1 gap-2 border-b border-line-subtle px-4 py-3 last:border-b-0 md:grid-cols-[180px_minmax(0,1fr)]'

/** Where a plugin came from, as its tag and its group say it. */
function source(view: PluginView): { tag: string; group: string } {
  const from = view.installed.source
  switch (from.type) {
    case 'shipped':
      return { tag: 'Silt', group: 'Shipped with Silt' }
    case 'marketplace':
      return { tag: `Marketplace · ${view.manifest?.publisher ?? ''}`, group: 'From the marketplace' }
    case 'git':
      return { tag: 'From git', group: 'From git' }
    case 'dev':
      return { tag: 'dev', group: 'Yours, in development' }
    case 'org':
      return { tag: from.org, group: `From ${from.org}'s policy` }
  }
}

/** What a plugin's source names, in a line. */
function origin(view: PluginView): string {
  const from = view.installed.source
  switch (from.type) {
    case 'shipped':
      return 'shipped with Silt'
    case 'git':
      return `git · ${from.url} @ ${view.installed.version} (${from.commit.slice(0, 7)})`
    case 'dev':
      return `dev · ${from.path}`
    case 'marketplace':
      return `v${view.installed.version}`
    case 'org':
      return `${from.org}'s policy${from.commit ? ` @ ${from.commit.slice(0, 7)}` : ''}`
  }
}

function onIn(view: PluginView, workspace: string): boolean {
  return view.installed.workspaces.includes(workspace) || view.installed.workspaces.includes('*')
}

/** The workspaces chips: each a switch for whether its agents get the plugin. */
function OnIn({ view }: { view: PluginView }) {
  const queries = useQueryClient()
  const workspaces = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces })
  const locked = view.installed.source.type === 'org'
  const turn = (workspace: string, on: boolean) =>
    api.turnPlugin(view.installed.id, workspace, on).then(
      () => void queries.invalidateQueries({ queryKey: keys.plugins }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <span className="flex flex-wrap items-center gap-1.5">
      {(workspaces.data?.workspaces ?? []).map((workspace) => {
        const on = onIn(view, workspace.workspace.id)
        return (
          <Chip
            key={workspace.workspace.id}
            selected={on}
            aria-label={`${view.installed.id} in ${workspace.workspace.name}`}
            aria-pressed={on}
            disabled={locked && on}
            onClick={() => void turn(workspace.workspace.id, !on)}
          >
            {workspace.workspace.name}
          </Chip>
        )
      })}
    </span>
  )
}

/** Settings › Plugins (ADR 0103): every plugin installed here, by where it came from. */
export function PluginsSettings() {
  const plugins = useQuery({ queryKey: keys.plugins, queryFn: api.plugins })
  const [adding, setAdding] = useState(false)
  const views = plugins.data ?? []
  const groups = new Map<string, PluginView[]>()
  for (const view of views) {
    const group = source(view).group
    groups.set(group, [...(groups.get(group) ?? []), view])
  }
  return (
    <section aria-label="Plugins" className="flex flex-col gap-4">
      <h2 className={SETTINGS_HEADING}>Plugins</h2>
      <p className="text-sm text-fg-2">
        What your sandboxed agents reach beyond their checkout, through Silt. A plugin's calls run outside the sandbox with
        credentials the agent never sees; each workspace chooses which plugins its agents get.
      </p>
      <div>
        <Button onClick={() => setAdding(true)}>Add from git…</Button>
      </div>
      {plugins.isPending && (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
        </div>
      )}
      {plugins.isError && <Banner tone="danger">Plugins couldn't be read: {message(plugins.error)}</Banner>}
      {[...groups.entries()].map(([group, members]) => (
        <div key={group} className="flex flex-col gap-2">
          <p className="font-mono text-2xs tracking-[0.14em] text-fg-2 uppercase">{group}</p>
          <div className="overflow-hidden rounded-md border border-line bg-elevated">
            {members.map((view) => (
              <PluginRow key={view.installed.id} view={view} />
            ))}
          </div>
        </div>
      ))}
      {plugins.isSuccess && views.every((view) => view.installed.source.type === 'shipped') && (
        <EmptyState title="Only Silt's own plugins so far">
          Add one from its git repository with <b>Add from git</b>, or{' '}
          <code className="font-mono">silt plugin add &lt;url&gt;#&lt;tag&gt;</code>.
        </EmptyState>
      )}
      <AddFromGit open={adding} onClose={() => setAdding(false)} />
    </section>
  )
}

function PluginRow({ view }: { view: PluginView }) {
  const name = view.manifest?.name ?? view.installed.id
  return (
    <div className="flex flex-col gap-1.5 border-b border-line-subtle px-4 py-3 last:border-b-0">
      <span className="flex flex-wrap items-center gap-2">
        <Link
          to="/settings/plugins/plugin"
          search={{ id: view.installed.id }}
          className="font-medium text-fg-1 hover:text-accent-fg"
        >
          {name}
        </Link>
        <Tag tone={view.installed.source.type === 'shipped' ? 'accent' : 'neutral'}>{source(view).tag}</Tag>
        {!view.trusted && view.installed.source.type !== 'shipped' && <Tag tone="warn">publisher not trusted</Tag>}
      </span>
      {view.manifest && <span className="text-sm text-fg-2">{withCode(view.manifest.description)}</span>}
      <span className="font-mono text-2xs text-fg-2">
        {origin(view)} · WebAssembly, run by Silt
        {view.hosts.length > 0 ? ` · reaches ${view.hosts.join(', ')}` : ''}
      </span>
      <span className="flex items-center gap-2 text-sm text-fg-2">
        On in <OnIn view={view} />
      </span>
    </div>
  )
}

/** Add from git: everything the plugin's manifest declares, shown before it's added. */
function AddFromGit({ open, onClose }: { open: boolean; onClose: () => void }) {
  const queries = useQueryClient()
  const [url, setUrl] = useState('')
  const [preview, setPreview] = useState<{ consent: string[]; commit: string; id: string }>()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const close = () => {
    setPreview(undefined)
    setError(undefined)
    setUrl('')
    onClose()
  }
  const look = () => {
    setBusy(true)
    setError(undefined)
    api
      .previewPlugin(url.trim())
      .then(setPreview, (failure: unknown) => setError(message(failure)))
      .finally(() => setBusy(false))
  }
  const add = () => {
    setBusy(true)
    setError(undefined)
    if (!preview) return
    api
      .addPlugin(url.trim(), preview.commit)
      .then(
        () => {
          void queries.invalidateQueries({ queryKey: keys.plugins })
          toast('Added; turn it on in the workspaces that need it', 'ok')
          close()
        },
        (failure: unknown) => setError(message(failure)),
      )
      .finally(() => setBusy(false))
  }
  return (
    <Dialog open={open} onClose={close} title="Add a plugin from git">
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-sm">
          The repository, with <code className="font-mono">#tag</code> to pin a version
          <Input
            aria-label="repository"
            placeholder="https://github.com/acme/silt-deploys#v2.1.0"
            value={url}
            onChange={(event) => {
              setUrl(event.target.value)
              setPreview(undefined)
            }}
          />
        </label>
        {preview && (
          <div className="flex flex-col gap-1.5 rounded-md border border-line bg-bg px-3 py-2.5" aria-label="what it gets">
            <span className="font-mono text-2xs text-fg-2 uppercase">
              {preview.id} at {preview.commit.slice(0, 7)} gets
            </span>
            {preview.consent.map((line) => (
              <span key={line} className="text-xs whitespace-pre-wrap text-fg-1">
                {line.trim()}
              </span>
            ))}
            <span className={SUB}>It's built here, offline, in a sandbox; nothing of it runs before you add it.</span>
          </div>
        )}
        {error && <Banner tone="danger">{error}</Banner>}
        <span className="flex justify-end gap-1.5">
          <Button onClick={close}>Cancel</Button>
          {preview ? (
            <Button variant="primary" disabled={busy} onClick={add}>
              {busy ? 'Building…' : 'Add it'}
            </Button>
          ) : (
            <Button variant="primary" disabled={busy || !url.trim()} onClick={look}>
              {busy ? 'Reading…' : 'See what it gets'}
            </Button>
          )}
        </span>
      </div>
    </Dialog>
  )
}

const RULES: { value: Rule; label: string }[] = [
  { value: 'allow', label: 'Allowed' },
  { value: 'ask', label: 'Asked' },
  { value: 'never', label: 'Never' },
]

/** A plugin's page (ADR 0103): what it is, how it works, its settings and its rules. */
export function PluginPage() {
  const { id } = useSearch({ strict: false }) as { id: string }
  const plugins = useQuery({ queryKey: keys.plugins, queryFn: api.plugins })
  const view = plugins.data?.find((candidate) => candidate.installed.id === id)
  const manifest = view?.manifest
  return (
    <Page
      title={
        <span className="flex min-w-0 items-center gap-2">
          <Link to="/settings/$section" params={{ section: 'plugins' }} className="text-fg-2 hover:text-fg-1">
            Plugins
          </Link>
          <Icon name="chevronRight" className="shrink-0 text-fg-3" />
          <span className="truncate">{manifest?.name ?? id}</span>
          {view && <Tag tone={view.installed.source.type === 'shipped' ? 'accent' : 'neutral'}>{source(view).tag}</Tag>}
        </span>
      }
      meta={view ? origin(view) : undefined}
      label="plugin"
    >
      {plugins.isPending && <Skeleton className="h-40" />}
      {plugins.isSuccess && !view && <EmptyState title={`No plugin ${id} here`} />}
      {view && manifest && (
        <div className="flex max-w-[860px] flex-col gap-6">
          <p className="text-sm text-fg-2">{withCode(manifest.description)}</p>
          {!view.trusted && <Untrusted publisher={manifest.publisher} />}
          <HowItWorks view={view} />
          {manifest.environments && <Environments view={view} />}
          {manifest.settings.length > 0 && <PluginSettings view={view} />}
          <Actions view={view} />
          {view.readme && (
            <section aria-label="About" className="flex flex-col gap-2">
              <h3 className="text-sm font-medium">About</h3>
              <div className="rounded-md border border-line bg-elevated px-4 py-3">
                <Markdown text={view.readme} />
              </div>
            </section>
          )}
          <Remove view={view} />
        </div>
      )}
    </Page>
  )
}

function Untrusted({ publisher }: { publisher: string }) {
  const queries = useQueryClient()
  const trust = () =>
    api.trustPublisher(publisher, true).then(
      () => void queries.invalidateQueries({ queryKey: keys.plugins }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <Banner tone="warn">
      <span className="flex flex-wrap items-center gap-2">
        You don't trust {publisher} yet, so each of its actions asks, whatever it says it does.
        <Button size="sm" onClick={() => void trust()}>
          Trust {publisher}
        </Button>
      </span>
    </Banner>
  )
}

function Section({ label, children, note }: { label: string; children: ReactNode; note?: ReactNode }) {
  return (
    <section aria-label={label} className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">{label}</h3>
      {note && <p className={SUB}>{note}</p>}
      <div className="overflow-hidden rounded-md border border-line bg-elevated">{children}</div>
    </section>
  )
}

function HowItWorks({ view }: { view: PluginView }) {
  const manifest = view.manifest!
  return (
    <Section label="How it works">
      <div className={ROW}>
        <span className={SUB}>Runs</span>
        <span className="text-sm">
          As WebAssembly in Silt's plugin host, as every plugin does: no files, network or processes beyond the capabilities below
          {view.installed.source.type === 'shipped' ? '. Shipped with Silt, built from its repository.' : '.'}
        </span>
      </div>
      <div className={ROW}>
        <span className={SUB}>Capabilities</span>
        <span className="flex flex-col gap-1.5">
          {manifest.capabilities.map((capability) => (
            <span key={capability.kind} className="flex flex-wrap items-baseline gap-2 text-sm">
              <Tag>{capability.kind}</Tag>
              <span className="text-fg-2">{capability.reason}</span>
              {['sockets', 'ssh', 'mcp-process'].includes(capability.kind) && <Tag tone="warn">Silt can't check its calls</Tag>}
            </span>
          ))}
        </span>
      </div>
      <div className={ROW}>
        <span className={SUB}>Reaches</span>
        <span className="font-mono text-sm">{view.hosts.length ? view.hosts.join(', ') : 'no host'}</span>
      </div>
      {manifest.credentials.length > 0 && (
        <div className={ROW}>
          <span className={SUB}>Credentials</span>
          <span className="flex flex-col gap-1 text-sm">
            {manifest.credentials
              .filter((credential) => !credential.same_as)
              .map((credential) => {
                const kept = Object.entries(view.installed.credentials).filter(
                  ([key]) => key === credential.id || key.startsWith(`${credential.id}@`),
                )
                return (
                  <span key={credential.id}>
                    {credential.label}:{' '}
                    {kept.length ? (
                      kept.map(([key, name]) => (
                        <code key={key} className="mr-2 font-mono">
                          {key.includes('@') ? `${key.split('@')[1]}: ` : ''}
                          {name}
                        </code>
                      ))
                    ) : (
                      <span className="text-warn-fg">none set</span>
                    )}
                    <span className="block text-xs text-fg-2">
                      Sent by Silt to {credential.hosts.join(', ') || 'the hosts you set'} only, never to the plugin.
                    </span>
                  </span>
                )
              })}
          </span>
        </div>
      )}
      <div className={ROW}>
        <span className={SUB}>On in</span>
        <OnIn view={view} />
      </div>
    </Section>
  )
}

function Environments({ view }: { view: PluginView }) {
  const queries = useQueryClient()
  const manifest = view.manifest!
  const names = Object.keys(view.installed.environments)
  const [adding, setAdding] = useState({ name: '', patterns: '' })
  const save = (name: string, patterns: string) =>
    api
      .setPluginEnvironment(
        view.installed.id,
        name,
        patterns.split(',').map((pattern) => pattern.trim()),
      )
      .then(
        () => void queries.invalidateQueries({ queryKey: keys.plugins }),
        (failure: unknown) => toast(message(failure), 'danger'),
      )
  return (
    <Section
      label="Environments"
      note={`A call goes to the environment whose ${manifest.environments!.label} match its ${manifest.environments!.field} most closely; one none names is refused.`}
    >
      {names.length === 0 && <p className="px-4 py-3 text-sm text-fg-2">None yet: every call is refused until one is set.</p>}
      {names.map((name) => (
        <div key={name} className={ROW}>
          <span className="font-mono text-sm">{name}</span>
          <Input
            aria-label={`${name}'s ${manifest.environments!.label}`}
            defaultValue={view.installed.environments[name].join(', ')}
            onBlur={(event) => void save(name, event.target.value)}
          />
        </div>
      ))}
      <div className={ROW}>
        <Input
          aria-label="new environment's name"
          placeholder="acme"
          value={adding.name}
          onChange={(event) => setAdding({ ...adding, name: event.target.value })}
        />
        <span className="flex gap-1.5">
          <Input
            aria-label={`new environment's ${manifest.environments!.label}`}
            placeholder="acme/*, acme-labs/*"
            value={adding.patterns}
            onChange={(event) => setAdding({ ...adding, patterns: event.target.value })}
          />
          <Button
            disabled={!adding.name.trim() || !adding.patterns.trim()}
            onClick={() => void save(adding.name.trim(), adding.patterns).then(() => setAdding({ name: '', patterns: '' }))}
          >
            Add
          </Button>
        </span>
      </div>
    </Section>
  )
}

function settingValue(view: PluginView, spec: SettingSpec, scope: string | null): { value: unknown; yours: boolean } {
  const stored = scope ? `${spec.key}${spec.scope === 'environment' ? '@' : '#'}${scope}` : spec.key
  const settings = view.installed.settings as Record<string, unknown>
  if (stored in settings) return { value: settings[stored], yours: true }
  if (spec.key in settings) return { value: settings[spec.key], yours: scope === null }
  return { value: spec.default ?? null, yours: false }
}

function PluginSettings({ view }: { view: PluginView }) {
  const manifest = view.manifest!
  const environments = Object.keys(view.installed.environments)
  const [environment, setEnvironment] = useState<string | null>(environments[0] ?? null)
  const scoped = manifest.settings.some((spec) => spec.scope === 'environment')
  return (
    <Section
      label="Settings"
      note="Drawn from what the plugin declares. Secrets aren't set here: `silt plugin secret` keeps them in the keyring."
    >
      {scoped && environments.length > 0 && (
        <div className={cx(ROW, 'items-center')}>
          <span className={SUB}>For environment</span>
          <Select aria-label="environment" value={environment ?? ''} onChange={(event) => setEnvironment(event.target.value)}>
            {environments.map((name) => (
              <option key={name}>{name}</option>
            ))}
          </Select>
        </div>
      )}
      {manifest.settings
        .filter((spec) => spec.type !== 'secret')
        .map((spec) => (
          <SettingRow key={spec.key} view={view} spec={spec} scope={spec.scope === 'environment' ? environment : null} />
        ))}
    </Section>
  )
}

function SettingRow({ view, spec, scope }: { view: PluginView; spec: SettingSpec; scope: string | null }) {
  const queries = useQueryClient()
  const { value, yours } = settingValue(view, spec, scope)
  const save = (next: unknown) =>
    api.setPluginSetting(view.installed.id, spec.key, next, spec.scope === 'environment' ? scope : null, null).then(
      () => void queries.invalidateQueries({ queryKey: keys.plugins }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  const shown = Array.isArray(value) ? value.join(', ') : value === null || value === undefined ? '' : String(value)
  const input = (() => {
    switch (spec.type) {
      case 'boolean':
        return (
          <input
            type="checkbox"
            aria-label={spec.label}
            className="accent-[var(--accent)]"
            checked={value === true}
            onChange={(event) => void save(event.target.checked)}
          />
        )
      case 'enum':
        return (
          <Select aria-label={spec.label} value={shown} onChange={(event) => void save(event.target.value)}>
            {(spec.options ?? []).map((option) => (
              <option key={option}>{option}</option>
            ))}
          </Select>
        )
      default:
        return (
          <Input
            key={`${spec.key}-${scope}-${shown}`}
            aria-label={spec.label}
            defaultValue={shown}
            onBlur={(event) => {
              const text = event.target.value.trim()
              if (text === shown) return
              const next =
                spec.type === 'list'
                  ? text
                      .split(',')
                      .map((item) => item.trim())
                      .filter(Boolean)
                  : spec.type === 'number'
                    ? Number(text)
                    : text
              void save(text === '' ? null : next)
            }}
          />
        )
    }
  })()
  return (
    <div className={ROW}>
      <span className="flex flex-col">
        <span className="text-sm">{spec.label}</span>
        <span className="text-2xs text-fg-2">{yours ? 'yours' : 'default'}</span>
      </span>
      <span className="flex flex-col gap-1">
        <span className="flex items-center gap-2">
          {input}
          {yours && (
            <Button size="sm" onClick={() => void save(null)}>
              Reset
            </Button>
          )}
        </span>
        <span className="text-xs text-fg-2">{withCode(spec.description)}</span>
      </span>
    </div>
  )
}

function Actions({ view }: { view: PluginView }) {
  const queries = useQueryClient()
  const manifest = view.manifest!
  const set = (action: ActionSpec, rule: Rule | null) =>
    api.setPluginRule(view.installed.id, action.id, rule, null).then(
      () => void queries.invalidateQueries({ queryKey: keys.plugins }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <Section
      label="What runs without asking"
      note={
        view.installed.source.type === 'shipped'
          ? `Silt classifies each of ${manifest.name}'s calls itself. An org's rule can only make one stricter.`
          : `${manifest.publisher} says which of its calls only read. An org's rule can only make one stricter.`
      }
    >
      {manifest.actions.map((action) => {
        const rules = view.installed.rules.filter((rule) => rule.action === action.id)
        const mine = rules.find((rule) => !rule.pattern && rule.source.from === 'yours')
        const org = rules.find((rule) => rule.source.from === 'org')
        const current = org?.rule ?? mine?.rule ?? (view.trusted ? action.default : 'ask')
        return (
          <div key={action.id} className="flex flex-col gap-1 border-b border-line-subtle px-4 py-3 last:border-b-0">
            <span className="flex flex-wrap items-center gap-2">
              <span className="text-sm font-medium">{action.label}</span>
              <Tag tone={action.kind === 'change' ? 'warn' : 'neutral'}>{action.kind}</Tag>
              {action.once && <Tag>once each</Tag>}
              <span className="ml-auto flex items-center gap-2">
                <Select
                  aria-label={`${action.label}'s rule`}
                  value={current}
                  disabled={Boolean(org)}
                  onChange={(event) => void set(action, event.target.value as Rule)}
                >
                  {RULES.map((rule) => (
                    <option key={rule.value} value={rule.value}>
                      {rule.label}
                    </option>
                  ))}
                </Select>
              </span>
            </span>
            <span className="text-xs text-fg-2">{withCode(action.description)}</span>
            <span className="text-2xs text-fg-2">
              {org
                ? `locked by ${org.source.from === 'org' ? org.source.name : 'your org'}`
                : mine
                  ? 'your rule'
                  : view.trusted
                    ? 'default'
                    : 'asked: its publisher isn’t trusted yet'}
              {mine && (
                <button type="button" className="ml-2 text-accent-fg" onClick={() => void set(action, null)}>
                  reset
                </button>
              )}
            </span>
            {rules
              .filter((rule) => rule.pattern)
              .map((rule) => (
                <span key={`${rule.field}=${rule.pattern}`} className="text-2xs text-fg-2">
                  {RULES.find((choice) => choice.value === rule.rule)?.label} where {rule.field} is{' '}
                  <code className="font-mono">{rule.pattern}</code>
                </span>
              ))}
          </div>
        )
      })}
    </Section>
  )
}

function Remove({ view }: { view: PluginView }) {
  const queries = useQueryClient()
  if (view.installed.source.type === 'shipped' || view.installed.source.type === 'org') return null
  const remove = () =>
    api.removePlugin(view.installed.id).then(
      () => {
        void queries.invalidateQueries({ queryKey: keys.plugins })
        toast(`Removed ${view.manifest?.name ?? view.installed.id}`, 'ok')
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <div>
      <Button variant="danger" onClick={() => void remove()}>
        Remove {view.manifest?.name ?? view.installed.id}
      </Button>
    </div>
  )
}
