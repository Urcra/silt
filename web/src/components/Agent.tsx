import { useQuery } from '@tanstack/react-query'
import { type ReactNode, useEffect, useState } from 'react'
import { rememberModes } from '../agentModes'
import { api, type AgentStatus, type PermissionAsked, type Presence, type Preset, type PromptAttachment } from '../api'
import { readAttachment } from '../attachments'
import { APPROVED, PRESETS, STATE_PHRASE, running } from '../agentState'
import { useNow } from '../now'
import { keys, message } from '../query'
import { hosted } from '../served'
import { formatKeys, useActions, useShortcut } from '../shortcuts'
import { Button, Dot, type DotTone, Icon, type IconName, Kbd, cx } from '../ui'
import { Markdown } from './Markdown'
import { PromptQueue } from './Queue'

export function AgentPicker({ value, onChange, className }: { value: string; onChange: (agent: string) => void; className?: string }) {
  const choices = useQuery({ queryKey: keys.agents, queryFn: api.agents })
  return (
    <select
      aria-label="agent to start"
      value={value}
      onChange={(event) => onChange(event.target.value)}
      className={className ?? 'rounded-sm border border-line bg-elevated px-2 py-1 text-sm text-fg-1'}
    >
      <option value="">pick an agent…</option>
      {(choices.data?.agents ?? []).map((agent) => (
        <option key={agent.id} value={agent.id} title={agent.description}>
          {agent.name}
        </option>
      ))}
    </select>
  )
}

const CHIP =
  'inline-flex items-center gap-1.5 rounded-sm border border-line bg-elevated px-2 py-0.5 text-xs text-fg-1 hover:bg-surface-2 max-md:min-h-11'

/** A choice in the composer's row, drawn as a chip: `prefix` then the chosen option. */
function ChipSelect({
  label,
  prefix,
  icon,
  value,
  onChange,
  options,
}: {
  label: string
  prefix?: string
  icon?: IconName
  value: string
  onChange: (value: string) => void
  options: { id: string; name: string; description?: string | null }[]
}) {
  return (
    <label className={cx(CHIP, 'relative cursor-pointer pr-1')}>
      {icon && <Icon name={icon} className="h-3 w-3" />}
      {prefix && <span className="text-fg-2">{prefix}</span>}
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className="cursor-pointer appearance-none bg-transparent pr-4 text-xs text-fg-1 focus:outline-none"
      >
        {options.map((option) => (
          <option key={option.id} value={option.id} title={option.description ?? undefined}>
            {option.name}
          </option>
        ))}
      </select>
      <Icon name="chevronDown" className="pointer-events-none absolute right-1 h-3 w-3 text-fg-2" />
    </label>
  )
}

/** Whether a prompt goes straight to the agent rather than its queue. */
function ready(status: AgentStatus): boolean {
  return status.state === 'idle' || status.state === 'awaiting_input'
}

/** Whether a turn is under way. */
function working(status: AgentStatus | null): boolean {
  return status?.state === 'working' || status?.state === 'awaiting_permission'
}

/** A permission the agent asks for, answered in place: ⏎ allows, Esc dismisses (ADR 0022). */
export function PermissionCard({ thread, asked, agent, first }: { thread: string; asked: PermissionAsked; agent: string; first: boolean }) {
  const [error, setError] = useState<string>()
  const answer = (option: string | null) => {
    setError(undefined)
    api.answer(thread, asked.id, option).catch((failure: unknown) => setError(message(failure)))
  }
  const [allow] = asked.options
  useShortcut('enter', 'Allow what the agent asks', () => {
    if (first && allow && !document.querySelector('dialog[open]')) answer(allow[0])
  })
  useShortcut('escape', 'Dismiss what the agent asks', () => {
    if (first && !document.querySelector('dialog[open]')) answer(null)
  })
  return (
    <div role="group" aria-label="permission" className="flex flex-col gap-2 rounded-md border border-warn bg-warn-tint px-3.5 py-3">
      <span className="text-sm font-medium">
        {agent} wants to: <code className="font-mono text-xs">{asked.title}</code>
      </span>
      <span className="flex flex-wrap gap-1.5">
        {asked.options.map(([option, label], index) => (
          <Button key={option} variant={index === 0 ? 'primary' : 'secondary'} onClick={() => answer(option)}>
            {label}
            {index === 0 && first && <Kbd onAccent>⏎</Kbd>}
          </Button>
        ))}
        <Button variant="ghost" onClick={() => answer(null)}>
          Dismiss {first && <Kbd>Esc</Kbd>}
        </Button>
      </span>
      {error && <span className="text-xs text-danger">{error}</span>}
    </div>
  )
}

