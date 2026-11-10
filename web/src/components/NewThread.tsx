import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useSearch } from '@tanstack/react-router'
import { useState } from 'react'
import { PRESETS, permissionModes, startSettings } from '../agentState'
import { api, type CloneInfo, type FoundFolder, type Preset, type Project, type ThreadSummary } from '../api'
import { threadDoing } from '../threadLook'
import { useNow } from '../now'
import { projectName, shortPath } from '../projects'
import { keys, message } from '../query'
import type { NewThreadSearch } from '../router'
import { useMedia } from '../media'
import { formatKeys } from '../shortcuts'
import { store, stored } from '../stored'
import { ago } from '../time'
import { Button, Icon, Input, Kbd, Menu, Popover, Skeleton, Tabs, cx, toast } from '../ui'
import { AgentSettings } from './AgentSettings'
import { type Takes, useAttachments } from '../attachments'
import { AttachButton, AttachedFiles } from './Attachments'
import { ChipSelect } from './ChipSelect'
import { CloudLaunches } from './Cloud'
import { ImportOffer } from './ImportOffer'
import { hosted } from '../served'
import { ComposerBox, RoundButton } from './Composer'
import { Page } from './Page'
import { CloneHere } from './Projects'
import { ThreadMark } from './ThreadMark'
import { addSending } from '../sending'

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
  if (where === 'local') return `Works in your clone at ${shortPath(clone)}${device ? ` on ${device}` : ''}; edits land in your working tree.`
  if (where === 'worktree')
    return `Gets its own checkout under .silt/checkouts${base ? ` from ${base}` : ''}; your clone stays as it is. Bring it back with a merge request.`
  return "Runs claude --cloud from the project's GitHub remote with your own sign-in; it records into a thread here."
}

type PickTab = NonNullable<NewThreadSearch['pick']>

