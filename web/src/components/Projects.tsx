import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, type Project } from '../api'
import { keys, message } from '../query'
import { Button, EmptyState, Input, Skeleton, Tabs, Tag, toast } from '../ui'

/** Where a clone of `project` might go: next to this machine's other clones. */
function suggestedPath(project: Project, projects: Project[]): string {
  const other = projects.find((candidate) => candidate.clone)?.clone
  if (!other) return ''
  const parent = other.replace(/\/+$/, '').replace(/\/[^/]*$/, '')
  return `${parent}/${project.name}`
}

function CloneHere({ project, projects }: { project: Project; projects: Project[] }) {
  const queries = useQueryClient()
  const [path, setPath] = useState(() => suggestedPath(project, projects))
  const [cloning, setCloning] = useState(false)
  const [error, setError] = useState<string>()
  if (!project.origin_url) return <span className="text-xs text-fg-3">No one has said where to clone it from yet</span>
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
      <Input aria-label={`where to clone ${project.name}`} className="w-72 font-mono text-xs" placeholder="/home/me/src/app" value={path} onChange={(event) => setPath(event.target.value)} />
      <Button type="submit" disabled={!path.trim() || cloning}>
        {cloning ? 'Cloning…' : 'Clone here'}
      </Button>
      {error && <span className="w-full text-xs text-danger">{error}</span>}
    </form>
  )
}

/** Adds a clone already on this machine, or clones a remote URL. */
function AddProject() {
  const queries = useQueryClient()
  const [how, setHow] = useState<'existing' | 'url'>('existing')
  const [path, setPath] = useState('')
  const [url, setUrl] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const done = (repo: string) => {
    toast(`Added ${repo}`, 'ok')
    setPath('')
    setUrl('')
    return queries.invalidateQueries({ queryKey: keys.projects })
  }
  return (
    <section aria-label="add a project" className="rounded-md border border-line p-3">
      <div className="mb-2 flex items-center justify-between">
        <h3 className="text-sm font-medium">Add a project</h3>
        <Tabs
          label="how to add it"
          value={how}
          onChange={setHow}
          tabs={[
            { id: 'existing', label: 'A clone on this machine' },
            { id: 'url', label: 'Clone a URL' },
          ]}
        />
      </div>
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault()
          setError(undefined)
          setBusy(true)
          const adding = how === 'existing' ? api.addClone(path) : api.cloneProject(url, path)
          adding.then(done, (failure: unknown) => setError(message(failure))).finally(() => setBusy(false))
        }}
      >
        {how === 'url' && (
          <Input aria-label="remote URL" className="w-80 font-mono text-xs" placeholder="git@github.com:acme/app.git" value={url} onChange={(event) => setUrl(event.target.value)} />
        )}
        <Input
          aria-label={how === 'existing' ? 'clone path' : 'where to clone it'}
          className="w-72 font-mono text-xs"
          placeholder="/home/me/src/app"
          value={path}
          onChange={(event) => setPath(event.target.value)}
        />
        <Button type="submit" variant="primary" disabled={busy || !path.trim() || (how === 'url' && !url.trim())}>
          {busy ? 'Adding…' : 'Add'}
        </Button>
        {error && <p className="w-full text-xs text-danger">{error}</p>}
      </form>
    </section>
  )
}

/** The projects the person works in, with this machine's clone of each (ADR 0023). */
export function Projects() {
  const projects = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const all = projects.data ?? []
  return (
    <div className="mx-auto max-w-3xl space-y-4">
      <h2 className="text-lg font-semibold">Projects</h2>
      {projects.isPending && <Skeleton className="h-16 w-full" />}
      {projects.error && <p className="text-sm text-danger">{message(projects.error)}</p>}
      {projects.isSuccess && all.length === 0 && (
        <EmptyState title="No projects yet">Add a clone below, or run `silt open` in one.</EmptyState>
      )}
      <ul data-testid="projects" className="divide-y divide-line-subtle rounded-md border border-line">
        {all.map((project) => (
          <li key={project.repo} aria-label={project.name} className="space-y-1 px-3 py-2 text-sm">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-medium">{project.name}</span>
              <span className="font-mono text-xs text-fg-3">{project.repo}</span>
              {project.org && <Tag>{project.org}</Tag>}
              <span className="ml-auto text-xs text-fg-2">
                {project.threads} {project.threads === 1 ? 'thread' : 'threads'}
                {project.open_mrs > 0 && `, ${project.open_mrs} open merge ${project.open_mrs === 1 ? 'request' : 'requests'}`}
              </span>
            </div>
            {project.clone ? (
              <p className="font-mono text-xs text-fg-2">Here at {project.clone}</p>
            ) : (
              <CloneHere project={project} projects={all} />
            )}
            {(project.default_agent || project.default_preset) && (
              <p className="text-xs text-fg-3">
                New threads start with {[project.default_agent, project.default_preset?.replaceAll('_', ' ')].filter(Boolean).join(', ')}
              </p>
            )}
          </li>
        ))}
      </ul>
      <AddProject />
    </div>
  )
}
