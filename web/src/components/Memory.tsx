import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useSearch } from '@tanstack/react-router'
import { useMemo, useState, type ReactNode } from 'react'
import {
  api,
  type CopilotMemoryView,
  type MemoryCount,
  type MemoryEntryView,
  type MemoryStoreView,
  type MemoryView,
  type RulesFileView,
} from '../api'
import { useNow } from '../now'
import { keys, message } from '../query'
import { agentName } from '../rules'
import { SETTINGS_HEADING } from '../settingsContext'
import { ago } from '../time'
import { Button, EmptyState, Icon, Select, Skeleton, Switch, Tag, Textarea, cx, toast, type Tone } from '../ui'
import { AgentMark } from './AgentMark'
import { Markdown } from './Markdown'

/** Where Settings › Memory shows: everywhere's, or a project's by its clone. */
const EVERYWHERE = 'everywhere'

/** Whose the memory everywhere is: Silt's own, which agents read in every project. */
const SILT = 'silt'

/** What an agent, or Silt, is called. */
function who(agent: string): string {
  return agent === SILT ? 'Silt' : agentName(agent)
}

const INTRO =
  "What your agents write down to remember in later sessions: about you, everywhere, and about each project. Silt reads it from each agent's own folders on this machine."

/** The kinds Claude Code files its memories under, as the viewer names them. */
const KINDS: { kind: string; label: string; tone: Tone }[] = [
  { kind: 'user', label: 'about you', tone: 'info' },
  { kind: 'feedback', label: 'feedback', tone: 'accent' },
  { kind: 'project', label: 'project', tone: 'neutral' },
  { kind: 'reference', label: 'reference', tone: 'ok' },
]

const CHIP_LABELS: Record<string, string> = {
  user: 'About you',
  feedback: 'Feedback',
  project: 'Project',
  reference: 'References',
}

/** Settings › Memory (ADR 0137): what agents remember everywhere and in each project, and a place's memories. */
export function MemorySettings() {
  const { memory } = useSearch({ strict: false }) as { memory?: string }
  if (!memory) return <Overview />
  return <Place clone={memory === EVERYWHERE ? null : memory} />
}

function Heading({ children, meta }: { children: ReactNode; meta?: string }) {
  return (
    <div className="flex min-h-7 items-center gap-2.5">
      <h2 className={SETTINGS_HEADING}>{children}</h2>
      {meta && <span className="font-mono text-2xs text-fg-2">{meta}</span>}
    </div>
  )
}

