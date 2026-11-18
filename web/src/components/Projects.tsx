import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { type ReactNode, useId, useState } from 'react'
import { DEFAULT_PRESETS, PRESETS } from '../agentState'
import { missingSystem } from '../artifacts'
import { api, type GitNotes, type Project, type WorkspaceView } from '../api'
import { serverLine, workspaceOf } from '../rules'
import { projectName } from '../projects'
import { keys, message } from '../query'
import { hosted, serverName } from '../served'
import { Button, EmptyState, Icon, Input, Select, Skeleton, Tag, buttonClass, cx, toast } from '../ui'
import { useWorkspace } from '../workspace'
import { ImportOffer } from './ImportOffer'
import { Mock } from './Mock'
import { Page } from './Page'
import { WorkspaceBadge } from './WorkspaceSwitcher'

/** Where a clone of `project` might go: next to this machine's other clones. */
function suggestedPath(project: Project, projects: Project[]): string {
  const other = projects.find((candidate) => candidate.clone)?.clone
  if (!other) return ''
  const parent = other.replace(/\/+$/, '').replace(/\/[^/]*$/, '')
  return `${parent}/${project.name}`
}

/** A path cut to the part that tells clones apart, its last two folders; the whole path is its title. */
function shortPath(path: string): string {
  const parts = path.replace(/\/+$/, '').split('/')
  return parts.length > 3 ? `…/${parts.slice(-2).join('/')}` : path
}

/** Whether this machine may set `project`'s defaults: it has a clone, and the project is on a server. */
const configurable = (project: Project) => Boolean(project.clone && project.server)

const CHANGING = "Changing a project's defaults takes an owner of its org or the server's admin."

/** Where the project's new threads go from this machine (ADR 0032). */
function ProjectHome({ project }: { project: Project }) {
  const queries = useQueryClient()
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const workspaces = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces, enabled: !hosted() })
  const locked = workspaceOf(workspaces.data, project.repo, project.workspace)?.effective.home_locked
  const servers = status.data?.servers ?? []
  if (servers.length === 0) return null
  const chosen = project.home ?? servers[0].url
  const choose = (home: string) =>
    api.projectHome(project.repo, home).then(
      (said) => {
        toast(said, 'ok')
        void queries.invalidateQueries({ queryKey: keys.projects })
        void queries.invalidateQueries({ queryKey: keys.threads })
      },
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <label className="flex flex-wrap items-center gap-2 text-xs text-fg-2">
      New threads sync with
      <Select
        aria-label={`where ${project.name} syncs`}
        value={chosen}
        disabled={Boolean(locked)}
        title={locked ? "Its workspace's rules keep it on the workspace's server" : undefined}
        onChange={(event) => void choose(event.target.value)}
        className="text-xs"
      >
        {servers.map((server, index) => (
          <option key={server.url} value={server.url}>
            {server.url}
            {index === 0 ? ' (the default)' : ''}
          </option>
        ))}
        <option value="local">This machine only</option>
      </Select>
    </label>
  )
}

/** The design system the project's artifacts take on this machine, above the repository's default and the workspace's (ADR 0071). */
function ProjectDesignSystem({ project }: { project: Project }) {
  const queries = useQueryClient()
  const clone = project.clone ?? ''
  const systems = useQuery({ queryKey: keys.cloneDesignSystems(clone), queryFn: () => api.designSystems(clone) })
  const picks = useQuery({ queryKey: keys.projectDesignSystems, queryFn: api.projectDesignSystems })
  if (!systems.data || !picks.data) return null
  const choose = (name: string) =>
    api.setProjectDesignSystem(project.repo, name || null).then(
      () => void queries.invalidateQueries({ queryKey: keys.projectDesignSystems }),
      (failure: unknown) => toast(message(failure), 'danger'),
    )
  return (
    <label className="flex flex-wrap items-center gap-2 text-xs text-fg-2">
      Artifacts' design system
      <Select
        aria-label={`${project.name}'s design system`}
        value={picks.data[project.repo] ?? ''}
        onChange={(event) => void choose(event.target.value)}
        className="text-xs"
      >
        <option value="">The repository's, else the workspace's</option>
        {systems.data.map((system) => (
          <option key={system.name} value={system.name}>
            {system.title}
          </option>
        ))}
        {missingSystem(picks.data[project.repo], systems.data) && (
          <option value={picks.data[project.repo]}>{picks.data[project.repo]} (not on this machine)</option>
        )}
        <option value="none">None</option>
      </Select>
    </label>
  )
}

