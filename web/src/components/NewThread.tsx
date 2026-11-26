import { useQuery, useQueryClient } from '@tanstack/react-query'
import { AgentMark } from './AgentMark'
import { Link, useNavigate, useSearch } from '@tanstack/react-router'
import { useEffect, useId, useRef, useState } from 'react'
import {
  PRESETS,
  nextPermission,
  ownModes,
  presetForOwnMode,
  presetUnavailable,
  startSettings,
  startingValue,
} from '../agentState'
import {
  type AgentSetting,
  api,
  type CloneInfo,
  type FoundFolder,
  type Preset,
  type Project,
  type ThreadSummary,
  type WorkspaceView,
} from '../api'
import { agentName, agentRefused, modeRefused, presetRefused, rulesLine, serverLine, within, workspaceOf } from '../rules'
import { threadDoing } from '../threadLook'
import { useNow } from '../now'
import { projectName, shortPath } from '../projects'
import { keys, message } from '../query'
import { byActivity } from '../sections'
import type { NewThreadSearch } from '../router'
import { useMedia } from '../media'
import { formatKeys } from '../shortcuts'
import { store, stored } from '../stored'
import { ago } from '../time'
import { Button, Icon, Input, Kbd, Popover, Skeleton, Tabs, cx, toast } from '../ui'
import { AgentSettings } from './AgentSettings'
import { AgentChip, PermissionChip } from './AgentPickers'
import { ATTACH, useAttachments } from '../attachments'
import { NEW_THREAD_DRAFT, useDraft } from '../drafts'
import { useHeldSend } from '../heldSend'
import { AttachedFiles } from './Attachments'
import { ChipSelect } from './ChipSelect'
import { UsageRing } from './UsageRing'
import { CloudLaunches } from './Cloud'
import { ImportOffer } from './ImportOffer'
import { hosted } from '../served'
import { type Backend, newThreadOn, useBackends, useListings, useMachinesView } from '../desktop'
import { countsOf, machineName } from '../elsewhere'
import { ComposerBox } from './Composer'
import { Page } from './Page'
import { CloneHere } from './Projects'
import { ThreadMark } from './ThreadMark'
import { addSending } from '../sending'
import { useTypeahead } from '../typeahead'
import { usePathCompletion, withoutTrailingSeparator } from '../pathCompletion'
import { SeasonalTree, Snowfall } from './Seasonal'

interface Draft {
  text: string
  project: string
  at: number
}

const DRAFTS_KEY = 'silt-stashed-prompts'

/** When a draft is saved, read as it's saved. */
const savedAt = () => Date.now()
const LAST_PROJECT_KEY = 'silt-last-project'
const MODE_KEY = 'silt-mode:'

/** Where a new thread's agent works (ADR 0044): the clone itself, a checkout of its own, or a cloud session. */
type Where = 'local' | 'worktree' | 'cloud'

const WHERE: { id: Where; label: string; says: string }[] = [
  { id: 'local', label: 'Local', says: 'In your clone, on this machine' },
  { id: 'worktree', label: 'Worktree', says: 'A checkout of its own; your clone stays as it is' },
  { id: 'cloud', label: 'Cloud', says: 'A Claude Code cloud session, picked up from anywhere' },
]

function whereSays(where: Where, clone: string, base: string, device: string | null): string {
  if (where === 'local')
    return `Works in your clone at ${shortPath(clone)}${device ? ` on ${device}` : ''}; edits land in your working tree.`
  if (where === 'worktree')
    return `Gets its own checkout under .silt/checkouts${base ? ` from ${base}` : ''}; your clone stays as it is. Bring it back with a merge request.`
  return "Runs claude --cloud from the project's GitHub remote with your own sign-in; it records into a thread here."
}

type PickTab = NonNullable<NewThreadSearch['pick']>