function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`
}

/** "3 memories · 2 files", leaving out what there's none of. */
function amount(count: MemoryCount): string {
  const parts = [
    count.memories > 0 && plural(count.memories, 'memory', 'memories'),
    count.files > 0 && plural(count.files, 'file', 'files'),
  ].filter(Boolean)
  return parts.length > 0 ? parts.join(' · ') : 'nothing yet'
}

/** `ago`'s dense form as a phrase: "2h ago", "yesterday". */
function since(ms: number, now: number): string {
  const said = ago(ms, now)
  return /^\d/.test(said) && !said.includes(' ') ? `${said} ago` : said
}

function Chevron() {
  return <Icon name="chevronRight" className="h-3.5 w-3.5 shrink-0 text-fg-2" />
}

function Marks({ agents }: { agents: string[] }) {
  if (agents.length === 0) return null
  return (
    <span className="flex items-center gap-1.5 text-fg-2" title={`Written by ${agents.map(who).join(', ')}`}>
      {agents.map((agent) => (
        <AgentMark key={agent} agent={agent} className="h-3.5 w-3.5" />
      ))}
    </span>
  )
}

function Unread({ agents }: { agents: string[] }) {
  return (
    <>
      {agents.map((agent) => (
        <Tag key={agent} tone="danger">
          {agentName(agent)} unread
        </Tag>
      ))}
    </>
  )
}

function Overview() {
  const overview = useQuery({ queryKey: keys.memory, queryFn: api.memory })
  const now = useNow()

  if (overview.isError) {
    return (
      <section aria-label="Memory" className="flex flex-col gap-3.5">
        <Heading>Memory</Heading>
        <Failure
          text={`Couldn't read what your agents remember: ${message(overview.error)}`}
          retry={() => void overview.refetch()}
        />
      </section>
    )
  }

  if (!overview.data) {
    return (
      <section aria-label="Memory" className="flex flex-col gap-3.5" aria-busy="true">
        <Heading meta="reading your agents' memory…">Memory</Heading>
        <p className="text-sm text-fg-2">{INTRO}</p>
        <div className="overflow-hidden rounded-md border border-line bg-surface-1">
          <h3 className="px-3 pt-2.5 pb-1.5 font-medium">Projects</h3>
          <ul>
            {[0, 1, 2].map((index) => (
              <li key={index} className="flex items-center gap-3 border-t border-line-subtle px-3 py-3">
                <div className="flex min-w-0 flex-1 flex-col gap-2">
                  <Skeleton className="h-3.5 w-24" />
                  <Skeleton className="h-3 w-64" />
                </div>
                <Skeleton className="h-3.5 w-12" />
              </li>
            ))}
          </ul>
        </div>
      </section>
    )
  }

  const { everywhere, projects } = overview.data
  const total = everywhere.memories + projects.reduce((sum, project) => sum + project.count.memories, 0)
  const unread = [...new Set([...everywhere.unread, ...projects.flatMap((project) => project.count.unread)])]
  const nothing = total === 0 && everywhere.files === 0 && projects.every((project) => project.count.files === 0)
  const changed = (count: MemoryCount) => (count.changed_ms ? ` · changed ${since(count.changed_ms, now)}` : '')

  return (
    <section aria-label="Memory" className="flex flex-col gap-3.5">
      <Heading meta={plural(total, 'memory', 'memories')}>Memory</Heading>
      <p className="text-sm text-fg-2">{INTRO}</p>
      <Sharing />
      {unread.length > 0 && (
        <Failure
          text={`Couldn't read what ${unread.map(agentName).join(' and ')} remember${unread.length === 1 ? 's' : ''} on this computer. Everything else is below.`}
          retry={() => void overview.refetch()}
        />
      )}
      {nothing ? (
        <EmptyState title="No memories yet">
          As they work, agents note how you like things done, the decisions behind the code and where to find things. Each shows
          here under its project. What Gemini CLI remembered before Silt joins the next time an agent starts in its project.
        </EmptyState>
      ) : (
        <>
          <section aria-label="everywhere" className="overflow-hidden rounded-md border border-line bg-surface-1">
            <Link
              to="/settings/$section"
              params={{ section: 'memory' }}
              search={{ memory: EVERYWHERE }}
              className="flex items-center gap-3 px-3 py-2.5 hover:bg-surface-2 max-md:min-h-11"
            >
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="text-sm font-medium text-fg-1">Everywhere</span>
                <span className="text-xs text-fg-2">
                  {everywhere.memories + everywhere.files === 0
                    ? 'Nothing yet: what agents keep about you, read in every project'
                    : `${amount(everywhere)} every agent reads in every project${changed(everywhere)}`}
                </span>
              </div>
              <Unread agents={everywhere.unread} />
              <Marks agents={everywhere.agents} />
              <Chevron />
            </Link>
          </section>
          <section aria-label="projects" className="overflow-hidden rounded-md border border-line bg-surface-1">
            <h3 className="px-3 pt-2.5 pb-1.5 font-medium">Projects</h3>
            {projects.length === 0 ? (
              <p className="border-t border-line-subtle px-3 py-2.5 text-xs text-fg-2">
                No project has a clone on this computer.
              </p>
            ) : (
              <ul>
                {projects.map((project) => (
                  <li key={project.clone} className="border-t border-line-subtle">
                    <Link
                      to="/settings/$section"
                      params={{ section: 'memory' }}
                      search={{ memory: project.clone }}
                      title={project.clone}
                      className="flex items-center gap-3 px-3 py-2.5 hover:bg-surface-2 max-md:min-h-11"
                    >
                      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                        <span className="truncate font-mono text-sm font-medium text-fg-1">{project.name}</span>
                        <span className="text-xs text-fg-2">
                          {amount(project.count)}
                          {changed(project.count)}
                        </span>
                      </div>
                      <Unread agents={project.count.unread} />
                      <Marks agents={project.count.agents} />
                      <Chevron />
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </>
      )}
    </section>
  )
}

/** Whether agents share memory (ADR 0137): on, every agent reads and adds to one memory per project and the memory everywhere. */
function Sharing() {
  const queries = useQueryClient()
  const settings = useQuery({ queryKey: keys.memorySettings, queryFn: api.memorySettings })
  const share = settings.data?.share ?? true
  const save = (on: boolean) =>
    api.setMemorySettings({ share: on }).then(
      () => void queries.invalidateQueries({ queryKey: keys.memorySettings }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <div className="flex items-start gap-3 rounded-md border border-line bg-elevated px-4 py-3">
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="text-sm font-medium text-fg-1">Share memory between agents</span>
        <span className="text-xs text-fg-2">
          {share
            ? 'Claude Code, Codex, Gemini CLI and your other agents read and add to one memory in each project, and to your memory everywhere. Off, each agent keeps its own, as it does without Silt.'
            : "Each agent keeps its own memory, as it does without Silt. What's shared already stays here."}
        </span>
      </div>
      <Switch on={share} label="Share memory between agents" disabled={!settings.data} onChange={(on) => void save(on)} />
    </div>
  )
}

function Failure({ text, retry }: { text: string; retry: () => void }) {
  return (
    <div role="alert" className="flex items-start gap-2 rounded-md bg-danger-tint px-3 py-2 text-sm text-danger">
      <Icon name="alert" className="mt-0.5 shrink-0" />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <span>{text}</span>
        <span>
          <button type="button" className="font-medium underline underline-offset-2 max-md:min-h-11" onClick={retry}>
            Try again
          </button>
        </span>
      </div>
    </div>
  )
}

/** A memory with the store it's in. */
type Found = { store: MemoryStoreView; entry: MemoryEntryView }

function Place({ clone }: { clone: string | null }) {
  const view = useQuery({
    queryKey: keys.memoryOf(clone),
    queryFn: () => api.memoryOf(clone),
  })
  const title = clone ? (view.data?.project ?? clone.split('/').pop() ?? clone) : 'Everywhere'
  const heading = (meta?: string) => (
    <Heading meta={meta}>
      <Link to="/settings/$section" params={{ section: 'memory' }} className="text-fg-2 hover:text-fg-1">
        Memory
      </Link>
      <span className="text-fg-3"> › </span>
      <span className={cx(clone && 'font-mono')}>{title}</span>
    </Heading>
  )

  if (view.isError) {
    return (
      <section aria-label={`Memory of ${title}`} className="flex flex-col gap-3.5">
        {heading()}
        <Failure text={`Couldn't read this memory: ${message(view.error)}`} retry={() => void view.refetch()} />
      </section>
    )
  }
  if (!view.data) {
    return (
      <section aria-label={`Memory of ${title}`} className="flex flex-col gap-3.5" aria-busy="true">
        {heading()}
        <Skeleton className="h-8" />
        <Skeleton className="h-48" />
      </section>
    )
  }
  return <PlaceBody view={view.data} clone={clone} heading={heading} />
}

function PlaceBody({ view, clone, heading }: { view: MemoryView; clone: string | null; heading: (meta?: string) => ReactNode }) {
  const [search, setSearch] = useState('')
  const [kind, setKind] = useState<string | null>(null)
  const [open, setOpen] = useState<string | null>(null)
  const found: Found[] = useMemo(
    () => view.stores.flatMap((store) => store.entries.map((entry) => ({ store, entry }))),
    [view.stores],
  )
  const files = view.stores.filter((store) => store.kind !== 'memory' && store.text !== null)
  const failed = view.stores.filter((store) => store.error)
  const kinds = KINDS.filter(({ kind: candidate }) => found.some(({ entry }) => entry.kind === candidate))
  const words = search.trim().toLowerCase()
  const shown = found.filter(
    ({ entry }) =>
      (kind === null || entry.kind === kind) &&
      (!words || [entry.name, entry.description, entry.body].some((text) => text.toLowerCase().includes(words))),
  )
  const name = clone ? view.project : null

  return (
    <section aria-label={`Memory of ${name ?? 'everywhere'}`} className="flex flex-col gap-3.5">
      {heading(
        found.length > 0 || files.length === 0
          ? plural(found.length, 'memory', 'memories')
          : plural(files.length, 'file', 'files'),
      )}
      <p className="text-sm text-fg-2">
        {name
          ? `What agents remember about ${name}, from their folders for it on this computer.`
          : 'What holds in every project: what agents keep about you, and the files each reads in every project.'}
      </p>
      {failed.map((store) => (
        <div
          key={store.path}
          role="alert"
          className="flex items-start gap-2 rounded-md bg-danger-tint px-3 py-2 text-sm text-danger"
        >
          <Icon name="alert" className="mt-0.5 shrink-0" />
          <span>
            Couldn't read {who(store.agent)}'s <code className="font-mono text-xs">{store.path}</code>: {store.error}
          </span>
        </div>
      ))}

      {found.length > 0 && (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <label className="sr-only" htmlFor="memory-search">
              Search memories
            </label>
            <input
              id="memory-search"
              type="search"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder={name ? `Search ${name}'s memory` : 'Search what is remembered everywhere'}
              className="h-8 min-w-[220px] flex-1 rounded-md border border-line bg-elevated px-2.5 text-sm text-fg-1 placeholder:text-fg-3 max-md:h-11"
            />
            {kinds.length > 1 && (
              <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="kind">
                <KindChip selected={kind === null} onClick={() => setKind(null)}>
                  All {found.length}
                </KindChip>
                {kinds.map(({ kind: candidate }) => (
                  <KindChip key={candidate} selected={kind === candidate} onClick={() => setKind(candidate)}>
                    {CHIP_LABELS[candidate]} {found.filter(({ entry }) => entry.kind === candidate).length}
                  </KindChip>
                ))}
              </div>
            )}
          </div>
          <section aria-label="memories" className="overflow-hidden rounded-md border border-line bg-surface-1">
            {shown.length === 0 ? (
              <p className="px-3 py-2.5 text-xs text-fg-2">No memory matches.</p>
            ) : (
              <ul>
                {shown.map(({ store, entry }, index) => {
                  const id = `${store.path}/${entry.file}`
                  return (
                    <MemoryRow
                      key={id}
                      store={store}
                      entry={entry}
                      clone={clone}
                      first={index === 0}
                      open={open === id}
                      toggle={() => setOpen(open === id ? null : id)}
                    />
                  )
                })}
              </ul>
            )}
          </section>
        </>
      )}

      {found.length === 0 && files.length === 0 && (view.rules.every((rules) => rules.text === null) || !clone) && (
        <EmptyState title="Nothing remembered here yet">
          {name
            ? `Agents working in ${name} note decisions, how you like things done and where to find things. They show here as they write them.`
            : 'What agents learn about you, and the rules files each reads in every project, show here.'}
        </EmptyState>
      )}

      {files.length > 0 && (
        <section aria-label="agents' own files" className="overflow-hidden rounded-md border border-line bg-surface-1">
          <h3 className="px-3 pt-2.5 pb-1.5 font-medium">{name ? "Agents' own files" : "Agents' own files, in every project"}</h3>
          <ul>
            {files.map((store) => (
              <FileRow key={store.path} store={store} clone={clone} />
            ))}
          </ul>
        </section>
      )}

      {clone && view.copilot && <CopilotRow clone={clone} copilot={view.copilot} />}

      {clone && view.rules.some((rules) => rules.text !== null) && (
        <section aria-label="rules files" className="overflow-hidden rounded-md border border-line bg-surface-1">
          <h3 className="px-3 pt-2.5 pb-1.5 font-medium">Rules files</h3>
          <p className="px-3 pb-2 text-xs text-fg-2">
            Written by people and committed, so the whole team reads them. Move a memory here when everyone should follow it.
          </p>
          <ul>
            {view.rules
              .filter((rules) => rules.text !== null)
              .map((rules) => (
                <RulesRow key={rules.file} rules={rules} />
              ))}
          </ul>
        </section>
      )}
    </section>
  )
}

/** Whether the project's memory goes to Copilot Memory on GitHub too, which Silt can send to but not read (ADR 0137). */
function CopilotRow({ clone, copilot }: { clone: string; copilot: CopilotMemoryView }) {
  const queries = useQueryClient()
  const now = useNow()
  const refresh = () => {
    void queries.invalidateQueries({ queryKey: keys.memoryOf(clone) })
    // Sending runs in the background; look again once it's had time.
    window.setTimeout(() => void queries.invalidateQueries({ queryKey: keys.memoryOf(clone) }), 4000)
  }
  const failed = (failure: unknown) => toast(message(failure), 'danger')
  const set = (on: boolean) => api.setCopilotMemory({ clone, on }).then(refresh, failed)
  const send = () => api.sendCopilotMemory().then(refresh, failed)
  const status = !copilot.repo
    ? 'Only for a project on GitHub: this clone’s origin isn’t.'
    : [
        copilot.repo,
        copilot.on
          ? copilot.sending
            ? 'sending'
            : copilot.sent_ms
              ? `sent ${since(copilot.sent_ms, now)}`
              : 'not sent yet'
          : 'off',
        copilot.on && copilot.waiting > 0 ? `${copilot.waiting} to send` : '',
      ]
        .filter(Boolean)
        .join(' · ')
  return (
    <section aria-label="Copilot Memory" className="overflow-hidden rounded-md border border-line bg-surface-1">
      <div className="flex items-start gap-3 px-3 py-2.5">
        <span className="mt-0.5 text-fg-2">
          <AgentMark agent="copilot-cli" className="h-4 w-4" />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-sm text-fg-1">Also save to Copilot Memory</span>
          <span className="text-xs text-fg-2">
            GitHub Copilot keeps its memory on GitHub, where Silt can't read it back. On, this project's memory is sent there too
            with the Copilot CLI, and what changes is sent every few minutes, so Copilot has it outside Silt.
          </span>
          <span className="font-mono text-2xs break-words text-fg-3">{status}</span>
          {copilot.error && <span className="text-xs text-danger">{copilot.error}</span>}
          {copilot.on && (
            <span>
              <Button size="sm" onClick={() => void send()} disabled={copilot.sending}>
                Send now
              </Button>
            </span>
          )}
        </div>
        <Switch on={copilot.on} label="Also save to Copilot Memory" disabled={!copilot.repo} onChange={(on) => void set(on)} />
      </div>
    </section>
  )
}

function KindChip({ selected, onClick, children }: { selected: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      className={cx(
        'rounded-sm border px-2 py-0.5 text-xs max-md:min-h-9',
        selected ? 'border-accent bg-accent-tint text-accent-fg' : 'border-line text-fg-2 hover:bg-surface-2',
      )}
    >
      {children}
    </button>
  )
}

function KindTag({ kind }: { kind: string | null }) {
  const known = KINDS.find((candidate) => candidate.kind === kind)
  if (!kind) return null
  return <Tag tone={known?.tone ?? 'neutral'}>{known?.label ?? kind}</Tag>
}

function useChange(clone: string | null) {
  const queries = useQueryClient()
  const refresh = () => void queries.invalidateQueries({ queryKey: keys.memory })
  return {
    save: (store: MemoryStoreView, file: string | null, text: string) =>
      api.setMemory({ clone, store: store.path, file, text }).then(
        () => {
          refresh()
          return true
        },
        (failure: unknown) => {
          toast(message(failure), 'danger')
          return false
        },
      ),
    move: (store: MemoryStoreView, entry: MemoryEntryView, to: { clone: string; name: string } | null) =>
      api.moveMemory({ clone, store: store.path, file: entry.file, to: to?.clone ?? null }).then(
        () => {
          refresh()
          toast(to ? `Moved ${entry.name} to ${to.name}` : `${entry.name} is remembered everywhere now`, 'ok')
        },
        (failure: unknown) => toast(message(failure), 'danger'),
      ),
    forget: (store: MemoryStoreView, entry: MemoryEntryView) =>
      api.forgetMemory({ clone, store: store.path, file: entry.file }).then(
        () => {
          refresh()
          toast(`Forgot ${entry.name}`, 'ok')
        },
        (failure: unknown) => toast(message(failure), 'danger'),
      ),
  }
}

function MemoryRow({
  store,
  entry,
  clone,
  first,
  open,
  toggle,
}: {
  store: MemoryStoreView
  entry: MemoryEntryView
  clone: string | null
  first: boolean
  open: boolean
  toggle: () => void
}) {
  const now = useNow()
  const { save, move, forget } = useChange(clone)
  const overview = useQuery({ queryKey: keys.memory, queryFn: api.memory, enabled: open && clone === null })
  const [moving, setMoving] = useState(false)
  const [editing, setEditing] = useState<string | null>(null)
  const [forgetting, setForgetting] = useState(false)
  const when = entry.modified_ms ? ago(entry.modified_ms, now) : ''
  return (
    <li className={cx(!first && 'border-t border-line-subtle', open && 'bg-elevated shadow-[inset_2px_0_0_var(--accent)]')}>
      <button
        type="button"
        aria-expanded={open}
        onClick={toggle}
        className="flex w-full items-start gap-3 px-3 py-2.5 text-left hover:bg-surface-2 max-md:flex-wrap"
      >
        <span className="flex min-w-0 flex-1 flex-col gap-0.5 max-md:order-last max-md:basis-full">
          <span className={cx('text-sm text-fg-1', open && 'font-medium')}>{entry.name}</span>
          {entry.description && <span className="text-xs text-fg-2">{entry.description}</span>}
        </span>
        <KindTag kind={entry.kind} />
        <span className="flex w-24 shrink-0 items-center justify-end gap-1.5 font-mono text-2xs text-fg-2 max-md:ml-auto">
          <AgentMark agent={store.agent} className="h-3 w-3" />
          {when}
        </span>
      </button>
      {open && (
        <div className="flex flex-col gap-3 px-3 pb-3">
          {editing === null ? (
            <div className="rounded-md border border-line-subtle bg-bg px-3 py-2.5 text-sm">
              <Markdown text={entry.body} />
            </div>
          ) : (
            <Editor
              label={`Edit ${entry.file}`}
              value={editing}
              onChange={setEditing}
              onCancel={() => setEditing(null)}
              onSave={() => void save(store, entry.file, editing).then((saved) => saved && setEditing(null))}
            />
          )}
          <dl className="grid grid-cols-[90px_1fr] gap-x-3 gap-y-1 text-xs">
            <dt className="font-mono text-fg-2">written by</dt>
            <dd className="min-w-0 text-fg-1">
              <span className="inline-flex flex-wrap items-center gap-1.5">
                <AgentMark agent={store.agent} className="h-3 w-3" />
                {who(store.agent)}
                {entry.thread && (
                  <>
                    {' in '}
                    <Link to="/t/$id" params={{ id: entry.thread.id }} className="text-accent-fg underline underline-offset-2">
                      {entry.thread.title ?? 'a thread'}
                    </Link>
                  </>
                )}
              </span>
            </dd>
            {when && (
              <>
                <dt className="font-mono text-fg-2">changed</dt>
                <dd className="min-w-0 text-fg-1">{entry.modified_ms ? since(entry.modified_ms, now) : ''}</dd>
              </>
            )}
            <dt className="font-mono text-fg-2">file</dt>
            <dd className="min-w-0 font-mono break-words text-fg-1">
              {store.path}/{entry.file}
            </dd>
          </dl>
          {editing === null && store.editable && (
            <div className="flex flex-wrap items-center gap-2">
              <Button icon="edit" onClick={() => setEditing(entry.text)}>
                Edit
              </Button>
              {clone !== null && store.kind === 'memory' && (
                <Button onClick={() => void move(store, entry, null)}>Use everywhere</Button>
              )}
              {clone === null && store.agent === SILT && !moving && (
                <Button onClick={() => setMoving(true)}>Move to a project</Button>
              )}
              {moving && (
                <>
                  <label className="sr-only" htmlFor={`move-${entry.file}`}>
                    Project to move it to
                  </label>
                  <Select
                    id={`move-${entry.file}`}
                    defaultValue=""
                    onChange={(event) => {
                      const project = overview.data?.projects.find((candidate) => candidate.clone === event.target.value)
                      if (project) void move(store, entry, { clone: project.clone, name: project.name })
                    }}
                  >
                    <option value="" disabled>
                      Choose a project
                    </option>
                    {overview.data?.projects.map((project) => (
                      <option key={project.clone} value={project.clone}>
                        {project.name}
                      </option>
                    ))}
                  </Select>
                  <Button variant="ghost" onClick={() => setMoving(false)}>
                    Cancel
                  </Button>
                </>
              )}
              {forgetting ? (
                <>
                  <span className="text-xs text-fg-2">Forget it for every agent that reads this folder?</span>
                  <Button variant="danger" onClick={() => void forget(store, entry)}>
                    Forget
                  </Button>
                  <Button variant="ghost" onClick={() => setForgetting(false)}>
                    Keep it
                  </Button>
                </>
              ) : (
                <Button variant="danger" onClick={() => setForgetting(true)}>
                  Forget
                </Button>
              )}
            </div>
          )}
        </div>
      )}
    </li>
  )
}

function Editor({
  label,
  value,
  onChange,
  onSave,
  onCancel,
}: {
  label: string
  value: string
  onChange: (value: string) => void
  onSave: () => void
  onCancel: () => void
}) {
  return (
    <div className="flex flex-col gap-2">
      <Textarea
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        rows={Math.min(24, Math.max(6, value.split('\n').length + 1))}
        className="font-mono text-xs"
      />
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" onClick={onSave}>
          Save
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  )
}

/** A whole file's text as shown: Codex's summary starts with a line naming its format, `v1`. */
function shownText(store: MemoryStoreView): string {
  const text = store.text ?? ''
  return store.kind === 'summary' ? text.replace(/^v\d+\n/, '') : text
}

/** What an agent's whole file is, in a line. */
function fileNote(store: MemoryStoreView): string {
  if (store.kind === 'summary') return 'A summary it rebuilds from its sessions; only it changes this.'
  if (store.agent === 'gemini-cli') return 'Your rules for it, where it also adds what it learns about you.'
  if (store.agent === 'claude-code') return 'Your rules for it, which it calls your user-level memory.'
  return 'Your rules for it, read in every session.'
}

function FileRow({ store, clone }: { store: MemoryStoreView; clone: string | null }) {
  const [open, setOpen] = useState(false)
  const [editing, setEditing] = useState<string | null>(null)
  const { save } = useChange(clone)
  return (
    <li className="border-t border-line-subtle">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex w-full items-start gap-3 px-3 py-2.5 text-left hover:bg-surface-2"
      >
        <span className="mt-0.5 text-fg-2">
          <AgentMark agent={store.agent} className="h-4 w-4" />
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="font-mono text-sm break-all text-fg-1">{store.path}</span>
          <span className="text-xs text-fg-2">
            {who(store.agent)}: {fileNote(store)}
          </span>
        </span>
        <Icon name={open ? 'chevronDown' : 'chevronRight'} className="mt-1 h-3.5 w-3.5 shrink-0 text-fg-2" />
      </button>
      {open && (
        <div className="flex flex-col gap-2 px-3 pb-3">
          {editing === null ? (
            <>
              <div className="rounded-md border border-line-subtle bg-bg px-3 py-2.5 text-sm">
                <Markdown text={shownText(store)} />
              </div>
              {store.editable && (
                <div>
                  <Button icon="edit" onClick={() => setEditing(store.text ?? '')}>
                    Edit
                  </Button>
                </div>
              )}
            </>
          ) : (
            <Editor
              label={`Edit ${store.path}`}
              value={editing}
              onChange={setEditing}
              onCancel={() => setEditing(null)}
              onSave={() => void save(store, null, editing).then((saved) => saved && setEditing(null))}
            />
          )}
        </div>
      )}
    </li>
  )
}

function RulesRow({ rules }: { rules: RulesFileView }) {
  const [open, setOpen] = useState(false)
  return (
    <li className="border-t border-line-subtle">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex w-full items-start gap-3 px-3 py-2.5 text-left hover:bg-surface-2"
      >
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="font-mono text-sm text-fg-1">{rules.file}</span>
          <span className="text-xs text-fg-2">Read by {rules.readers.map(agentName).join(', ')}</span>
        </span>
        <Icon name={open ? 'chevronDown' : 'chevronRight'} className="mt-1 h-3.5 w-3.5 shrink-0 text-fg-2" />
      </button>
      {open && (
        <div className="px-3 pb-3">
          <div className="rounded-md border border-line-subtle bg-bg px-3 py-2.5 text-sm">
            <Markdown text={rules.text ?? ''} />
          </div>
        </div>
      )}
    </li>
  )
}