/** Which workspace the project is in, and why: by rule, set by the person, or matched nothing (ADR 0065). */
function ProjectWorkspace({ project, views }: { project: Project; views: WorkspaceView[] }) {
  const queries = useQueryClient()
  const [refused, setRefused] = useState<string>()
  const choose = (workspace: string) => {
    setRefused(undefined)
    api.assignProject(project.repo, workspace === '' ? null : workspace).then(
      (said) => {
        toast(said, 'ok')
        for (const key of [keys.projects, keys.workspaces, keys.threads]) void queries.invalidateQueries({ queryKey: key })
      },
      (failure: unknown) => setRefused(message(failure)),
    )
  }
  return (
    <span className="flex flex-col gap-1">
      <label className="flex flex-wrap items-center gap-2 text-xs text-fg-2">
        Workspace
        <Select
          aria-label={`${project.name}'s workspace`}
          value={project.workspace ?? ''}
          onChange={(event) => choose(event.target.value)}
          className="text-xs"
        >
          {views.map((view) => (
            <option key={view.workspace.id} value={view.workspace.id}>
              {view.workspace.name}
            </option>
          ))}
        </Select>
        {project.workspace_why && <span className="font-mono text-2xs">{project.workspace_why}</span>}
        {project.workspace_why === 'set by you' && (
          <Button variant="ghost" size="sm" onClick={() => choose('')}>
            Back to the rules
          </Button>
        )}
      </label>
      {refused && (
        <span role="alert" className="rounded-sm bg-danger-tint px-2 py-1 text-xs text-fg-1">
          {refused}
        </span>
      )}
    </span>
  )
}

export function CloneHere({ project, projects }: { project: Project; projects: Project[] }) {
  const queries = useQueryClient()
  const [path, setPath] = useState(() => suggestedPath(project, projects))
  const [cloning, setCloning] = useState(false)
  const [error, setError] = useState<string>()
  if (!project.origin_url) return <span className="text-xs text-fg-2">No one has said where to clone it from yet</span>
  return (
    <form
      className="flex flex-wrap items-center gap-2"
      onSubmit={(event) => {
        event.preventDefault()
        setError(undefined)
        setCloning(true)
        api
          .cloneProject(project.origin_url!, path)
          .then(
            () => {
              toast(`Cloned ${project.name}`, 'ok')
              return queries.invalidateQueries({ queryKey: keys.projects })
            },
            (failure: unknown) => setError(message(failure)),
          )
          .finally(() => setCloning(false))
      }}
    >
      <Input
        aria-label={`where to clone ${project.name}`}
        className="min-w-48 flex-1 font-mono text-xs"
        placeholder="/home/me/src/app"
        value={path}
        onChange={(event) => setPath(event.target.value)}
      />
      <Button type="submit" disabled={!path.trim() || cloning}>
        {cloning ? 'Cloning…' : 'Clone here'}
      </Button>
      {error && <span className="w-full text-xs text-danger">{error}</span>}
    </form>
  )
}

function Section({ title, children }: { title: ReactNode; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="font-mono text-2xs tracking-[0.06em] text-fg-2 uppercase">{title}</h2>
      {children}
    </section>
  )
}

const ROW = 'flex flex-wrap items-center gap-3 rounded-md border border-line bg-elevated px-3 py-2.5'

/** One setting on one row: what it is, its control, and what it does. On a phone they stack. */
function SettingRow({ id, label, help, children }: { id?: string; label: string; help: ReactNode; children: ReactNode }) {
  return (
    <div className="grid gap-1 border-t border-line-subtle py-2.5 md:grid-cols-[168px_minmax(0,300px)_minmax(0,1fr)] md:items-center md:gap-4">
      {id ? (
        <label htmlFor={id} className="text-fg-2">
          {label}
        </label>
      ) : (
        <span className="text-fg-2">{label}</span>
      )}
      {children}
      <span className="text-2xs text-fg-2">{help}</span>
    </div>
  )
}