/** Opens a folder Silt hasn't seen as a project, picking it once added. */
function OpenFolder({ found, onAdded }: { found: FoundFolder[]; onAdded: (path: string) => void }) {
  const [path, setPath] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const open = (chosen: string) => {
    setError(undefined)
    setBusy(true)
    api
      .addClone(chosen)
      .then(() => onAdded(chosen), (failure: unknown) => setError(message(failure)))
      .finally(() => setBusy(false))
  }
  const suggested = found.filter((folder) => !folder.added && folder.git && folder.path.toLowerCase().includes(path.trim().toLowerCase())).slice(0, 5)
  return (
    <form
      className="flex flex-col gap-1.5"
      onSubmit={(event) => {
        event.preventDefault()
        if (path.trim()) open(path.trim())
      }}
    >
      <span className="flex gap-1.5">
        <Input aria-label="folder path" autoFocus placeholder="/home/me/code/app" value={path} onChange={(event) => setPath(event.target.value)} className="min-w-0 flex-1 font-mono text-xs" />
        <Button type="submit" variant="primary" disabled={busy || !path.trim()}>
          {busy ? 'Opening…' : 'Open'}
        </Button>
      </span>
      {suggested.length > 0 && (
        <ul aria-label="folders found on this machine" className="flex flex-col">
          {suggested.map((folder) => (
            <li key={folder.path}>
              <button type="button" onClick={() => open(folder.path)} className="flex w-full flex-col rounded-sm px-2 py-1.5 text-left hover:bg-surface-2 max-md:min-h-11">
                <span className="font-mono text-xs">{folder.path}</span>
                <span className="text-2xs text-fg-2">{[folder.repo || 'no remote', sessionsSaid(folder)].filter(Boolean).join(' · ')}</span>
              </button>
            </li>
          ))}
        </ul>
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
          .then(() => onAdded(path), (failure: unknown) => setError(message(failure)))
          .finally(() => setBusy(false))
      }}
    >
      <Input aria-label="project name" autoFocus placeholder="my-app" value={name} onChange={(event) => setName(event.target.value)} className="font-mono text-xs" />
      <span className="flex gap-1.5">
        <Input aria-label="in the folder" placeholder="/home/me/code" value={parent} onChange={(event) => setParent(event.target.value)} className="min-w-0 flex-1 font-mono text-xs" />
        <Button type="submit" variant="primary" disabled={busy || !path}>
          {busy ? 'Creating…' : 'Create'}
        </Button>
      </span>
      <p className="text-2xs text-fg-2">{path ? `Makes ${path} with an empty git repository; no remote until you add one.` : 'A new folder with an empty git repository, for the agent to start from.'}</p>
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
          .then(() => onAdded(into.trim()), (failure: unknown) => setError(message(failure)))
          .finally(() => setBusy(false))
      }}
    >
      <Input aria-label="remote URL" autoFocus placeholder="git@github.com:acme/app.git" value={url} onChange={(event) => setUrl(event.target.value)} className="font-mono text-xs" />
      <span className="flex gap-1.5">
        <Input aria-label="where to clone it" placeholder="/home/me/code/app" value={into} onChange={(event) => setInto(event.target.value)} className="min-w-0 flex-1 font-mono text-xs" />
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
  return (
    <Popover
      label="Pick a project"
      triggerLabel={`project: ${chosen ? projectName(chosen.repo || chosen.path) : 'pick one'}`}
      openOnMount={opening !== undefined}
      triggerClassName="inline-flex items-center gap-1.5 rounded-sm border border-line px-2.5 py-0.5 font-mono text-[22px] font-medium text-fg-1 hover:bg-surface-2 aria-expanded:border-accent aria-expanded:bg-accent-tint"
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
        <div className="flex flex-col gap-2 font-sans text-sm font-normal" onSubmit={(event) => event.stopPropagation()}>
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
            <ul role="listbox" aria-label="projects" className="flex flex-col">
              {clones.map((clone) => (
                <li key={clone.path} role="option" aria-selected={clone.path === value}>
                  <button
                    type="button"
                    onClick={() => {
                      onPick(clone.path)
                      close()
                    }}
                    className={cx('flex w-full items-center gap-2.5 rounded-sm px-2 py-1.5 text-left hover:bg-surface-2 max-md:min-h-11', clone.path === value && 'bg-surface-2')}
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
          )}
          {tab === 'new' && <NewProject near={chosen?.path ?? clones[0]?.path} onAdded={(path) => added(path, close)} />}
          {tab === 'folder' && <OpenFolder found={found.data ?? []} onAdded={(path) => added(path, close)} />}
          {tab === 'clone' && <CloneRepository onAdded={(path) => added(path, close)} />}
          {tab === 'workspace' && (
            <ul className="flex flex-col gap-2">
              {elsewhere.length === 0 && <li className="px-2 py-1.5 text-xs text-fg-2">Every project in your workspace has a clone here.</li>}
              {elsewhere.map((project) => (
                <li key={`${project.server ?? ''}:${project.repo}`} className="flex flex-col gap-1 px-1">
                  <span className="font-mono text-xs font-medium">
                    {project.name} <span className="font-normal text-fg-2">· {project.threads === 1 ? '1 thread' : `${project.threads} threads`}</span>
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
    api.addClone(path).then(() => added(path), (failure: unknown) => setError(message(failure)))
  }
  return (
    <div className="mx-auto flex w-full max-w-[720px] flex-col gap-5 px-4 py-10 md:py-16">
      <div>
        <h2 className="text-[26px] font-medium tracking-[-0.015em]">Pick a folder to start in</h2>
        <p className="mt-1.5 text-fg-2">Silt records what your agent does there, so you can search it, fork it, or carry on from another machine.</p>
      </div>
      <section aria-labelledby="found" className="flex flex-col gap-1.5">
        <h3 id="found" className="flex items-baseline gap-2 px-0.5 font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">
          Found on this machine
          <span className="font-sans tracking-normal normal-case">where your agents have run before</span>
        </h3>
        {found.isPending && <Skeleton className="h-24" />}
        {found.error && <p className="text-xs text-danger">{message(found.error)}</p>}
        {found.isSuccess && folders.length === 0 && <p className="text-xs text-fg-2">No agent sessions on this machine yet. Open a folder, or clone a repository.</p>}
        {folders.length > 0 && (
          <ul className="overflow-hidden rounded-md border border-line bg-elevated">
            {folders.map((folder) => (
              <li key={folder.path} className="flex flex-wrap items-center gap-3 border-b border-line-subtle px-3.5 py-2.5 last:border-b-0">
                <span className="flex min-w-[220px] flex-1 flex-col">
                  <span className="font-mono font-medium">{folder.path}</span>
                  <span className="text-xs text-fg-2">
                    {[
                      folder.git ? folder.repo || 'no remote' : 'not a git repository',
                      sessionsSaid(folder),
                      folder.last_ms !== null && `last ${ago(folder.last_ms, now)}`,
                    ]
                      .filter(Boolean)
                      .join(' · ')}
                  </span>
                  {!startable(folder) && <span className="text-xs text-fg-3">Threads need git: ask an agent to run git init here. Its sessions can be imported now.</span>}
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
        <Button icon="folder" aria-expanded={opening === 'folder'} onClick={() => setOpening(opening === 'folder' ? null : 'folder')}>
          Open another folder
        </Button>
        <Button icon="branch" aria-expanded={opening === 'clone'} onClick={() => setOpening(opening === 'clone' ? null : 'clone')}>
          Clone a repository
        </Button>
      </div>
      {opening === 'new' && <NewProject near={folders[0]?.path} onAdded={added} />}
      {opening === 'folder' && <OpenFolder found={[]} onAdded={added} />}
      {opening === 'clone' && <CloneRepository onAdded={added} />}
      <ImportOffer />
      <p className="text-xs text-fg-2">
        Prefer the terminal? Run <code className="font-mono text-fg-1">silt claude</code> in any clone. It shows up here as it works.
      </p>
    </div>
  )
}

/** The drafts and the threads of the project to pick up instead of starting anew. */
function pickUpSays(thread: ThreadSummary): string {
  if (thread.location === 'moved') return 'Last on another device · resume here or fork'
  const doing = threadDoing(thread)
  if (doing) return doing.charAt(0).toUpperCase() + doing.slice(1)
  const said = [thread.messages > 0 && `${thread.messages} ${thread.messages === 1 ? 'message' : 'messages'}`, thread.edits > 0 && `${thread.edits} ${thread.edits === 1 ? 'edit' : 'edits'}`].filter(Boolean)
  return said.length > 0 ? said.join(' · ') : 'Nothing said yet'
}

function PickUp({ repo, drafts, onDraft }: { repo: string; drafts: Draft[]; onDraft: (draft: Draft) => void }) {
  const now = useNow()
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const recent = (threads.data ?? [])
    .filter((thread) => thread.repo === repo && !thread.mine.settled && !thread.merged)
    .sort((a, b) => b.last_ms - a.last_ms)
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
          {thread.agent && <span className="shrink-0 text-xs text-fg-2">{thread.agent}</span>}
          <span className="min-w-8 shrink-0 text-right font-mono text-2xs whitespace-nowrap text-fg-3" data-volatile>
            {ago(thread.last_ms, now)}
          </span>
        </Link>
      ))}
    </section>
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
  const project = known(picked) ?? known(search.project) ?? known(stored<string>(LAST_PROJECT_KEY, '')) ?? clones[0]?.path ?? ''
  const clone = clones.find((candidate) => candidate.path === project)
  const repo = clone?.repo ?? ''
  const defaults = projectsQuery.data?.find((candidate) => candidate.clone === project)
  const [where, setWhere] = useState<Where>()
  const chosenWhere = where ?? stored<Where>(MODE_KEY + project, 'local')
  const [base, setBase] = useState('')
  const [chosenAgent, setAgent] = useState<string>()
  const [chosenSettings, setChosenSettings] = useState<Record<string, string>>({})
  const [chosenPreset, setPreset] = useState<Preset>()
  const [prompt, setPrompt] = useState(search.prompt ?? '')
  const [drafts, setDrafts] = useState(() => stored<Draft[]>(DRAFTS_KEY, []))
  const [error, setError] = useState<string>()
  const [starting, setStarting] = useState(false)
  const agents = choices.data?.agents ?? []
  const agent = chosenAgent ?? agents.find((candidate) => candidate.id === defaults?.default_agent)?.id ?? agents[0]?.id ?? ''
  const preset: Preset = chosenPreset ?? PRESETS.find((choice) => choice.id === defaults?.default_preset)?.id ?? 'ask_before_commands'
  const branches = useQuery({ queryKey: keys.branches(project), queryFn: () => api.branches(project), enabled: project !== '' && chosenWhere === 'worktree' })
  const agentName = agents.find((candidate) => candidate.id === agent)?.name ?? ''
  // As this browser last saw the agent run; its mode is the permissions chip's to say.
  // As the agent last ran on this machine, kept by the daemon: its modes and its model, effort and the rest.
  const seen = agents.find((candidate) => candidate.id === agent)
  const modes = seen?.modes?.available
  const settings = startSettings(seen?.settings ?? [], modes)
  const [mode, setMode] = useState<string>()
  const chosenMode = mode ?? seen?.modes?.current
  // Claude's modes (Manual, Accept edits, Plan, Auto) are its permissions, which then stand in for the preset (ADR 0048).
  const modesArePermissions = permissionModes(modes)
  const takes: Takes = { agent: agentName || 'the agent', images: seen?.takes_images ?? null, files: seen?.takes_files ?? null }
  const { attached, add, remove, clear } = useAttachments(takes)
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
    if (chosenWhere === 'cloud') {
      if (!prompt.trim()) return setError('Describe the task for the cloud session.')
      setStarting(true)
      api
        .cloudStart(project, prompt.trim())
        .then(
          (launch) => {
            toast(launch.warnings.length > 0 ? `Cloud session started. ${launch.warnings.join(' ')}` : 'Cloud session started', 'ok')
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
          // In the mode chosen here where its modes are its permissions, else the one its preset says; its model and the rest as shown.
          mode: modesArePermissions ? (chosenMode ?? null) : null,
          settings: settings.map((setting) => [setting.id, chosenSettings[setting.id] ?? setting.current]),
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
          if (!background && prompt.trim()) addSending(thread, prompt, attached.map((file) => file.name), 0)
          clear()
          void queries.invalidateQueries({ queryKey: keys.threads })
          if (!background) {
            void navigate({ to: '/t/$id', params: { id: thread } })
            return
          }
          toast(`${agentName} started in a new thread`, 'ok')
          setPrompt('')
        },
        (failure: unknown) => setError(message(failure)),
      )
      .finally(() => setStarting(false))
  }

  if (choices.isSuccess && clones.length === 0) {
    return (
      <Page title="New thread" meta="first open" label="new thread">
        <Welcome />
      </Page>
    )
  }

  const ready = !starting && Boolean(project) && (chosenWhere === 'cloud' ? Boolean(prompt.trim()) : Boolean(agent))
  return (
    <Page title="New thread" flush label="new thread">
      <div className="mx-auto flex w-full max-w-[760px] flex-1 flex-col justify-center gap-[18px] px-4 pt-10 pb-16 md:px-6 md:pt-14">
        <h2 className="flex flex-wrap items-center justify-center gap-2.5 text-center text-[26px] font-medium tracking-[-0.015em] max-md:text-xl">
          What should we work on in
          {choices.isPending ? (
            <Skeleton className="h-8 w-24" />
          ) : (
            <ProjectPicker clones={clones} projects={projectsQuery.data ?? []} value={project} onPick={pick} opening={search.pick} />
          )}
          <span className="-ml-1.5">?</span>
        </h2>
        <form
          aria-label="new thread"
          className="flex flex-col"
          onSubmit={(event) => {
            event.preventDefault()
            start(false)
          }}
        >
          <ComposerBox
            id="first-prompt"
            label="first prompt"
            rows={4}
            value={prompt}
            onChange={setPrompt}
            onSend={() => ready && start(false)}
            onCycle={() => {
              if (modesArePermissions && modes) setMode(modes[(modes.findIndex((choice) => choice.id === chosenMode) + 1) % modes.length]!.id)
              else setPreset(PRESETS[(PRESETS.findIndex((choice) => choice.id === preset) + 1) % PRESETS.length]!.id)
            }}
            placeholder={chosenWhere === 'cloud' ? 'Describe the task for the cloud session.' : 'Describe the task. You can also start the agent and prompt it later.'}
            onFiles={chosenWhere === 'cloud' ? undefined : attach}
            commands={chosenWhere === 'cloud' ? undefined : seen?.commands}
            voice={!hosted()}
            below={chosenWhere !== 'cloud' && <AttachedFiles files={attached} onRemove={remove} />}
            tools={
              chosenWhere !== 'cloud' && (
                <>
                  <AttachButton takes={takes} onFiles={attach} />
                  {agents.length > 0 ? (
                    <ChipSelect
                      ghost
                      label="agent to start"
                      value={agent}
                      onChange={(chosen) => {
                        setAgent(chosen)
                        setMode(undefined)
                        setChosenSettings({})
                      }}
                      options={agents.map((choice) => ({ id: choice.id, name: choice.name, description: choice.description }))}
                    />
                  ) : (
                    choices.isSuccess && <span className="px-2 text-xs text-fg-2">No agent found</span>
                  )}
                  <AgentSettings
                    modes={modesArePermissions ? modes : undefined}
                    mode={chosenMode}
                    onMode={setMode}
                    settings={settings}
                    value={(setting) => chosenSettings[setting.id] ?? setting.current}
                    onSetting={(id, value) => setChosenSettings({ ...chosenSettings, [id]: value })}
                    narrow={narrow}
                  />
                  {!modesArePermissions && (
                    <ChipSelect
                      ghost
                      label="permissions"
                      icon="lock"
                      value={preset}
                      onChange={(chosen) => setPreset(chosen as Preset)}
                      options={PRESETS.map((choice) => ({ id: choice.id, name: choice.label, description: choice.says }))}
                    />
                  )}
                </>
              )
            }
            actions={
              <>
                <span className="flex items-center">
                  <RoundButton type="submit" label="Start" icon="arrowUp" disabled={!ready} busy={starting} title={chosenWhere === 'cloud' ? 'Start in the cloud (⏎)' : 'Start and open it (⏎)'} />
                  <Menu
                    label="More ways to start"
                    trigger={<Icon name="chevronDown" className="h-3.5 w-3.5" />}
                    items={[
                      ...(chosenWhere === 'cloud' ? [] : [{ label: 'Start in the background', onSelect: () => start(true), disabled: !ready }]),
                      { label: 'Save as a draft', onSelect: saveDraft, disabled: !prompt.trim() },
                    ]}
                  />
                </span>
              </>
            }
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
                options={[{ id: '', name: 'current checkout' }, ...(branches.data ?? []).map((branch) => ({ id: branch, name: branch }))]}
              />
            )}
          </div>
          <p className="mt-2 px-0.5 text-xs text-fg-2">{whereSays(chosenWhere, clone?.path ?? '', base, chosenWhere !== 'cloud' && devices > 0 ? device : null)}</p>
          {choices.isSuccess && agents.length === 0 && chosenWhere !== 'cloud' && (
            <p className="mt-1 px-0.5 text-xs text-warn">
              No agent CLI is on the daemon's <code className="font-mono">PATH</code>. Install one, such as Claude Code, then run{' '}
              <code className="font-mono">silt daemon stop</code> so the next start finds it, or add your own in{' '}
              <code className="font-mono">agents.json</code> in Silt's data directory.
            </p>
          )}
          {chosenWhere === 'local' && busyHere && (
            <p className="mt-1 px-0.5 text-xs text-warn">An agent is already working in this project. A second Local one shares its files; pick Worktree to keep them apart.</p>
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
