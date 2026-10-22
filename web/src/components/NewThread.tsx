import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate, useSearch } from '@tanstack/react-router'
import { useState } from 'react'
import { rememberedModes } from '../agentModes'
import { PRESETS } from '../agentState'
import { api, type Preset } from '../api'
import { useNow } from '../now'
import { projectName } from '../projects'
import { keys, message } from '../query'
import { hosted } from '../served'
import { formatKeys } from '../shortcuts'
import { threadTone } from '../threadLook'
import { ago } from '../time'
import { Button, Dot, EmptyState, Icon, Kbd, cx, toast } from '../ui'
import { CHIP, ChipSelect } from './ChipSelect'
import { Mock } from './Mock'
import { Page } from './Page'

/** What each permission preset lets the agent do without asking (ADR 0022). */
const PRESET_SAYS: Record<Preset, string> = {
  ask_before_commands: 'edits apply, every command asks',
  ask_for_everything: 'edits and commands ask',
  full_access: 'nothing asks; for machines you trust',
}

interface Stashed {
  text: string
  project: string
  at: number
}

const STASH_KEY = 'silt-stashed-prompts'

function readStash(): Stashed[] {
  try {
    return JSON.parse(localStorage.getItem(STASH_KEY) ?? '[]') as Stashed[]
  } catch {
    return []
  }
}

function writeStash(stash: Stashed[]) {
  try {
    localStorage.setItem(STASH_KEY, JSON.stringify(stash))
  } catch {
    // Kept for this page only, then.
  }
}

