import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { type ReactNode, useState } from 'react'
import { DEFAULT_PRESETS, PRESETS } from '../agentState'
import { api, type GitNotes, type Project } from '../api'
import { projectName } from '../projects'
import { keys, message } from '../query'
import { hosted, serverName } from '../served'
import { Button, EmptyState, Input, Select, Skeleton, Tag, buttonClass, toast } from '../ui'
import { useWorkspace } from '../workspace'
import { ImportOffer } from './ImportOffer'
import { Mock } from './Mock'
import { Page } from './Page'

/** Where a clone of `project` might go: next to this machine's other clones. */
function suggestedPath(project: Project, projects: Project[]): string {
  const other = projects.find((candidate) => candidate.clone)?.clone
  if (!other) return ''
  const parent = other.replace(/\/+$/, '').replace(/\/[^/]*$/, '')
  return `${parent}/${project.name}`
}

/** Where the project's new threads go from this machine (ADR 0032). */
function ProjectHome({ project }: { project: Project }) {
  const queries = useQueryClient()
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
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
      <Select aria-label={`where ${project.name} syncs`} value={chosen} onChange={(event) => void choose(event.target.value)} className="text-xs">
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
      <Input aria-label={`where to clone ${project.name}`} className="min-w-48 flex-1 font-mono text-xs" placeholder="/home/me/src/app" value={path} onChange={(event) => setPath(event.target.value)} />
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

/** A project's defaults for its new threads: agent and permissions, and who sees them (ADR 0023). */
function Defaults({ projects }: { projects: Project[] }) {
  const queries = useQueryClient()
  const agents = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const owned = projects.filter((project) => project.clone && project.server)
  const [repo, setRepo] = useState('')
  const project = owned.find((candidate) => candidate.repo === repo) ?? owned[0]
  if (!project) return null
  const save = (change: { default_agent?: string | null; default_preset?: string | null; git_notes?: GitNotes; forge_comments?: boolean }) =>
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
    <Section
      title={
        <>
          Defaults for <span className="normal-case">{project.name}</span>
        </>
      }
    >
      <div className="grid grid-cols-[120px_minmax(0,1fr)] items-center gap-x-3 gap-y-2 rounded-md border border-line bg-elevated p-3 text-xs">
        {owned.length > 1 && (
          <>
            <label htmlFor="defaults-project" className="text-fg-2">
              Project
            </label>
            <Select id="defaults-project" value={project.repo} onChange={(event) => setRepo(event.target.value)} className="text-xs">
              {owned.map((candidate) => (
                <option key={candidate.repo} value={candidate.repo}>
                  {candidate.name}
                </option>
              ))}
            </Select>
          </>
        )}
        <label htmlFor="defaults-visibility" className="text-fg-2">
          Visible to
        </label>
        <Mock id="add.visibility">
          <Select id="defaults-visibility" disabled className="text-xs" defaultValue="org">
            <option value="org">Everyone in {project.org ?? 'the org'} (org default)</option>
          </Select>
        </Mock>
        <label htmlFor="defaults-agent" className="text-fg-2">
          Default agent
        </label>
        <Select id="defaults-agent" value={project.default_agent ?? ''} onChange={(event) => void save({ default_agent: event.target.value || null })} className="text-xs">
          <option value="">None; pick each time</option>
          {(agents.data?.agents ?? []).map((agent) => (
            <option key={agent.id} value={agent.id}>
              {agent.name}
            </option>
          ))}
        </Select>
        <label htmlFor="defaults-preset" className="text-fg-2">
          Permissions
        </label>
        <Select id="defaults-preset" value={project.default_preset ?? ''} onChange={(event) => void save({ default_preset: event.target.value || null })} className="text-xs">
          <option value="">Ask before commands (Silt's default)</option>
          {DEFAULT_PRESETS.map((preset) => (
            <option key={preset.id} value={preset.id}>
              {preset.label}
            </option>
          ))}
        </Select>
        <label htmlFor="defaults-notes" className="text-fg-2">
          Git notes
        </label>
        <Select id="defaults-notes" value={project.git_notes} onChange={(event) => void save({ git_notes: event.target.value as GitNotes })} className="text-xs">
          <option value="off">Off</option>
          <option value="local">On each commit, kept in the clone</option>
          <option value="push">On each commit, pushed to origin</option>
        </Select>
        <label htmlFor="defaults-forge" className="text-fg-2">
          Pull requests
        </label>
        <Select id="defaults-forge" value={project.forge_comments ? 'on' : 'off'} onChange={(event) => void save({ forge_comments: event.target.value === 'on' })} className="text-xs">
          <option value="off">No comment from Silt</option>
          <option value="on">One comment naming the thread</option>
        </Select>
        <span className="text-fg-2">New checkouts</span>
        <Mock id="add.prepare">
          <span>
            run <code className="font-mono">.agents/prepare</code>, found in the repository
          </span>
        </Mock>
      </div>
      <p className="text-2xs text-fg-2">Changing them takes an owner of its org or the server's admin.</p>
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
        <EmptyState title="Projects are added from a machine">Open Silt on the machine that has the clone, with `silt open`.</EmptyState>
      </Page>
    )
  return (
    <Page title={`Add a project to ${where}`} width="narrow" actions={<Link to="/settings/$section" params={{ section: 'projects' }} className={buttonClass('ghost')}>Done</Link>}>
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
            <Input aria-label="clone path" className="min-w-48 flex-1 font-mono text-xs" placeholder="/home/me/src/app" value={path} onChange={(event) => setPath(event.target.value)} />
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
            <Input aria-label="remote URL" className="min-w-60 flex-1 font-mono text-xs" placeholder="git@github.com:acme/app.git" value={url} onChange={(event) => setUrl(event.target.value)} />
            <Input aria-label="where to clone it" className="min-w-48 flex-1 font-mono text-xs" placeholder="/home/me/src/app" value={into} onChange={(event) => setInto(event.target.value)} />
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

/** The projects the person works in, with this machine's clone of each (ADR 0023). */
export function Projects() {
  const projects = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const agents = useQuery({ queryKey: keys.agents, queryFn: api.agents, enabled: !hosted() })
  const status = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const several = (status.data?.servers?.length ?? 0) > 1
  const all = projects.data ?? []
  return (
    <Page
      title="Projects"
      meta={`${all.length} ${all.length === 1 ? 'project' : 'projects'}`}
      width="wide"
      actions={
        !hosted() && (
          <Link to="/settings/$section" params={{ section: 'add' }} className={buttonClass('secondary')}>
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
          From <Link to="/" search={{ pick: 'folder' }} className="text-accent-fg">New thread</Link>, or run <code>silt open</code> in one.
        </EmptyState>
      )}
      <ul data-testid="projects" className="overflow-hidden rounded-md border border-line bg-elevated">
        {all.map((project) => (
          <li key={`${project.server ?? ''}:${project.repo}`} aria-label={project.name} className="flex flex-col gap-1.5 border-b border-line-subtle px-4 py-3 text-sm last:border-b-0">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-mono font-medium">{project.name}</span>
              <span className="font-mono text-2xs text-fg-2">{project.repo}</span>
              {project.org && <Tag>{project.org}</Tag>}
              {several && project.server && <Tag>{serverName(project.server)}</Tag>}
              <span className="ml-auto text-xs text-fg-2">
                {project.threads} {project.threads === 1 ? 'thread' : 'threads'}
                {project.open_mrs > 0 && `, ${project.open_mrs} open merge ${project.open_mrs === 1 ? 'request' : 'requests'}`}
              </span>
            </div>
            {project.clone ? (
              <p className="font-mono text-2xs text-fg-2">Here at {project.clone}</p>
            ) : (
              !hosted() && <CloneHere project={project} projects={all} />
            )}
            {!hosted() && project.clone && <ProjectHome project={project} />}
            {(project.default_agent || project.default_preset) && (
              <p className="text-xs text-fg-2">
                New threads start with{' '}
                {[
                  project.default_agent && (agents.data?.agents.find((agent) => agent.id === project.default_agent)?.name ?? project.default_agent),
                  project.default_preset && (PRESETS.find((preset) => preset.id === project.default_preset)?.label.toLowerCase() ?? project.default_preset),
                ]
                  .filter(Boolean)
                  .join(', ')}
              </p>
            )}
          </li>
        ))}
      </ul>
    </Page>
  )
}
