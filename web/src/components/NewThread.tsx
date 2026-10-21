import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { rememberedModes } from '../agentModes'
import { PRESETS } from '../agentState'
import { api, type Preset } from '../api'
import { keys, message } from '../query'
import { Button, EmptyState, Select, Textarea, toast } from '../ui'
import { AgentPicker } from './Agent'

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs text-fg-2">
      {label}
      {children}
    </label>
  )
}

/** Starts an agent in a new thread: the prompt first, then where and how (ADR 0022). */
export function NewThread() {
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const navigate = useNavigate()
  const [clone, setClone] = useState('')
  const [here, setHere] = useState(false)
  const [base, setBase] = useState('')
  const [agent, setAgent] = useState('')
  const [mode, setMode] = useState('')
  const [preset, setPreset] = useState<Preset>('ask_before_commands')
  const [visibility, setVisibility] = useState('')
  const [prompt, setPrompt] = useState('')
  const [plan, setPlan] = useState(false)
  const [background, setBackground] = useState(false)
  const [error, setError] = useState<string>()
  const clones = choices.data?.clones ?? []
  const project = clone || clones[0]?.path || ''
  const branches = useQuery({
    queryKey: keys.branches(project),
    queryFn: () => api.branches(project),
    enabled: project !== '' && !here,
  })
  const agentName = choices.data?.agents.find((candidate) => candidate.id === agent)?.name ?? ''
  const modes = agentName ? rememberedModes(agentName) : []

  if (choices.isSuccess && clones.length === 0) {
    return (
      <EmptyState title="No projects on this machine yet">
        Run <code>silt open</code> in a clone to start threads in it from here.
      </EmptyState>
    )
  }

  const start = () => {
    setError(undefined)
    const options = {
      here,
      base: here || !base ? null : base,
      mode: mode || null,
      preset,
      visibility: visibility || null,
      prompt: prompt.trim() ? prompt : null,
      plan: plan && prompt.trim() !== '',
    }
    api.startAgent(agent, { clone: project }, options).then(
      (thread) => {
        if (!background) {
          void navigate({ to: '/t/$id', params: { id: thread } })
          return
        }
        toast(`${agentName} started in a new thread`, 'ok')
        setPrompt('')
        setPlan(false)
      },
      (failure: unknown) => setError(message(failure)),
    )
  }

  return (
    <form
      aria-label="new thread"
      className="mx-auto max-w-2xl space-y-4"
      onSubmit={(event) => {
        event.preventDefault()
        start()
      }}
    >
      <h2 className="text-lg font-semibold">New thread</h2>
      <Textarea
        aria-label="first prompt"
        rows={5}
        placeholder="What should the agent do? (optional: you can also start it and prompt it later)"
        value={prompt}
        onChange={(event) => setPrompt(event.target.value)}
      />
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Field label="Project">
          <Select aria-label="project" value={project} onChange={(event) => setClone(event.target.value)}>
            {clones.map((known) => (
              <option key={known.path} value={known.path}>
                {known.repo || known.path}
              </option>
            ))}
          </Select>
        </Field>
        <Field label="Works in">
          <Select aria-label="works in" value={here ? 'here' : 'own'} onChange={(event) => setHere(event.target.value === 'here')}>
            <option value="own">Its own checkout</option>
            <option value="here">My clone, as it is</option>
          </Select>
        </Field>
        {!here && (
          <Field label="Starts from">
            <Select aria-label="base branch" value={base} onChange={(event) => setBase(event.target.value)}>
              <option value="">What my clone has checked out</option>
              {(branches.data ?? []).map((branch) => (
                <option key={branch} value={branch}>
                  {branch}
                </option>
              ))}
            </Select>
          </Field>
        )}
        <Field label="Agent">
          <AgentPicker
            value={agent}
            onChange={(chosen) => {
              setAgent(chosen)
              setMode('')
            }}
          />
        </Field>
        {modes.length > 0 && (
          <Field label="Mode">
            <Select aria-label="mode" value={mode} onChange={(event) => setMode(event.target.value)}>
              <option value="">The agent&apos;s default</option>
              {modes.map((choice) => (
                <option key={choice.id} value={choice.id}>
                  {choice.name}
                </option>
              ))}
            </Select>
          </Field>
        )}
        <Field label="Permissions">
          <Select aria-label="permissions" value={preset} onChange={(event) => setPreset(event.target.value as Preset)}>
            {PRESETS.map((choice) => (
              <option key={choice.id} value={choice.id}>
                {choice.label}
              </option>
            ))}
          </Select>
        </Field>
        {status.data?.server && (
          <Field label="Who sees it">
            <Select aria-label="visibility" value={visibility} onChange={(event) => setVisibility(event.target.value)}>
              <option value="">As the project says</option>
              <option value="private">Only me and those I share it with</option>
              <option value="org">My org</option>
            </Select>
          </Field>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-4 text-xs text-fg-2">
        <label className="flex items-center gap-1" title="The agent writes a plan for you to approve before it edits anything">
          <input type="checkbox" checked={plan} disabled={!prompt.trim()} onChange={(event) => setPlan(event.target.checked)} />
          Plan first
        </label>
        <label className="flex items-center gap-1" title="Stay here and start another">
          <input type="checkbox" checked={background} onChange={(event) => setBackground(event.target.checked)} />
          Start in the background
        </label>
      </div>
      {preset === 'full_access' && <p className="text-xs text-warn">Nothing asks; the agent&apos;s own sandbox is all that limits it.</p>}
      {error && <p className="text-sm text-danger">{error}</p>}
      <div className="flex gap-2">
        <Button type="submit" variant="primary" icon="play" disabled={!agent || !project}>
          Start
        </Button>
        <Link to="/" className="px-2.5 py-1 text-xs text-fg-2 hover:text-fg-1">
          Cancel
        </Link>
      </div>
    </form>
  )
}