/** Starts an agent in a new thread, composer first (ADR 0022, the New thread board). */
export function NewThread() {
  const now = useNow()
  const asked = useSearch({ strict: false }) as { prompt?: string }
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  const status = useQuery({ queryKey: keys.status, queryFn: api.status })
  const threads = useQuery({ queryKey: keys.threads, queryFn: api.threads })
  const navigate = useNavigate()
  const [clone, setClone] = useState('')
  const [here, setHere] = useState(false)
  const [base, setBase] = useState('')
  const [chosenAgent, setAgent] = useState<string>()
  const [mode, setMode] = useState('')
  const [chosenPreset, setPreset] = useState<Preset>()
  const [visibility, setVisibility] = useState('')
  const [prompt, setPrompt] = useState(asked.prompt ?? '')
  const [plan, setPlan] = useState(false)
  const [stash, setStash] = useState(readStash)
  const [error, setError] = useState<string>()
  const clones = choices.data?.clones ?? []
  const project = clone || clones[0]?.path || ''
  // What the project's new threads start with, until the person picks (ADR 0023).
  const projects = useQuery({ queryKey: keys.projects, queryFn: api.projects })
  const defaults = projects.data?.find((candidate) => candidate.clone === project)
  const suggestedAgent = choices.data?.agents.find((candidate) => candidate.id === defaults?.default_agent)?.id ?? ''
  const agent = chosenAgent ?? suggestedAgent
  const preset = chosenPreset ?? PRESETS.find((choice) => choice.id === defaults?.default_preset)?.id ?? 'ask_before_commands'
  const branches = useQuery({
    queryKey: keys.branches(project),
    queryFn: () => api.branches(project),
    enabled: project !== '' && !here,
  })
  const agentName = choices.data?.agents.find((candidate) => candidate.id === agent)?.name ?? ''
  const modes = agentName ? rememberedModes(agentName) : []
  const repo = clones.find((known) => known.path === project)?.repo ?? ''
  const device = status.data?.servers?.[0]?.name ?? 'this machine'

  if (hosted()) {
    return (
      <Page width="narrow">
        <EmptyState title="New threads start on your devices">
          Open Silt from the machine that should run the agent, with <code>silt open</code>.
        </EmptyState>
      </Page>
    )
  }
  if (choices.isSuccess && clones.length === 0) {
    return (
      <Page width="narrow">
        <EmptyState title="Add a project to start threads in it">
          Add one from{' '}
          <Link to="/projects/add" className="text-accent-fg">
            Add a project
          </Link>
          , or run <code>silt open</code> in a clone.
        </EmptyState>
      </Page>
    )
  }

  const start = (background: boolean) => {
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
  const saveStash = (next: Stashed[]) => {
    setStash(next)
    writeStash(next)
  }
  const recent = (threads.data ?? [])
    .filter((thread) => !thread.mine.settled && !thread.merged)
    .sort((a, b) => b.last_ms - a.last_ms)
    .slice(0, 4)
  const ready = Boolean(agent && project)

  return (
    <Page flush label="new thread">
      <div className="mx-auto flex w-full max-w-[880px] flex-col gap-6 px-4 py-8 md:px-8">
        <h1 className="pt-6 text-center text-2xl font-medium tracking-[-0.01em] md:pt-12">What should the agent work on?</h1>
        <form
          aria-label="new thread"
          className="flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault()
            start(false)
          }}
        >
          <div className="flex flex-wrap items-center gap-1.5">
            <ChipSelect
              label="project"
              prefix="Project"
              mono
              value={project}
              onChange={setClone}
              options={clones.map((known) => ({ id: known.path, name: known.repo ? projectName(known.repo) : known.path, description: known.path }))}
            />
            {!here && (
              <ChipSelect
                label="base branch"
                prefix="From"
                mono={base !== ''}
                value={base}
                onChange={setBase}
                options={[{ id: '', name: 'current checkout' }, ...(branches.data ?? []).map((branch) => ({ id: branch, name: branch }))]}
              />
            )}
            <div role="radiogroup" aria-label="works in" className="inline-flex overflow-hidden rounded-sm border border-line">
              {[
                ['own', 'Own checkout', false],
                ['here', 'In my clone', true],
              ].map(([id, label, value]) => (
                <button
                  key={id as string}
                  type="button"
                  role="radio"
                  aria-checked={here === value}
                  onClick={() => setHere(value as boolean)}
                  className={cx('px-2.5 py-0.5 text-xs max-md:min-h-11', here === value ? 'bg-accent-tint text-accent-fg' : 'bg-elevated text-fg-2 hover:bg-surface-2')}
                >
                  {label as string}
                </button>
              ))}
            </div>
            <span className={cx(CHIP, 'md:ml-auto')}>
              <Icon name="laptop" className="h-3 w-3" />
              <span className="text-fg-2">On</span>
              <Mock id="new.device">{device}</Mock>
            </span>
          </div>
          <div className="flex flex-col rounded-md border border-line-strong bg-elevated">
            <label htmlFor="first-prompt" className="sr-only">
              First prompt
            </label>
            <textarea
              id="first-prompt"
              aria-label="first prompt"
              rows={6}
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && ready) {
                  event.preventDefault()
                  start(event.shiftKey)
                }
              }}
              placeholder="Describe the task. You can also start the agent and prompt it later."
              className="resize-none border-0 bg-transparent px-3.5 py-3 text-sm leading-normal text-fg-1 placeholder:text-fg-2 focus:outline-none"
            />
            <div className="flex flex-wrap items-center gap-1.5 border-t border-line-subtle px-2.5 py-2">
              <ChipSelect
                label="agent to start"
                value={agent}
                onChange={(chosen) => {
                  setAgent(chosen)
                  setMode('')
                }}
                options={[{ id: '', name: 'pick an agent…' }, ...(choices.data?.agents ?? []).map((choice) => ({ id: choice.id, name: choice.name, description: choice.description }))]}
              />
              {modes.length > 0 && (
                <ChipSelect
                  label="mode"
                  prefix="Mode"
                  value={mode}
                  onChange={setMode}
                  options={[{ id: '', name: 'its default' }, ...modes.map((choice) => ({ id: choice.id, name: choice.name }))]}
                />
              )}
              <label
                className={cx(CHIP, 'cursor-pointer', !prompt.trim() && 'cursor-not-allowed text-fg-2', plan && 'border-accent bg-accent-tint text-accent-fg')}
                title={prompt.trim() ? 'The agent writes a plan for you to approve before it edits anything' : 'Write a first prompt for the agent to plan'}
              >
                <input
                  type="checkbox"
                  checked={plan}
                  disabled={!prompt.trim()}
                  onChange={(event) => setPlan(event.target.checked)}
                  className="accent-[var(--accent)]"
                />
                Plan first
              </label>
              <ChipSelect
                label="permissions"
                icon="lock"
                value={preset}
                onChange={(chosen) => setPreset(chosen as Preset)}
                options={PRESETS.map((choice) => ({ id: choice.id, name: choice.label }))}
              />
              {status.data?.server && (
                <ChipSelect
                  label="visibility"
                  prefix="Visible"
                  value={visibility}
                  onChange={setVisibility}
                  options={[
                    { id: '', name: 'project default' },
                    { id: 'private', name: 'only me' },
                    { id: 'org', name: 'my org' },
                  ]}
                />
              )}
              <span className="ml-auto flex flex-wrap items-center gap-1.5">
                <Button disabled={!prompt.trim()} variant="ghost" icon="clock" onClick={() => {
                  saveStash([{ text: prompt.trim(), project: repo, at: Date.now() }, ...stash].slice(0, 10))
                  setPrompt('')
                }}>
                  Stash
                </Button>
                <Button disabled={!ready} onClick={() => start(true)} title="Starts it and stays here, to start another">
                  Start in background
                  <Kbd>{formatKeys('mod+shift+⏎')}</Kbd>
                </Button>
                <Button type="submit" variant="primary" icon="play" disabled={!ready}>
                  Start
                  <Kbd onAccent>{formatKeys('mod+⏎')}</Kbd>
                </Button>
              </span>
            </div>
          </div>
          <ul aria-label="permission presets" className="flex flex-wrap gap-x-5 gap-y-1 text-2xs text-fg-2">
            {PRESETS.map((choice) => (
              <li key={choice.id} className={cx(choice.id === preset && 'text-fg-1')}>
                <b className="font-medium">{choice.label}</b> {PRESET_SAYS[choice.id]}
              </li>
            ))}
          </ul>
          {preset === 'full_access' && <p className="text-xs text-fg-2">Nothing asks; the agent&apos;s own sandbox is all that limits it.</p>}
          {error && <p className="text-sm text-danger">{error}</p>}
        </form>
        <div className="grid gap-6 md:grid-cols-2">
          <section aria-labelledby="stashed" className="flex flex-col gap-1.5">
            <h2 id="stashed" className="font-mono text-2xs font-medium tracking-[0.06em] text-fg-2 uppercase">
              Stashed prompts
            </h2>
            {stash.length === 0 && <p className="text-xs text-fg-2">Stash a prompt to start it later; it stays in this browser.</p>}
            {stash.map((stashed, index) => (
              <button
                key={stashed.at}
                type="button"
                className="flex flex-col gap-0.5 rounded-md border border-line bg-elevated px-3 py-2 text-left text-sm hover:bg-surface-2"
                onClick={() => {
                  setPrompt(stashed.text)
                  saveStash(stash.filter((_, kept) => kept !== index))
                }}
              >
                <span className="line-clamp-2">{stashed.text}</span>
                <span className="font-mono text-2xs text-fg-2">
                  {[stashed.project && projectName(stashed.project), `stashed ${ago(stashed.at, now)}`].filter(Boolean).join(' · ')}
                </span>
              </button>
            ))}
          </section>
          <section aria-labelledby="pickup" className="flex flex-col gap-1.5">
            <h2 id="pickup" className="font-mono text-2xs font-medium tracking-[0.06em] text-fg-2 uppercase">
              Or pick up
            </h2>
            {recent.map((thread) => (
              <Link
                key={thread.id}
                to="/t/$id"
                params={{ id: thread.id }}
                className="flex flex-col gap-0.5 rounded-md border border-line bg-elevated px-3 py-2 text-sm text-fg-1 hover:bg-surface-2"
              >
                <span className="flex items-center gap-1.5">
                  <Dot tone={threadTone(thread)} />
                  <span className="truncate">{thread.title ?? 'Untitled thread'}</span>
                </span>
                <span className="font-mono text-2xs text-fg-2">
                  {[thread.location === 'moved' ? 'last on another device · resume here or fork' : thread.agent, <span key="when" data-volatile>{ago(thread.last_ms, now)}</span>]
                    .filter(Boolean)
                    .map((part, index) => (
                      <span key={index}>
                        {index > 0 && ' · '}
                        {part}
                      </span>
                    ))}
                </span>
              </Link>
            ))}
          </section>
        </div>
      </div>
    </Page>
  )
}