/** A project's defaults for its new threads: agent and permissions, and who sees them (ADR 0023). */
function ProjectDefaults({ project }: { project: Project }) {
  const queries = useQueryClient()
  const agents = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const id = useId()
  const save = (change: {
    default_agent?: string | null
    default_preset?: string | null
    git_notes?: GitNotes
    forge_comments?: boolean
  }) =>
    api
      .orgChange({
        change: 'project_settings',
        repo: project.repo,
        name: null,
        default_agent: change.default_agent !== undefined ? change.default_agent : project.default_agent,
        default_preset: change.default_preset !== undefined ? change.default_preset : project.default_preset,
        git_notes: change.git_notes ?? null,
        forge_comments: change.forge_comments ?? null,
      })
      .then(
        () => {
          toast(`Saved ${project.name}'s defaults`, 'ok')
          return queries.invalidateQueries({ queryKey: keys.projects })
        },
        (failure: unknown) => toast(message(failure), 'danger'),
      )
  return (
    <div
      role="group"
      aria-label={`defaults for ${project.name}`}
      className="flex flex-col text-xs [&>div:first-child]:border-t-0 [&>span:first-child>div]:border-t-0"
    >
      <Mock id="add.visibility">
        <SettingRow id={`${id}-visibility`} label="Visible to" help="Set by the org; an owner can change it.">
          <Select id={`${id}-visibility`} disabled className="w-full text-xs" defaultValue="org">
            <option value="org">Everyone in {project.org ?? 'the org'} (org default)</option>
          </Select>
        </SettingRow>
      </Mock>
      <SettingRow id={`${id}-agent`} label="Default agent" help="New threads start with it; another can be picked each time.">
        <Select
          id={`${id}-agent`}
          className="w-full text-xs"
          value={project.default_agent ?? ''}
          onChange={(event) => void save({ default_agent: event.target.value || null })}
        >
          <option value="">None; pick each time</option>
          {(agents.data?.agents ?? []).map((agent) => (
            <option key={agent.id} value={agent.id}>
              {agent.name}
            </option>
          ))}
        </Select>
      </SettingRow>
      <SettingRow id={`${id}-preset`} label="Permissions" help="What new threads' agents may do without asking.">
        <Select
          id={`${id}-preset`}
          className="w-full text-xs"
          value={project.default_preset ?? ''}
          onChange={(event) => void save({ default_preset: event.target.value || null })}
        >
          <option value="">Ask before commands (Silt's default)</option>
          {DEFAULT_PRESETS.map((preset) => (
            <option key={preset.id} value={preset.id}>
              {preset.label}
            </option>
          ))}
        </Select>
      </SettingRow>
      <SettingRow id={`${id}-notes`} label="Git notes" help="A note on each commit naming the thread that made it.">
        <Select
          id={`${id}-notes`}
          className="w-full text-xs"
          value={project.git_notes}
          onChange={(event) => void save({ git_notes: event.target.value as GitNotes })}
        >
          <option value="off">Off</option>
          <option value="local">On each commit, kept in the clone</option>
          <option value="push">On each commit, pushed to origin</option>
        </Select>
      </SettingRow>
      <SettingRow id={`${id}-forge`} label="Pull requests" help="Whether Silt comments on a pull request with its thread.">
        <Select
          id={`${id}-forge`}
          className="w-full text-xs"
          value={project.forge_comments ? 'on' : 'off'}
          onChange={(event) => void save({ forge_comments: event.target.value === 'on' })}
        >
          <option value="off">No comment from Silt</option>
          <option value="on">One comment naming the thread</option>
        </Select>
      </SettingRow>
      <Mock id="add.prepare">
        <SettingRow label="New checkouts" help="Found in the repository.">
          <span>
            Run <code className="font-mono">.agents/prepare</code>
          </span>
        </SettingRow>
      </Mock>
    </div>
  )
}

/** The defaults of one of this machine's projects, picked when there are several. */
function Defaults({ projects }: { projects: Project[] }) {
  const owned = projects.filter(configurable)
  const [repo, setRepo] = useState('')
  const project = owned.find((candidate) => candidate.repo === repo) ?? owned[0]
  if (!project) return null
  return (
    <Section
      title={
        <>
          Defaults for <span className="normal-case">{project.name}</span>
        </>
      }
    >
      {owned.length > 1 && (
        <label className="flex items-center gap-2 text-xs text-fg-2">
          Project
          <Select value={project.repo} onChange={(event) => setRepo(event.target.value)} className="text-xs">
            {owned.map((candidate) => (
              <option key={candidate.repo} value={candidate.repo}>
                {candidate.name}
              </option>
            ))}
          </Select>
        </label>
      )}
      <div className="rounded-md border border-line bg-elevated px-3 py-0.5">
        <ProjectDefaults project={project} />
      </div>
      <p className="text-2xs text-fg-2">{CHANGING}</p>
    </Section>
  )
}

/** Adding a project to the workspace: a clone here, one of the org's, or a URL (the Add a project board). */
export function AddProjectPage() {
  const queries = useQueryClient()
  const workspace = useWorkspace()
  const projects = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const [path, setPath] = useState('')
  const [url, setUrl] = useState('')
  const [into, setInto] = useState('')
  const [busy, setBusy] = useState<'path' | 'url' | null>(null)
  const [error, setError] = useState<{ path?: string; url?: string }>({})
  const all = projects.data ?? []
  const here = all.filter((project) => project.clone)
  const elsewhere = all.filter((project) => !project.clone)
  const device = status.data?.servers?.[0]?.name ?? 'this machine'
  const where = workspace.kind === 'server' ? (workspace.org ?? serverName(workspace.server)) : 'your workspace'
  const added = (repo: string) => {
    toast(`Added ${projectName(repo)}`, 'ok')
    setPath('')
    setUrl('')
    setInto('')
    return queries.invalidateQueries({ queryKey: keys.projects })
  }
  if (hosted())
    return (
      <Page width="narrow">
        <EmptyState title="Projects are added from a machine">
          Open Silt on the machine that has the clone, with `silt open`.
        </EmptyState>
      </Page>
    )
  return (
    <Page
      title={`Add a project to ${where}`}
      width="narrow"
      actions={
        <Link to="/settings/$section" params={{ section: 'projects' }} className={buttonClass('ghost')}>
          Done
        </Link>
      }
    >
      <section aria-label="add a project" className="flex flex-col gap-6">
        <Section title={`Clones on ${device}`}>
          <div className="overflow-hidden rounded-md border border-line bg-elevated [&>*]:border-b [&>*]:border-line-subtle [&>*:last-child]:border-b-0">
            <Mock id="add.discovered">
              <div className="flex flex-wrap items-center gap-3 px-3 py-2.5">
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="font-mono text-xs">~/code/scratch</span>
                  <span className="text-2xs text-fg-2">no remote · its threads can only stay on this machine</span>
                </span>
                <Button disabled>Add here only</Button>
              </div>
            </Mock>
            {here.map((project) => (
              <div key={`${project.server ?? ''}:${project.repo}`} className="flex items-center gap-3 px-3 py-2.5 text-fg-2">
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate font-mono text-xs">{project.clone}</span>
                  <span className="text-2xs">{project.org ? `in ${project.org}` : project.repo}</span>
                </span>
                <span className="text-2xs">added</span>
              </div>
            ))}
          </div>
        </Section>
        <Section title="Or a path on this machine">
          <form
            className="flex flex-wrap items-center gap-2"
            onSubmit={(event) => {
              event.preventDefault()
              setError({})
              setBusy('path')
              api
                .addClone(path)
                .then(added, (failure: unknown) => setError({ path: message(failure) }))
                .finally(() => setBusy(null))
            }}
          >
            <Input
              aria-label="clone path"
              className="min-w-48 flex-1 font-mono text-xs"
              placeholder="/home/me/src/app"
              value={path}
              onChange={(event) => setPath(event.target.value)}
            />
            <Button type="submit" variant="primary" disabled={busy !== null || !path.trim()}>
              {busy === 'path' ? 'Adding…' : 'Add'}
            </Button>
            {error.path && <p className="w-full text-xs text-danger">{error.path}</p>}
          </form>
        </Section>
        {elsewhere.length > 0 && (
          <Section title={`In ${where}, not on this machine`}>
            {elsewhere.map((project) => (
              <div key={`${project.server ?? ''}:${project.repo}`} className={ROW}>
                <span className="flex min-w-0 flex-col">
                  <span className="font-mono text-xs">{project.repo}</span>
                  <span className="text-2xs text-fg-2">
                    {project.threads} {project.threads === 1 ? 'thread' : 'threads'}
                  </span>
                </span>
                <span className="min-w-60 flex-1">
                  <CloneHere project={project} projects={all} />
                </span>
              </div>
            ))}
          </Section>
        )}
        <Section title="Or a remote URL">
          <form
            className="flex flex-wrap items-center gap-2"
            onSubmit={(event) => {
              event.preventDefault()
              setError({})
              setBusy('url')
              api
                .cloneProject(url, into)
                .then(added, (failure: unknown) => setError({ url: message(failure) }))
                .finally(() => setBusy(null))
            }}
          >
            <Input
              aria-label="remote URL"
              className="min-w-60 flex-1 font-mono text-xs"
              placeholder="git@github.com:acme/app.git"
              value={url}
              onChange={(event) => setUrl(event.target.value)}
            />
            <Input
              aria-label="where to clone it"
              className="min-w-48 flex-1 font-mono text-xs"
              placeholder="/home/me/src/app"
              value={into}
              onChange={(event) => setInto(event.target.value)}
            />
            <Button type="submit" disabled={busy !== null || !url.trim() || !into.trim()}>
              {busy === 'url' ? 'Cloning…' : 'Clone'}
            </Button>
            {error.url && <p className="w-full text-xs text-danger">{error.url}</p>}
          </form>
        </Section>
        <Defaults projects={all} />
      </section>
    </Page>
  )
}

/** The projects the person works in, with this machine's clone of each and its defaults (ADR 0023). */
export function Projects() {
  const projects = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const agents = useQuery({ queryKey: keys.agents, queryFn: api.agents, enabled: !hosted() })
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const several = (status.data?.servers?.length ?? 0) > 1
  const all = projects.data ?? []
  const settable = (project: Project) => !hosted() && configurable(project)
  // The first project whose defaults this machine sets opens to them; the rest stay folded until asked.
  const first = all.find(settable)
  const [opened, setOpened] = useState<Record<string, boolean>>({})
  // Grouped by this machine's workspaces once the person made some (ADR 0065).
  const workspaces = useQuery({ queryKey: keys.workspaces, queryFn: api.workspaces, enabled: !hosted() })
  const groups: { view?: WorkspaceView; projects: Project[] }[] =
    !hosted() && workspaces.data?.saved
      ? workspaces.data.workspaces
          .map((view) => ({
            view,
            projects: all.filter((project) => (project.workspace ?? workspaces.data.default) === view.workspace.id),
          }))
          .filter((group) => group.projects.length > 0)
      : [{ projects: all }]
  return (
    <Page
      title="Projects"
      meta={`${all.length} ${all.length === 1 ? 'project' : 'projects'}`}
      width="wide"
      actions={
        !hosted() && (
          <Link to="/settings/$section" params={{ section: 'add' }} className={buttonClass('secondary')}>
            <Icon name="plus" />
            Add a project
          </Link>
        )
      }
    >
      <div className="mb-3 empty:hidden">
        <ImportOffer />
      </div>
      {projects.isPending && <Skeleton className="h-16 w-full" />}
      {projects.error && <p className="text-sm text-danger">{message(projects.error)}</p>}
      {projects.isSuccess && all.length === 0 && (
        <EmptyState title="Add a clone to start threads in it">
          From{' '}
          <Link to="/" search={{ pick: 'folder' }} className="text-accent-fg">
            New thread
          </Link>
          , or run <code>silt open</code> in one.
        </EmptyState>
      )}
      {groups.map((group) => (
        <section
          key={group.view?.workspace.id ?? 'all'}
          aria-label={group.view ? `${group.view.workspace.name} projects` : undefined}
          className="mb-5 flex flex-col gap-2 last:mb-0"
        >
          {group.view && (
            <div className="flex flex-wrap items-center gap-2">
              <WorkspaceBadge name={group.view.workspace.name} size={20} />
              <h2 className="text-sm font-medium">{group.view.workspace.name}</h2>
              <span className="font-mono text-2xs text-fg-2">
                {serverLine(group.view)}
                {group.view.workspace.id === workspaces.data?.default && ' · default'}
              </span>
            </div>
          )}
          <ul data-testid="projects" className="overflow-hidden rounded-md border border-line bg-elevated empty:hidden">
            {group.projects.map((project) => {
              const own = settable(project)
              const open = own && (opened[project.repo] ?? project === first)
              return (
                <li
                  key={`${project.server ?? ''}:${project.repo}`}
                  aria-label={project.name}
                  className="flex flex-col gap-1.5 border-b border-line-subtle px-4 py-3 text-sm last:border-b-0"
                >
                  <div className="flex flex-wrap items-center gap-2">
                    {own ? (
                      <button
                        type="button"
                        aria-expanded={open}
                        title={`${open ? 'Hide' : 'Show'} its defaults`}
                        onClick={() => setOpened((was) => ({ ...was, [project.repo]: !open }))}
                        className="-ml-1 inline-flex items-center gap-1.5 rounded-sm px-1 hover:bg-surface-2 max-md:min-h-11"
                      >
                        <Icon name="chevronRight" className={cx('h-3 w-3 text-fg-2 transition-transform', open && 'rotate-90')} />
                        <span className="font-mono font-medium">{project.name}</span>
                      </button>
                    ) : (
                      <span className="font-mono font-medium">{project.name}</span>
                    )}
                    <span className="min-w-0 truncate font-mono text-2xs text-fg-2" title={project.repo}>
                      {shortPath(project.repo)}
                    </span>
                    {project.org && <Tag>{project.org}</Tag>}
                    {several && project.server && <Tag>{serverName(project.server)}</Tag>}
                    <span className="ml-auto text-xs text-fg-2">
                      {project.threads} {project.threads === 1 ? 'thread' : 'threads'}
                      {project.open_mrs > 0 &&
                        ` · ${project.open_mrs} open merge ${project.open_mrs === 1 ? 'request' : 'requests'}`}
                    </span>
                  </div>
                  <div className={cx('flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-fg-2', own && 'md:pl-5')}>
                    {project.clone ? (
                      <span>
                        Cloned here at{' '}
                        <span className="font-mono" title={project.clone}>
                          {shortPath(project.clone)}
                        </span>
                      </span>
                    ) : (
                      !hosted() && <CloneHere project={project} projects={all} />
                    )}
                    {!hosted() && project.clone && <ProjectHome project={project} />}
                    {!hosted() && project.clone && <ProjectDesignSystem project={project} />}
                    {!hosted() && workspaces.data?.saved && (
                      <ProjectWorkspace project={project} views={workspaces.data.workspaces} />
                    )}
                  </div>
                  {open ? (
                    <div className="mt-1 md:pl-5">
                      <ProjectDefaults project={project} />
                    </div>
                  ) : (
                    (project.default_agent || project.default_preset) && (
                      <p className={cx('text-xs text-fg-2', own && 'md:pl-5')}>
                        New threads start with{' '}
                        {[
                          project.default_agent &&
                            (agents.data?.agents.find((agent) => agent.id === project.default_agent)?.name ??
                              project.default_agent),
                          project.default_preset &&
                            (PRESETS.find((preset) => preset.id === project.default_preset)?.label.toLowerCase() ??
                              project.default_preset),
                        ]
                          .filter(Boolean)
                          .join(', ')}
                      </p>
                    )
                  )}
                </li>
              )
            })}
          </ul>
        </section>
      ))}
      {all.some(settable) && <p className="mt-2 text-2xs text-fg-2">{CHANGING}</p>}
    </Page>
  )
}