/** The agent's plan, waiting for a yes or a change. */
export function PlanCard({ thread }: { thread: string }) {
  const [error, setError] = useState<string>()
  return (
    <div role="group" aria-label="plan" className="flex flex-wrap items-center gap-2 rounded-md border border-accent bg-accent-tint px-3.5 py-3">
      <span className="text-sm">Approve the plan, or send a prompt to change it.</span>
      <Button
        variant="primary"
        className="ml-auto"
        onClick={() => api.prompt(thread, APPROVED).catch((failure: unknown) => setError(message(failure)))}
      >
        Approve plan
      </Button>
      {error && <span className="w-full text-xs text-danger">{error}</span>}
    </div>
  )
}

/** What the agent is doing now, before it is recorded: its words, its thinking, the tool's output. */
export function AgentTail({ status, toolOutput }: { status: AgentStatus; toolOutput: string }) {
  if (!working(status) && !status.streaming) return null
  return (
    <div className="flex flex-col gap-2" aria-live="polite">
      {status.thinking && <p className="text-xs text-fg-2 italic">{status.thinking}</p>}
      {status.streaming && <Markdown text={status.streaming} />}
      {toolOutput && (
        <pre data-testid="tool-output" className="max-h-48 overflow-auto rounded-sm border border-line-subtle bg-surface-1 p-2 font-mono text-xs">
          {toolOutput}
        </pre>
      )}
    </div>
  )
}

/** Starts an agent in the thread on one of the person's online devices, which takes it over (ADR 0026). */
function ContinueOn({ thread, act }: { thread: string; act: (action: Promise<unknown>) => void }) {
  const devices = useQuery({ queryKey: keys.devices, queryFn: api.devices })
  const [device, setDevice] = useState('')
  const [agent, setAgent] = useState('')
  const online = (devices.data ?? []).filter((candidate) => candidate.online && candidate.startable.length > 0)
  const chosen = online.find((candidate) => candidate.id === device) ?? online[0]
  if (!chosen) return <p className="text-fg-2">No agent runs in this thread, and none of your devices is online to continue it on.</p>
  const chosenAgent = chosen.startable.find((candidate) => candidate.id === agent) ?? chosen.startable[0]
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span>Continue on</span>
      <ChipSelect
        label="device to continue on"
        icon="laptop"
        value={chosen.id}
        onChange={setDevice}
        options={online.map((candidate) => ({ id: candidate.id, name: candidate.name }))}
      />
      <span>with</span>
      <ChipSelect
        label="agent to continue with"
        value={chosenAgent.id}
        onChange={setAgent}
        options={chosen.startable.map((candidate) => ({ id: candidate.id, name: candidate.name }))}
      />
      <Button variant="primary" onClick={() => act(api.continueOn(thread, chosen.id, chosenAgent.id))}>
        Continue there
      </Button>
    </div>
  )
}