/** Opens a folder Silt hasn't seen as a project, picking it once added; its path completes as it's typed. */
function OpenFolder({ found, onAdded }: { found: FoundFolder[]; onAdded: (path: string) => void }) {
  const [path, setPath] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const completing = usePathCompletion(path, setPath)
  const listId = useId()
  const activeOption = useRef<HTMLLIElement>(null)
  useEffect(() => {
    activeOption.current?.scrollIntoView({ block: 'nearest' })
  }, [completing.active])
  const open = (chosen: string) => {
    setError(undefined)
    setBusy(true)
    api
      .addClone(chosen)
      .then(
        () => onAdded(chosen),
        (failure: unknown) => setError(message(failure)),
      )
      .finally(() => setBusy(false))
  }
  const suggested = found
    .filter((folder) => !folder.added && folder.git && folder.path.toLowerCase().includes(path.trim().toLowerCase()))
    .slice(0, 5)
  const foundAt = new Map(found.map((folder) => [folder.path, folder]))
  return (
    <form
      className="flex flex-col gap-1.5"
      onSubmit={(event) => {
        event.preventDefault()
        if (path.trim()) open(withoutTrailingSeparator(path.trim()))
      }}
    >
      <span className="flex gap-1.5">
        <Input
          aria-label="folder path"
          role="combobox"
          aria-autocomplete="list"
          aria-expanded={completing.matches.length > 0}
          aria-controls={listId}
          aria-activedescendant={completing.active >= 0 ? `${listId}-${completing.active}` : undefined}
          autoFocus
          autoComplete="off"
          spellCheck={false}
          placeholder="/home/me/code/app"
          value={path}
          onChange={(event) => setPath(event.target.value)}
          onKeyDown={completing.onKeyDown}
          className="min-w-0 flex-1 font-mono text-xs"
        />
        <Button type="submit" variant="primary" disabled={busy || !path.trim()}>
          {busy ? 'Opening…' : 'Open'}
        </Button>
      </span>
      {completing.matches.length > 0 ? (
        <ul
          id={listId}
          role="listbox"
          aria-label={`folders in ${completing.dir}`}
          className="flex max-h-60 flex-col overflow-y-auto"
        >
          {completing.matches.map((folder, index) => {
            const known = foundAt.get(`${completing.dir}${folder.name}`)
            const said = known ? sessionsSaid(known) : ''
            return (
              <li
                key={folder.name}
                id={`${listId}-${index}`}
                ref={index === completing.active ? activeOption : undefined}
                role="option"
                aria-selected={index === completing.active}
              >
                <button
                  type="button"
                  tabIndex={-1}
                  // Keeps the focus in the path, to type on from the folder picked.
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => completing.complete(index)}
                  className={cx(
                    'flex w-full items-baseline gap-2 rounded-sm px-2 py-1 text-left hover:bg-surface-2 max-md:min-h-11',
                    index === completing.active && 'bg-surface-2',
                  )}
                >
                  <span className="min-w-0 flex-1 truncate font-mono text-xs">
                    {folder.name}
                    {completing.dir.slice(-1)}
                  </span>
                  {known && said ? (
                    <span className="shrink-0 text-2xs text-fg-2">
                      <SessionMarks folder={known} />
                    </span>
                  ) : (
                    folder.git && <span className="shrink-0 text-2xs text-fg-2">git</span>
                  )}
                </button>
              </li>
            )
          })}
        </ul>
      ) : (
        suggested.length > 0 && (
          <ul aria-label="folders found on this machine" className="flex flex-col">
            {suggested.map((folder) => (
              <li key={folder.path}>
                <button
                  type="button"
                  onClick={() => open(folder.path)}
                  className="flex w-full flex-col rounded-sm px-2 py-1.5 text-left hover:bg-surface-2 max-md:min-h-11"
                >
                  <span className="font-mono text-xs">{folder.path}</span>
                  <span className="inline-flex flex-wrap items-center gap-x-1 text-2xs text-fg-2">
                    {folder.repo || 'no remote'}
                    {folder.sessions.length > 0 && (
                      <>
                        {' · '}
                        <SessionMarks folder={folder} />
                      </>
                    )}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )
      )}
      {error && <p className="text-xs text-danger">{error}</p>}
    </form>
  )
}

/** The folder holding `path`. */
function folderOf(path: string): string {
  return path.replace(/[\\/]+$/, '').replace(/[\\/][^\\/]*$/, '')
}

/** `name` in the folder `parent`, with the separator `parent` uses. */
function inFolder(parent: string, name: string): string {
  const separator = parent.includes('\\') && !parent.includes('/') ? '\\' : '/'
  return `${parent.replace(/[\\/]+$/, '')}${separator}${name}`
}

/** Makes a folder with an empty git repository and picks it: a project from nothing, whose git is up to you and the agent (ADR 0053). */
function NewProject({ near, onAdded }: { near?: string; onAdded: (path: string) => void }) {
  const [name, setName] = useState('')
  const [parent, setParent] = useState(near ? folderOf(near) : '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const path = name.trim() && parent.trim() ? inFolder(parent.trim(), name.trim()) : ''
  return (
    <form
      className="flex flex-col gap-1.5"
      onSubmit={(event) => {
        event.preventDefault()
        setError(undefined)
        setBusy(true)
        api
          .newProject(path)
          .then(
            () => onAdded(path),
            (failure: unknown) => setError(message(failure)),
          )
          .finally(() => setBusy(false))
      }}
    >
      <Input
        aria-label="project name"
        autoFocus
        placeholder="my-app"
        value={name}
        onChange={(event) => setName(event.target.value)}
        className="font-mono text-xs"
      />
      <span className="flex gap-1.5">
        <Input
          aria-label="in the folder"
          placeholder="/home/me/code"
          value={parent}
          onChange={(event) => setParent(event.target.value)}
          className="min-w-0 flex-1 font-mono text-xs"
        />
        <Button type="submit" variant="primary" disabled={busy || !path}>
          {busy ? 'Creating…' : 'Create'}
        </Button>
      </span>
      <p className="text-2xs text-fg-2">
        {path
          ? `Makes ${path} with an empty git repository; no remote until you add one.`
          : 'A new folder with an empty git repository, for the agent to start from.'}
      </p>
      {error && <p className="text-xs text-danger">{error}</p>}
    </form>
  )
}

/** Clones a repository and picks it. */
function CloneRepository({ onAdded }: { onAdded: (path: string) => void }) {
  const [url, setUrl] = useState('')
  const [into, setInto] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  return (
    <form
      className="flex flex-col gap-1.5"
      onSubmit={(event) => {
        event.preventDefault()
        setError(undefined)
        setBusy(true)
        api
          .cloneProject(url.trim(), into.trim())
          .then(
            () => onAdded(into.trim()),
            (failure: unknown) => setError(message(failure)),
          )
          .finally(() => setBusy(false))
      }}
    >
      <Input
        aria-label="remote URL"
        autoFocus
        placeholder="git@github.com:acme/app.git"
        value={url}
        onChange={(event) => setUrl(event.target.value)}
        className="font-mono text-xs"
      />
      <span className="flex gap-1.5">
        <Input
          aria-label="where to clone it"
          placeholder="/home/me/code/app"
          value={into}
          onChange={(event) => setInto(event.target.value)}
          className="min-w-0 flex-1 font-mono text-xs"
        />
        <Button type="submit" variant="primary" disabled={busy || !url.trim() || !into.trim()}>
          {busy ? 'Cloning…' : 'Clone'}
        </Button>
      </span>
      {error && <p className="text-xs text-danger">{error}</p>}
    </form>
  )
}

function sessionsSaid(folder: FoundFolder): string {
  if (folder.sessions.length === 0) return ''
  const parts = folder.sessions.map((sessions) => `${sessions.count} ${sessions.agent}`)
  const total = folder.sessions.reduce((sum, sessions) => sum + sessions.count, 0)
  return `${parts.length > 1 ? `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}` : parts[0]} ${total === 1 ? 'session' : 'sessions'}`
}

/** A folder's sessions by their agents' marks and counts, most first: `[mark] 3 [mark] 2 sessions` (ADR 0114). */
function SessionMarks({ folder }: { folder: FoundFolder }) {
  const total = folder.sessions.reduce((sum, sessions) => sum + sessions.count, 0)
  return (
    <span role="img" aria-label={sessionsSaid(folder)} className="inline-flex items-center gap-2">
      {folder.sessions.map((sessions) => (
        <span
          key={sessions.agent_id || sessions.agent}
          className="inline-flex items-center gap-1 font-mono tabular-nums"
          title={`${sessions.count} ${sessions.agent} ${sessions.count === 1 ? 'session' : 'sessions'}`}
        >
          <AgentMark agent={sessions.agent_id || sessions.agent} name={sessions.agent} className="h-3 w-3" />
          {sessions.count}
        </span>
      ))}
      <span>{total === 1 ? 'session' : 'sessions'}</span>
    </span>
  )
}

function cloneName(clone: CloneInfo): string {
  return projectName(clone.repo || clone.path.replaceAll('\\', '/'))
}

/** This machine's projects, the one picked focused as they open so typing its first letters moves to another. */
function ProjectList({ clones, value, onPick }: { clones: CloneInfo[]; value: string; onPick: (path: string) => void }) {
  const list = useRef<HTMLUListElement>(null)
  const [opened] = useState(() =>
    Math.max(
      0,
      clones.findIndex((clone) => clone.path === value),
    ),
  )
  useEffect(() => {
    const box = list.current
    // Back from another tab, focus stays on the tabs.
    if (!box || box.closest('[role=dialog]')?.contains(document.activeElement)) return
    box.querySelectorAll<HTMLButtonElement>('button')[opened]?.focus()
  }, [opened])
  return (
    <ul ref={list} role="listbox" aria-label="projects" className="flex flex-col">
      {clones.map((clone) => (
        <li key={clone.path} role="option" aria-selected={clone.path === value}>
          <button
            type="button"
            onClick={() => onPick(clone.path)}
            className={cx(
              'flex w-full items-center gap-2.5 rounded-sm px-2 py-1.5 text-left hover:bg-surface-2 focus-visible:bg-surface-2 max-md:min-h-11',
              clone.path === value && 'bg-surface-2',
            )}
          >
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="font-mono font-medium">{clone.repo ? projectName(clone.repo) : clone.path}</span>
              <span className="truncate font-mono text-2xs text-fg-2">{clone.path}</span>
            </span>
            {clone.path === value && <Icon name="check" className="text-accent-fg" />}
          </button>
        </li>
      ))}
      {clones.length === 0 && <li className="px-2 py-1.5 text-xs text-fg-2">No projects on this machine yet.</li>}
    </ul>
  )
}

/** The project picker (ADR 0044): the projects here, a folder, a clone, or one of the workspace's not on this machine. */
function ProjectPicker({
  clones,
  projects,
  value,
  onPick,
  opening,
}: {
  clones: CloneInfo[]
  projects: Project[]
  value: string
  onPick: (path: string) => void
  opening?: PickTab
}) {
  const queries = useQueryClient()
  const [tab, setTab] = useState<PickTab>(opening ?? 'projects')
  const found = useQuery({ queryKey: keys.found, queryFn: api.found, enabled: tab === 'folder' })
  const elsewhere = projects.filter((project) => !project.clone)
  const chosen = clones.find((clone) => clone.path === value)
  const added = (path: string, close: () => void) => {
    void queries.invalidateQueries({ queryKey: keys.agents })
    void queries.invalidateQueries({ queryKey: keys.projects })
    toast(`Added ${projectName(path)}`, 'ok')
    onPick(path)
    close()
  }
  const typeTo = useTypeahead(
    (panel) => [...panel.querySelectorAll<HTMLButtonElement>('[aria-label=projects] button')],
    clones.map(cloneName),
  )
  return (
    <Popover
      label="Pick a project"
      triggerLabel={`project: ${chosen ? projectName(chosen.repo || chosen.path) : 'pick one'}`}
      openOnMount={opening !== undefined}
      triggerClassName="inline-flex items-center gap-1.5 rounded-sm border border-line px-2.5 py-0.5 font-mono text-display-sm font-medium text-fg-1 hover:bg-surface-2 aria-expanded:border-accent aria-expanded:bg-accent-tint"
      panelClassName="w-[440px] max-w-[calc(100vw-20px)] p-2"
      trigger={
        <>
          {chosen ? projectName(chosen.repo || chosen.path) : 'a project'}
          <Icon name="chevronDown" className="h-4 w-4 text-fg-2" />
        </>
      }
    >
      {(close) => (
        // The picker sits in the new thread's form, and React carries a
        // portal's events to it: its own forms' submits stop here.
        <div
          className="flex flex-col gap-2 font-sans text-sm font-normal"
          onSubmit={(event) => event.stopPropagation()}
          onKeyDown={tab === 'projects' ? typeTo : undefined}
        >
          <Tabs
            label="open from"
            tabs={[
              { id: 'projects' as const, label: 'Projects' },
              { id: 'new' as const, label: 'New' },
              { id: 'folder' as const, label: 'Folder' },
              { id: 'clone' as const, label: 'Clone' },
              { id: 'workspace' as const, label: `In your workspace${elsewhere.length ? ` · ${elsewhere.length}` : ''}` },
            ]}
            value={tab}
            onChange={setTab}
          />
          {tab === 'projects' && (
            <ProjectList
              clones={clones}
              value={value}
              onPick={(path) => {
                onPick(path)
                close()
              }}
            />
          )}
          {tab === 'new' && <NewProject near={chosen?.path ?? clones[0]?.path} onAdded={(path) => added(path, close)} />}
          {tab === 'folder' && <OpenFolder found={found.data ?? []} onAdded={(path) => added(path, close)} />}
          {tab === 'clone' && <CloneRepository onAdded={(path) => added(path, close)} />}
          {tab === 'workspace' && (
            <ul className="flex flex-col gap-2">
              {elsewhere.length === 0 && (
                <li className="px-2 py-1.5 text-xs text-fg-2">Every project in your workspace has a clone here.</li>
              )}
              {elsewhere.map((project) => (
                <li key={`${project.server ?? ''}:${project.repo}`} className="flex flex-col gap-1 px-1">
                  <span className="font-mono text-xs font-medium">
                    {project.name}{' '}
                    <span className="font-normal text-fg-2">
                      · {project.threads === 1 ? '1 thread' : `${project.threads} threads`}
                    </span>
                  </span>
                  <CloneHere project={project} projects={projects} />
                </li>
              ))}
            </ul>
          )}
          {/* Only beside the projects the keys pick from. */}
          {tab === 'projects' && (
            <p className="border-t border-line-subtle px-1 pt-1.5 text-2xs text-fg-2 max-md:hidden">
              <Kbd>{formatKeys('n')}</Kbd> starts in the current project · <Kbd>{formatKeys('shift+n')}</Kbd> opens this picker
            </p>
          )}
        </div>
      )}
    </Popover>
  )
}

/** The first open, with no projects yet: the folders the agents have run in, found without asking (ADR 0044). */
function Welcome() {
  const now = useNow()
  const queries = useQueryClient()
  const navigate = useNavigate()
  const found = useQuery({ queryKey: keys.found, queryFn: api.found })
  const [opening, setOpening] = useState<'new' | 'folder' | 'clone' | null>(null)
  const [error, setError] = useState<string>()
  const folders = found.data ?? []
  const startable = (folder: FoundFolder) => folder.git
  const firstStartable = folders.find(startable)?.path
  const added = (path: string) => {
    void queries.invalidateQueries({ queryKey: keys.agents })
    void queries.invalidateQueries({ queryKey: keys.projects })
    void navigate({ to: '/', search: { project: path } })
  }
  const start = (path: string) => {
    setError(undefined)
    api.addClone(path).then(
      () => added(path),
      (failure: unknown) => setError(message(failure)),
    )
  }
  return (
    <div className="mx-auto flex w-full max-w-[720px] flex-col gap-5 px-4 py-10 md:py-16">
      <div>
        <h2 className="text-display font-medium tracking-[-0.015em]">Pick a folder to start in</h2>
        <p className="mt-1.5 text-fg-2">
          Silt records what your agent does there, so you can search it, fork it, or carry on from another machine.
        </p>
      </div>
      <section aria-labelledby="found" className="flex flex-col gap-1.5">
        <h3 id="found" className="flex items-baseline gap-2 px-0.5 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">
          Found on this machine
          <span className="font-sans tracking-normal normal-case">where your agents have run before</span>
        </h3>
        {found.isPending && <Skeleton className="h-24" />}
        {found.error && <p className="text-xs text-danger">{message(found.error)}</p>}
        {found.isSuccess && folders.length === 0 && (
          <p className="text-xs text-fg-2">No agent sessions on this machine yet. Open a folder, or clone a repository.</p>
        )}
        {folders.length > 0 && (
          <ul className="overflow-hidden rounded-md border border-line bg-elevated">
            {folders.map((folder) => (
              <li
                key={folder.path}
                className="flex flex-wrap items-center gap-3 border-b border-line-subtle px-3.5 py-2.5 last:border-b-0"
              >
                <span className="flex min-w-[220px] flex-1 flex-col">
                  <span className="font-mono font-medium">{folder.path}</span>
                  <span className="inline-flex flex-wrap items-center gap-x-1 text-xs text-fg-2">
                    {folder.git ? folder.repo || 'no remote' : 'not a git repository'}
                    {folder.sessions.length > 0 && (
                      <>
                        {' · '}
                        <SessionMarks folder={folder} />
                      </>
                    )}
                    {folder.last_ms !== null && ` · last ${ago(folder.last_ms, now)}`}
                  </span>
                  {!startable(folder) && (
                    <span className="text-xs text-fg-3">
                      Threads need git: ask an agent to run git init here. Its sessions can be imported now.
                    </span>
                  )}
                </span>
                {startable(folder) && (
                  <Button variant={folder.path === firstStartable ? 'primary' : 'secondary'} onClick={() => start(folder.path)}>
                    Start here
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}
        {error && <p className="text-xs text-danger">{error}</p>}
      </section>
      <div className="flex flex-wrap gap-2">
        <Button icon="plus" aria-expanded={opening === 'new'} onClick={() => setOpening(opening === 'new' ? null : 'new')}>
          New project
        </Button>
        <Button
          icon="folder"
          aria-expanded={opening === 'folder'}
          onClick={() => setOpening(opening === 'folder' ? null : 'folder')}
        >
          Open another folder
        </Button>
        <Button
          icon="branch"
          aria-expanded={opening === 'clone'}
          onClick={() => setOpening(opening === 'clone' ? null : 'clone')}
        >
          Clone a repository
        </Button>
      </div>
      {opening === 'new' && <NewProject near={folders[0]?.path} onAdded={added} />}
      {opening === 'folder' && <OpenFolder found={[]} onAdded={added} />}
      {opening === 'clone' && <CloneRepository onAdded={added} />}
      <ImportOffer />
      <p className="text-xs text-fg-2">
        Prefer the terminal? Run <code className="font-mono text-fg-1">silt claude</code> in any clone. It shows up here as it
        works.
      </p>
    </div>
  )
}

/** The drafts and the threads of the project to pick up instead of starting anew. */
function pickUpSays(thread: ThreadSummary): string {
  if (thread.location === 'moved') return 'Last on another device · resume here or fork'
  const doing = threadDoing(thread)
  if (doing) return doing.charAt(0).toUpperCase() + doing.slice(1)
  const said = [
    thread.messages > 0 && `${thread.messages} ${thread.messages === 1 ? 'message' : 'messages'}`,
    thread.edits > 0 && `${thread.edits} ${thread.edits === 1 ? 'edit' : 'edits'}`,
  ].filter(Boolean)
  return said.length > 0 ? said.join(' · ') : 'Nothing said yet'
}

function PickUp({ repo, drafts, onDraft }: { repo: string; drafts: Draft[]; onDraft: (draft: Draft) => void }) {
  const now = useNow()
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const recent = (threads.data ?? [])
    .filter((thread) => thread.repo === repo && !thread.mine.settled && !thread.merged)
    .sort(byActivity)
    .slice(0, 4)
  const mine = drafts.filter((draft) => !draft.project || draft.project === repo)
  if (recent.length === 0 && mine.length === 0) return null
  return (
    <section aria-labelledby="pickup" className="flex flex-col">
      <h2 id="pickup" className="flex px-2.5 pb-1.5 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">
        <span className="flex-1">Pick up in {projectName(repo)}</span>
        {mine.length > 0 && <span className="font-sans tracking-normal normal-case">Drafts · {mine.length}</span>}
      </h2>
      {mine.map((draft) => (
        <button
          key={draft.at}
          type="button"
          onClick={() => onDraft(draft)}
          className="flex items-center gap-2.5 rounded-sm px-2.5 py-2 text-left hover:bg-surface-2 max-md:min-h-11"
        >
          <Icon name="edit" className="h-3.5 w-3.5 shrink-0 text-fg-2" />
          <span className="min-w-0 flex-1 truncate">{draft.text}</span>
          <span className="text-xs text-fg-2">draft</span>
          <span className="w-8 text-right font-mono text-2xs text-fg-3" data-volatile>
            {ago(draft.at, now)}
          </span>
        </button>
      ))}
      {recent.map((thread) => (
        <Link
          key={thread.id}
          to="/t/$id"
          params={{ id: thread.id }}
          className="flex items-center gap-2.5 rounded-sm px-2.5 py-2 text-fg-1 hover:bg-surface-2 max-md:min-h-11"
        >
          <ThreadMark thread={thread} />
          {/* The title, then where it stands in words, so the row says what picking it up means. */}
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate">{thread.title ?? 'Untitled thread'}</span>
            <span className="truncate text-xs text-fg-2">{pickUpSays(thread)}</span>
          </span>
          {/* A slot of its own, so the times line up whatever the agent (ADR 0114). */}
          <span className="flex w-4 shrink-0 justify-center text-fg-2" title={thread.agent ? agentName(thread.agent) : undefined}>
            {thread.agent && <AgentMark agent={thread.agent} labelled className="h-3.5 w-3.5" />}
          </span>
          <span className="min-w-8 shrink-0 text-right font-mono text-2xs whitespace-nowrap text-fg-3" data-volatile>
            {ago(thread.last_ms, now)}
          </span>
        </Link>
      ))}
    </section>
  )
}

/** The workspace a new thread starts in, and its rules, under the composer (ADR 0065). */
export function WorkspaceLine({ view }: { view: WorkspaceView }) {
  const rules = rulesLine(view.effective)
  return (
    <p
      aria-label="workspace"
      className="mt-2 flex flex-wrap items-center gap-2 rounded-md border border-line bg-surface-1 px-3 py-2 text-xs"
    >
      <Icon name="lock" className="h-3.5 w-3.5 text-fg-2" />
      <span className="min-w-0 flex-1">
        In <b className="font-medium">{view.workspace.name}</b> · syncs{' '}
        {view.workspace.server ? `with ${serverLine(view)}` : 'nowhere: this machine only'}
        {rules && ` · ${rules}`} · sees only {view.workspace.name}'s history
      </span>
      <Link to="/settings/$section" params={{ section: 'workspaces' }} className="text-accent-fg hover:underline">
        {view.workspace.name}'s rules
      </Link>
    </p>
  )
}

/** An empty thread: what to work on, in which project, and how (ADR 0044). `/` on a machine. */
export function NewThread() {
  const queries = useQueryClient()
  const search = useSearch({ strict: false }) as NewThreadSearch
  const navigate = useNavigate()
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const projectsQuery = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const clones = choices.data?.clones ?? []
  const known = (path: string | undefined) => (path && clones.some((clone) => clone.path === path) ? path : undefined)
  const [picked, setPicked] = useState<string>()
  // Another machine's New thread names its project by origin (ADR 0132).
  const fromOrigin = search.origin ? clones.find((candidate) => candidate.repo === search.origin)?.path : undefined
  const project =
    known(picked) ?? known(search.project) ?? fromOrigin ?? known(stored<string>(LAST_PROJECT_KEY, '')) ?? clones[0]?.path ?? ''
  const clone = clones.find((candidate) => candidate.path === project)
  const repo = clone?.repo ?? ''
  const defaults = projectsQuery.data?.find((candidate) => candidate.clone === project)
  const [where, setWhere] = useState<Where>()
  const chosenWhere = where ?? stored<Where>(MODE_KEY + project, 'local')
  const [base, setBase] = useState('')
  const [chosenAgent, setAgent] = useState<string>()
  const [chosenSettings, setChosenSettings] = useState<Record<string, string>>({})
  const [chosenPreset, setChosenPreset] = useState<Preset>()
  // One of the agent's own modes to start it in, such as Copilot's Autopilot, from the permissions menu (ADR 0118).
  const [chosenMode, setChosenMode] = useState<string>()
  const setPreset = (preset: Preset) => {
    setChosenPreset(preset)
    setChosenMode(undefined)
  }
  const chooseMode = (mode: string) => {
    setChosenPreset(presetForOwnMode(mode))
    setChosenMode(mode)
  }
  const [prompt, setPrompt] = useDraft(NEW_THREAD_DRAFT)
  // What was typed on another machine's New thread comes along once, then leaves the address.
  useEffect(() => {
    if (search.draft === undefined) return
    if (search.draft) setPrompt(search.draft)
    void navigate({ to: '.', search: { ...search, draft: undefined }, replace: true })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.draft])
  const [drafts, setDrafts] = useState(() => stored<Draft[]>(DRAFTS_KEY, []))
  const [error, setError] = useState<string>()
  const [starting, setStarting] = useState(false)
  const agents = choices.data?.agents ?? []
  // The project's workspace, whose rules grey out what the daemon would refuse (ADR 0065).
  const workspacesQuery = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces, enabled: !hosted() })
  const workspace = workspaceOf(workspacesQuery.data, repo, defaults?.workspace)
  const allowed = (id: string | undefined | null) => (id && !agentRefused(workspace, id) ? id : undefined)
  const agent =
    allowed(chosenAgent) ??
    allowed(agents.find((candidate) => candidate.id === defaults?.default_agent)?.id) ??
    allowed(agents.find((candidate) => candidate.id === workspace?.workspace.defaults.agent)?.id) ??
    agents.find((candidate) => !agentRefused(workspace, candidate.id))?.id ??
    ''
  const wanted: Preset =
    chosenPreset ??
    PRESETS.find((choice) => choice.id === defaults?.default_preset)?.id ??
    workspace?.workspace.defaults.preset ??
    'ask_before_commands'
  const cap = workspace?.effective.max_preset
  const capped: Preset = within(wanted, cap) ? wanted : cap!
  const branches = useQuery({
    queryKey: keys.branches(project),
    queryFn: () => api.branches(project),
    enabled: project !== '' && chosenWhere === 'worktree',
  })
  const agentName = agents.find((candidate) => candidate.id === agent)?.name ?? ''
  // As this browser last saw the agent run; its mode is the permissions chip's to say.
  // As the agent last ran on this machine, kept by the daemon: its modes and its model, effort and the rest.
  const seen = agents.find((candidate) => candidate.id === agent)
  const modes = seen?.modes?.available
  const settings = startSettings(seen?.settings ?? [], modes)
  const agentDefaults = workspace?.workspace.defaults.agents?.[agent]
  const settingValue = (setting: AgentSetting) =>
    startingValue(setting, chosenSettings[setting.id], agentDefaults?.settings?.[setting.id])
  // The preset is the one control for every agent; the agent starts in the mode that does what it says (ADR 0114).
  // One without an auto mode asks as for Ask before commands.
  const refusedPreset = (id: Preset) => presetRefused(workspace, id) ?? presetUnavailable(id, modes, agentName)
  const preset: Preset = refusedPreset(capped) && capped === 'auto' ? 'ask_before_commands' : capped
  const own = ownModes(seen?.modes)
  const inOwn = own.find((mode) => mode.id === chosenMode)?.id
  const refusedOwn = (mode: string) => modeRefused(workspace, agent, mode)
  const { attached, add, remove, clear, busy, failure } = useAttachments()
  // A start asked for while files upload, which goes once they're there (ADR 0121).
  const startOnceUploaded = useRef<boolean | null>(null)
  const attach = (files: File[], pasted = false) => {
    setError(undefined)
    add(files, pasted).catch((failure: unknown) => setError(message(failure)))
  }
  const narrow = useMedia('(max-width: 767px)')
  const devices = status.data?.servers?.length ?? 0
  const device = status.data?.servers?.[0]?.name ?? 'this machine'
  const busyHere = (threads.data ?? []).some((thread) => thread.repo === repo && repo !== '' && thread.agent_state === 'working')

  const pick = (path: string) => {
    setPicked(path)
    setWhere(undefined)
    setBase('')
    store(LAST_PROJECT_KEY, path)
  }
  const chooseWhere = (next: Where) => {
    setWhere(next)
    store(MODE_KEY + project, next)
  }
  const saveDrafts = (next: Draft[]) => {
    setDrafts(next)
    store(DRAFTS_KEY, next)
  }
  const saveDraft = () => {
    if (!prompt.trim()) return
    saveDrafts([{ text: prompt.trim(), project: repo, at: savedAt() }, ...drafts].slice(0, 10))
    setPrompt('')
    toast(attached.length > 0 ? 'Saved as a draft, without its files' : 'Saved as a draft', 'ok')
    clear()
  }

  const start = (background: boolean) => {
    setError(undefined)
    if (failure) {
      setError(failure)
      return
    }
    if (busy) {
      startOnceUploaded.current = background
      return
    }
    if (chosenWhere === 'cloud') {
      if (!prompt.trim()) return setError('Describe the task for the cloud session.')
      setStarting(true)
      api
        .cloudStart(project, prompt.trim())
        .then(
          (launch) => {
            toast(
              launch.warnings.length > 0 ? `Cloud session started. ${launch.warnings.join(' ')}` : 'Cloud session started',
              'ok',
            )
            setPrompt('')
          },
          (failure: unknown) => setError(message(failure)),
        )
        .finally(() => setStarting(false))
      return
    }
    if (!agent || !project) return
    setStarting(true)
    api
      .startAgent(
        agent,
        { clone: project },
        {
          here: chosenWhere === 'local',
          base: chosenWhere === 'worktree' && base ? base : null,
          // In the mode its preset says; its model and the rest as shown.
          mode: inOwn ?? null,
          settings: settings.map((setting) => [setting.id, settingValue(setting)]),
          preset,
          visibility: null,
          prompt: prompt.trim() ? prompt : null,
          plan: false,
          attachments: attached,
        },
      )
      .then(
        (thread) => {
          // The agent takes the first prompt once it's up, seconds later; the thread shows it as sent meanwhile.
          if (!background && prompt.trim())
            addSending(
              thread,
              prompt,
              attached.map((file) => file.name),
              0,
            )
          clear()
          // The draft was this one's; New thread opens empty next time.
          setPrompt('')
          void queries.invalidateQueries({ queryKey: keys.threads })
          if (!background) {
            void navigate({ to: '/t/$id', params: { id: thread } })
            return
          }
          toast(`${agentName} started in a new thread`, 'ok')
        },
        (failure: unknown) => setError(message(failure)),
      )
      .finally(() => setStarting(false))
  }
  useEffect(() => {
    if (busy || startOnceUploaded.current === null) return
    const background = startOnceUploaded.current
    startOnceUploaded.current = null
    start(background)
  })

  const ready = !starting && Boolean(project) && (chosenWhere === 'cloud' ? Boolean(prompt.trim()) : Boolean(agent))
  const send = useHeldSend(choices.isPending, ready, () => start(false))

  if (choices.isSuccess && clones.length === 0) {
    return (
      <Page title="New thread" meta="first open" label="new thread">
        <Welcome />
      </Page>
    )
  }

  return (
    <Page title="New thread" flush label="new thread">
      {/* Snow falls behind the screen, so what's on it is drawn after, over it (ADR 0133). */}
      <Snowfall />
      <div className="relative mx-auto flex w-full max-w-[760px] flex-1 flex-col justify-center gap-[18px] px-4 pt-10 pb-16 md:px-6 md:pt-14">
        <h2 className="relative flex flex-wrap items-center justify-center gap-2.5 text-center text-display font-medium tracking-[-0.015em] max-md:text-xl">
          <SeasonalTree />
          What should we work on in
          {choices.isPending ? (
            <Skeleton className="h-8 w-24" />
          ) : (
            <ProjectPicker
              clones={clones}
              projects={projectsQuery.data ?? []}
              value={project}
              onPick={pick}
              opening={search.pick ?? (search.origin && choices.isSuccess && !fromOrigin ? 'folder' : undefined)}
            />
          )}
          <MachinePicker draft={prompt} origin={repo} />
          <span className="-ml-1.5">?</span>
        </h2>
        <form
          aria-label="new thread"
          className="flex flex-col"
          onSubmit={(event) => {
            event.preventDefault()
            send()
          }}
        >
          <ComposerBox
            id="first-prompt"
            label="first prompt"
            rows={4}
            value={prompt}
            onChange={setPrompt}
            onSend={send}
            onCycle={() => {
              const next = nextPermission(preset, inOwn, own, refusedPreset, refusedOwn)
              if (next?.mode !== undefined) chooseMode(next.mode)
              else if (next) setPreset(next.preset)
            }}
            placeholder={
              chosenWhere === 'cloud'
                ? 'Describe the task for the cloud session.'
                : 'Describe the task. You can also start the agent and prompt it later.'
            }
            onFiles={chosenWhere === 'cloud' ? undefined : attach}
            commands={chosenWhere === 'cloud' ? undefined : seen?.commands}
            voice={!hosted()}
            below={chosenWhere !== 'cloud' && <AttachedFiles files={attached} onRemove={remove} />}
            tools={
              chosenWhere !== 'cloud' && (
                <>
                  {agents.length > 0 && !agent && workspace ? (
                    <span className="px-2 text-xs text-warn-fg">
                      No agent installed here is allowed in {workspace.workspace.name}
                    </span>
                  ) : agents.length > 0 ? (
                    <AgentChip
                      value={agent}
                      agents={agents}
                      onChange={(chosen) => {
                        setAgent(chosen)
                        setChosenSettings({})
                        if (chosenMode !== undefined) {
                          setChosenMode(undefined)
                          setChosenPreset(undefined)
                        }
                      }}
                      refused={(id) => agentRefused(workspace, id)}
                    />
                  ) : (
                    choices.isSuccess && <span className="px-2 text-xs text-fg-2">No agent found</span>
                  )}
                  <PermissionChip
                    value={preset}
                    onChange={setPreset}
                    refused={refusedPreset}
                    own={
                      own.length > 0
                        ? { agent, name: agentName, modes: own, mode: inOwn, onMode: chooseMode, refused: refusedOwn }
                        : undefined
                    }
                  />
                  <AgentSettings
                    settings={settings}
                    value={settingValue}
                    onSetting={(id, value) => setChosenSettings({ ...chosenSettings, [id]: value })}
                    narrow={narrow}
                  />
                  {agent && <UsageRing agent={agent} />}
                </>
              )
            }
            attach={ATTACH}
            enter="start"
            send={{
              label: 'Start',
              title: chosenWhere === 'cloud' ? 'Start in the cloud' : 'Start and open it',
              disabled: !ready,
              busy: starting,
              submit: true,
            }}
            menu={[
              ...(chosenWhere === 'cloud'
                ? []
                : [{ label: 'Start in the background', icon: 'clock' as const, onSelect: () => start(true), disabled: !ready }]),
              { label: 'Save as a draft', icon: 'edit' as const, onSelect: saveDraft, disabled: !prompt.trim() },
            ]}
          />
          <div className="mt-2.5 flex flex-wrap items-center gap-2 px-0.5">
            <div role="radiogroup" aria-label="works in" className="inline-flex overflow-hidden rounded-sm border border-line">
              {WHERE.map((choice) => (
                <button
                  key={choice.id}
                  type="button"
                  role="radio"
                  aria-checked={chosenWhere === choice.id}
                  title={choice.says}
                  onClick={() => chooseWhere(choice.id)}
                  className={cx(
                    'inline-flex h-7 items-center gap-1.5 px-3 text-xs max-md:min-h-11',
                    chosenWhere === choice.id ? 'bg-accent-tint text-accent-fg' : 'text-fg-2 hover:bg-surface-2',
                  )}
                >
                  {choice.id === 'cloud' && <Icon name="cloud" className="h-3.5 w-3.5" />}
                  {choice.label}
                </button>
              ))}
            </div>
            {chosenWhere === 'worktree' && (
              <ChipSelect
                label="base branch"
                prefix="from"
                icon="branch"
                mono={base !== ''}
                value={base}
                onChange={setBase}
                options={[
                  { id: '', name: 'current checkout' },
                  ...(branches.data ?? []).map((branch) => ({ id: branch, name: branch })),
                ]}
              />
            )}
          </div>
          <p className="mt-2 px-0.5 text-xs text-fg-2">
            {whereSays(chosenWhere, clone?.path ?? '', base, chosenWhere !== 'cloud' && devices > 0 ? device : null)}
          </p>
          {workspace && (workspacesQuery.data?.saved || rulesLine(workspace.effective)) && <WorkspaceLine view={workspace} />}
          {choices.isSuccess && agents.length === 0 && chosenWhere !== 'cloud' && (
            <p className="mt-1 px-0.5 text-xs text-warn-fg">
              No agent CLI is on the daemon's <code className="font-mono">PATH</code>. Install one, such as Claude Code, then run{' '}
              <code className="font-mono">silt daemon stop</code> so the next start finds it, or add your own in{' '}
              <code className="font-mono">agents.json</code> in Silt's data directory.
            </p>
          )}
          {chosenWhere === 'local' && busyHere && (
            <p className="mt-1 px-0.5 text-xs text-warn-fg">
              An agent is already working in this project. A second Local one shares its files; pick Worktree to keep them apart.
            </p>
          )}
          {error && <p className="mt-2 text-sm whitespace-pre-wrap text-danger">{error}</p>}
        </form>
        <CloudLaunches />
        {repo && (
          <div className="mt-6">
            <PickUp
              repo={repo}
              drafts={drafts}
              onDraft={(draft) => {
                setPrompt(draft.text)
                saveDrafts(drafts.filter((kept) => kept.at !== draft.at))
              }}
            />
          </div>
        )}
        <ImportOffer />
      </div>
    </Page>
  )
}

/** "on [machine]" in New thread's headline, in All machines (ADR 0132): picking another machine starts the thread there. */
function MachinePicker({ draft, origin }: { draft: string; origin: string }) {
  const view = useMachinesView()
  const { backends, current } = useBackends()
  const listings = useListings()
  const connected = backends.filter(
    (backend) => backend.kind !== 'ssh' || backend.state === 'connected' || backend.id === current,
  )
  const shown = backends.find((backend) => backend.id === current)
  if (view.list !== 'all' || connected.length < 2 || !shown) return null
  const said = (backend: Backend) => {
    const kind = backend.kind === 'ssh' ? 'over SSH' : backend.kind === 'wsl' ? 'WSL' : 'this computer'
    const listing = listings.find((each) => each.id === backend.id)
    const { running } = countsOf(listing)
    return [kind, running > 0 && `${running} running`].filter(Boolean).join(' · ')
  }
  return (
    <>
      <span>on</span>
      <Popover
        label="Pick a machine"
        triggerLabel={`machine: ${machineName(shown)}`}
        triggerValue={shown.id}
        triggerClassName="inline-flex items-center gap-1.5 rounded-sm border border-line px-2.5 py-0.5 text-display-sm font-medium text-fg-1 hover:bg-surface-2 aria-expanded:border-accent aria-expanded:bg-accent-tint"
        panelClassName="w-[300px] max-w-[calc(100vw-20px)] p-1.5"
        trigger={
          <>
            <Icon name={shown.kind === 'ssh' ? 'terminal' : 'laptop'} className="h-4 w-4" />
            {machineName(shown)}
            <Icon name="chevronDown" className="h-4 w-4 text-fg-2" />
          </>
        }
      >
        {(close) => (
          <div className="flex flex-col gap-0.5 text-ui font-normal tracking-normal">
            <p className="px-2.5 pt-1 pb-1.5 text-left text-2xs text-fg-2">
              Starts the thread there. The window switches to it, and what you typed goes along.
            </p>
            {connected.map((backend) => {
              const chosen = backend.id === current
              return (
                <button
                  key={backend.id}
                  type="button"
                  role="menuitemradio"
                  aria-checked={chosen}
                  onClick={() => {
                    close()
                    if (!chosen) newThreadOn(backend.id, draft, origin)
                  }}
                  className={cx(
                    'grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-2.5 rounded-sm px-2.5 py-1.5 text-left text-fg-1 hover:bg-surface-2 max-md:min-h-11',
                    chosen && 'bg-accent-tint',
                  )}
                >
                  <span className="flex min-w-0 flex-col">
                    <b className="truncate font-medium">{machineName(backend)}</b>
                    <span className="truncate font-mono text-2xs text-fg-2">{said(backend)}</span>
                  </span>
                  {chosen ? <Icon name="check" className="text-accent-fg" /> : <span />}
                </button>
              )
            })}
          </div>
        )}
      </Popover>
    </>
  )
}