/** How long the turn under way has run, as `0:42`. */
function useTurnClock(status: AgentStatus | null): string | null {
  const now = useNow(1000)
  const [started, setStarted] = useState<number | null>(null)
  const turning = working(status)
  if (turning && started === null) setStarted(now)
  if (!turning && started !== null) setStarted(null)
  if (started === null) return null
  const seconds = Math.max(0, Math.floor((now - started) / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}

const STATE_TONE: Record<AgentStatus['state'], DotTone> = {
  starting: 'idle',
  idle: 'idle',
  working: 'ok',
  awaiting_permission: 'warn',
  awaiting_input: 'warn',
  error: 'danger',
  stopped: 'idle',
}

/**
 * The agent working in the thread, docked under its conversation (ADR 0014,
 * the Workspace board): how it stands, what's queued for it, and the
 * composer, whose draft everyone here sees and sends together.
 */
export function AgentDock({
  thread,
  status,
  preset,
  onPreset,
  draft,
  onDraft,
  people,
  turn,
}: {
  thread: string
  status: AgentStatus | null
  preset: Preset
  onPreset: (preset: Preset) => void
  draft: string
  onDraft: (draft: string) => void
  /** Everyone else here, whose machines may run an agent in the thread. */
  people: Presence[]
  /** The turn under way, when there is one. */
  turn: number | null
}) {
  const [agent, setAgent] = useState('')
  const [planFirst, setPlanFirst] = useState(false)
  const [attached, setAttached] = useState<PromptAttachment[]>([])
  const [error, setError] = useState<string>()
  const act = (action: Promise<unknown>, after?: () => void) => {
    setError(undefined)
    action.then(after, (failure: unknown) => setError(message(failure)))
  }
  const live = status !== null && running(status.state)
  const elsewhere = people.find((person) => person.agent)
  const modes = status?.modes?.available
  const agentName = status?.agent
  const clock = useTurnClock(status)
  const narrow = typeof window !== 'undefined' && window.matchMedia?.('(max-width: 767px)').matches
  const signedIn = useQuery({ queryKey: keys.status, queryFn: api.status, enabled: !hosted() })
  const device = signedIn.data?.servers?.[0]?.name
  useEffect(() => {
    if (agentName && modes) rememberModes(agentName, modes)
  }, [agentName, modes])
  const turning = working(status)
  useActions([
    ...(live ? [{ id: 'stop', label: `Stop ${status.agent}`, run: () => act(api.stopAgent(thread)) }] : []),
    ...(turning ? [{ id: 'cancel', label: 'Cancel the turn', run: () => act(api.cancelTurn(thread)) }] : []),
    ...(status?.state === 'awaiting_input'
      ? [{ id: 'approve', label: 'Approve the plan', run: () => act(api.prompt(thread, APPROVED)) }]
      : []),
  ])
  const sendable = live && draft.trim() !== '' && !(attached.length > 0 && !ready(status))
  const send = () => {
    if (!live || !sendable) return
    act(api.prompt(thread, draft, planFirst && ready(status), attached), () => {
      setPlanFirst(false)
      setAttached([])
      onDraft('')
    })
  }

  let statusLine: ReactNode = null
  if (live) {
    statusLine = (
      <>
        <Dot tone={STATE_TONE[status.state]} />
        <span data-testid="agent-state" className="text-xs font-medium">
          {status.agent} {STATE_PHRASE[status.state]}
        </span>
        <span className="font-mono text-2xs text-fg-2">
          {[turn !== null && turning && `turn ${turn}`, !hosted() && device && `running on ${device}`].filter(Boolean).join(' · ')}
        </span>
      </>
    )
  } else if (elsewhere?.agent) {
    statusLine = (
      <>
        <Dot tone={STATE_TONE[elsewhere.agent.state]} />
        <span data-testid="agent-state" className="text-xs font-medium">
          {elsewhere.user}&apos;s {elsewhere.agent.name} {STATE_PHRASE[elsewhere.agent.state]}
        </span>
      </>
    )
  }

  return (
    <section aria-label="agent at work" className="flex flex-col gap-2">
      {!hosted() && <PromptQueue thread={thread} />}
      <div className="flex flex-col rounded-md border border-line-strong bg-elevated">
        {statusLine && (
          <div className="flex flex-wrap items-center gap-2 border-b border-line-subtle px-3 py-1.5">
            {statusLine}
            {live && (
              <span className="ml-auto flex gap-1">
                {turning && (
                  <Button variant="ghost" size="sm" onClick={() => act(api.cancelTurn(thread))}>
                    Cancel turn
                  </Button>
                )}
                <Button variant="ghost" size="sm" icon="stop" onClick={() => act(api.stopAgent(thread))}>
                  Stop agent
                </Button>
              </span>
            )}
          </div>
        )}
        {elsewhere?.agent && !live && (
          <div className="border-b border-line-subtle px-3 py-2 text-xs text-fg-2">
            {elsewhere.agent.streaming && <Markdown text={elsewhere.agent.streaming} className="mb-1 text-fg-1" />}
            Your draft goes along with {elsewhere.user}&apos;s next prompt.
          </div>
        )}
        {people.some((person) => person.draft.trim()) && (
          <ul data-testid="drafts" aria-label="what others here are writing" className="flex flex-col gap-1 border-b border-line-subtle px-3 py-2 text-xs">
            {people
              .filter((person) => person.draft.trim())
              .map((person) => (
                <li key={person.device}>
                  <span className="font-mono text-2xs text-fg-2">{person.user} is drafting:</span> {person.draft}
                </li>
              ))}
          </ul>
        )}
        <label htmlFor={`draft-${thread}`} className="sr-only">
          draft
        </label>
        <textarea
          id={`draft-${thread}`}
          aria-label="draft"
          rows={narrow ? 1 : 3}
          value={draft}
          onChange={(event) => onDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
              event.preventDefault()
              send()
            }
          }}
          placeholder={live ? `Ask ${status.agent}, or attach files for context` : 'Draft the next prompt; everyone here sees it'}
          className="resize-none border-0 bg-transparent px-3 py-2.5 text-sm leading-normal text-fg-1 placeholder:text-fg-2 focus:outline-none"
        />
        {attached.length > 0 && (
          <ul aria-label="attached files" className="flex flex-wrap gap-1.5 px-3 pb-2 text-xs">
            {attached.map((file, index) => (
              <li key={`${file.name}-${index}`} className="flex items-center gap-1 rounded-sm bg-surface-2 px-1.5 py-0.5">
                <Icon name="file" className="h-3 w-3" />
                {file.name}
                <button
                  type="button"
                  aria-label={`remove ${file.name}`}
                  className="text-fg-2 hover:text-fg-1"
                  onClick={() => setAttached(attached.filter((_, kept) => kept !== index))}
                >
                  <Icon name="x" className="h-3 w-3" />
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className="flex items-center gap-1.5 border-t border-line-subtle px-2 py-1.5 max-md:overflow-x-auto md:flex-wrap [&>*]:shrink-0">
          {live && <span className={cx(CHIP, 'font-mono')}>{status.agent}</span>}
          {!live && !elsewhere && !hosted() && (
            <>
              <span className="text-xs text-fg-2">Run an agent here:</span>
              <AgentPicker value={agent} onChange={setAgent} className={cx(CHIP, 'pr-2')} />
              <Button variant="primary" disabled={!agent} onClick={() => act(api.startAgent(agent, { thread }))}>
                Start agent
              </Button>
            </>
          )}
          {!live && !elsewhere && hosted() && <ContinueOn thread={thread} act={act} />}
          {live && status.modes && (
            <ChipSelect
              label="mode"
              prefix="Mode:"
              value={status.modes.current}
              onChange={(mode) => act(api.setMode(thread, mode))}
              options={status.modes.available.map((mode) => ({ id: mode.id, name: mode.name, description: mode.description }))}
            />
          )}
          {live &&
            status.settings.map((setting) => (
              <ChipSelect
                key={setting.id}
                label={setting.name.toLowerCase()}
                prefix={`${setting.name}:`}
                value={setting.current}
                onChange={(value) => act(api.setSetting(thread, setting.id, value))}
                options={setting.choices}
              />
            ))}
          {(live || !hosted()) && (
            <ChipSelect
              label="permissions"
              icon="lock"
              value={preset}
              onChange={(chosen) => {
                const picked = chosen as Preset
                void api.setPreset(thread, picked).then(() => onPreset(picked))
              }}
              options={PRESETS.map((choice) => ({ id: choice.id, name: choice.label }))}
            />
          )}
          {live && ready(status) && (
            <label className={cx(CHIP, 'cursor-pointer')} title="The agent writes a plan for you to approve before it edits anything">
              <input type="checkbox" checked={planFirst} onChange={(event) => setPlanFirst(event.target.checked)} className="accent-[var(--accent)]" />
              Plan first
            </label>
          )}
          {live && !hosted() && (
            <label
              className={cx(CHIP, 'cursor-pointer')}
              title={status.takes_images ? 'Attach images or files' : `Attach text files; ${status.agent} takes no images`}
            >
              <Icon name="plus" className="h-3 w-3" />
              Context
              <input
                type="file"
                multiple
                aria-label="attach files"
                className="sr-only"
                accept={status.takes_images || status.takes_files ? undefined : 'text/*,.md,.json,.ts,.tsx,.rs,.py,.toml,.yaml,.yml'}
                onChange={(event) => {
                  const files = [...(event.target.files ?? [])]
                  event.target.value = ''
                  act(Promise.all(files.map(readAttachment)).then((read) => setAttached((before) => [...before, ...read])))
                }}
              />
            </label>
          )}
          {live && (
            <span className="ml-auto flex items-center gap-1.5">
              {clock && <span className="font-mono text-2xs text-fg-2">turn running · {clock}</span>}
              {turning && (
                <Button
                  disabled={!draft.trim()}
                  onClick={() => act(api.steer(thread, draft), () => onDraft(''))}
                  title="Interrupts the agent's turn to tell it your draft"
                >
                  Steer now
                </Button>
              )}
              <Button
                variant="primary"
                disabled={!sendable}
                onClick={send}
                title={
                  ready(status)
                    ? "Sends your draft together with everyone else's"
                    : attached.length > 0
                      ? 'Attached files go with a prompt the agent takes now'
                      : 'Queues your draft for when the agent is ready'
                }
              >
                {ready(status) ? 'Send' : 'Queue'}
                <Kbd onAccent>{formatKeys('mod+⏎')}</Kbd>
              </Button>
            </span>
          )}
        </div>
      </div>
      {preset === 'full_access' && (
        <p className="text-xs text-fg-2">
          <Icon name="alert" className="mr-1 inline h-3 w-3 text-warn" />
          Full access: nothing asks; the agent&apos;s own sandbox is all that limits it.
        </p>
      )}
      {(status?.error || error) && <p className="text-sm text-danger">{status?.error ?? error}</p>}
    </section>
  )
}
